//! Lowering: a checked [`Ast`] to [`Ir`].
//!
//! Lowering picks the type-specialised IR node for every operator, drops
//! `Paren` and `Ascribe` nodes (the checker has already coerced whatever an
//! ascription affects), turns integer literals typed `Float` into
//! `FloatConst` directly, and inserts `IntToFloat` for concrete `Int`
//! operands of `/`. Variables become `Store` nodes on the slots the checker
//! assigned, and each use becomes what its [`VarRef`] says: a `Load` of a
//! slot, a `LoadCapture` of a captured value, or a `LoadCallee`. Every
//! block, including the file's body, becomes a `Block` whose items are its
//! declarations and non-final expression statements, and whose result is
//! its `= EXPR` or final expression statement, or an explicit `Empty` node
//! if it has neither.
//!
//! A rebind of a mutable variable, from whatever block, is a `Store` into
//! the slot of the variable's declaration, which the checker gives the
//! item, and a dotted one a `Store` there of the update of the record or
//! list loaded from that slot (see below), so lowering does nothing of its
//! own for them. Nothing here assumes that a slot is written once: every `Load`
//! is a read of its slot where it is evaluated, never folded into another,
//! cached or moved out of the slot, and every `Store` is kept.
//!
//! A function expression's body is lowered into the IR's function table at
//! its [`crate::ast::FnId`], as code of [`FnKind::Memo`] if it is declared
//! with `->>` and of [`FnKind::Function`] otherwise (the IR does not tell
//! a pure function from an impure one, since only the checker cares), with
//! its frame and capture counts from the
//! checker's [`crate::check::FnResolution`] and its type as text, computed
//! once here, for its closures to display as. The expression itself becomes
//! a `MakeClosure` whose captures are the reads, in the frame creating the
//! closure, of each value the function captures. Function bodies nest, and
//! so does their lowering. A call lowers to a `Call` whose `Arg` nodes are
//! its arguments in source order, each with the parameter the checker bound
//! it to, and then one for each parameter the call gives its default (see
//! the checker's "Calls", [`crate::check::Resolution::call_defaults`]), in
//! parameter order, at the call's closing parenthesis, whose value is the
//! default's [`Constant`] built where the call is: an `IntConst`,
//! `FloatConst`, `BoolConst` or `StrConst`, a `MakeList` of its elements, a
//! `MakeRecord` of its fields in the order they are written, its layout
//! shared as a record block's is, or `Empty` (`Lowerer::lower_constant`).
//! So the callee is given a full list of arguments, as if every one had
//! been written, and a memoised function's key is the same either way.
//!
//! A declaration a later one shadows (see [`crate::mentions`], "Shadowed
//! declarations") lowers to nothing and is in no record: it is never run.
//! Its function expressions and `lazy`s are never made, but the function
//! table has an entry for every one, so each has code that does nothing.
//!
//! A top-level function's closure is a `Store` of its `MakeClosure` into
//! the function's slot, placed where the checker says the closure is made
//! ([`crate::check::Resolution::top_functions`]): before the outermost
//! block's first item, just after the item that binds the last of what it
//! copies, or at its own item. Where it is stored before its own item, its
//! own item lowers to nothing, or, for a `def`, only to its place among the
//! record's fields. A function's or `lazy`'s use of a top-level function of
//! its input is an [`IrKind::Top`] of the function's slot, of kind `Fn`,
//! read from the top level's frame, not a capture; top-level code's is the
//! `Load` any read is. A `def` named by a dotted path is a top-level
//! function whose slot is its leaf's cell (below): its closure, the
//! `MakeClosure` of its value as any function's (never an `UpdateField`
//! chain, as its path would otherwise make it), is stored into the cell,
//! its item is the dotted field it is in the record of the input (a leaf
//! under its root, whose cell the record loads), and its code is shown in
//! the dump by its path ([`Ir::set_paths`]).
//!
//! A record block's fields become `Store`s to their slots, as `let`s do, so
//! that later items can read them, and its result is a `MakeRecord` whose
//! values are `Load`s of those slots in definition order; every expression
//! statement in it, the last included, is a statement. Each record block's
//! field names become a [`Layout`], built here once and shared by every
//! record the block builds; blocks with the same names in the same order
//! share one layout, found through a map from the names to the layout. A
//! record block written where a record type with defaults is expected, and
//! lacking a defaulted field, is given the field by the checker (see its
//! "Field defaults", [`crate::check::Resolution::record_fills`]): its
//! `MakeRecord`'s values after those `Load`s are each such field's default,
//! in the order of the fields' names' text, built where the block closes as
//! a parameter's default is built where a call closes
//! (`Lowerer::lower_constant`), and its layout has their names after the
//! block's own. A `{}` given fields so is the `MakeRecord` of them alone,
//! not `Empty`. So at run time the value simply has the field.
//!
//! A spread item is what its soft assignments say: a `Store` of its
//! operand, awaited first if the checker marked it to be, into the item's
//! own slot, so that it is evaluated once, and then, for each field of the
//! operand's static type once awaited, in the type's order, a `Store` into
//! the field's slot (see [`crate::check::Resolution::spread_slots`]) of a
//! `Field` of a `Load` of the item's slot, which reads the field by name, of
//! the kind of the field's type, as it is. So `...r`, with `r: {a: Int, b:
//! String}`, lowers to `Store(t, Load(r))`, `Store(a, Field(Load(t), a))`
//! and `Store(b, Field(Load(t), b))`. A field that overrides a soft one
//! stores into its slot, and the record loads each field once, from its one
//! slot, in the place where it was first defined, as the kind of the
//! definition that stands. The layout is static, as for any record block.
//!
//! Only the kind of each node's value matters to the IR (see
//! [`crate::ir::Ir::kind_of`]), so lowering reads each expression's type
//! from the checker's tables and keeps only its [`ValueKind`]. A value of a
//! record subtype where a supertype is expected needs no conversion: a
//! record is the same value whatever record type it is seen as.
//!
//! A dotted `let` or `:=` lowers to a `Store` of one `Update` node, whose
//! children are, in the order they are evaluated, its value, the record or
//! list it updates and the position of each index segment of its path, and
//! whose path, one [`crate::ir::UpdateStep`] for each segment after the
//! first, is in the IR's side table (`Lowerer::lower_update`). The record or
//! list updated is the variable's slot, or capture, read as a use of it
//! would read it (see [`crate::check::PathResolution`]), of kind `List` if
//! the path's second segment indexes it and `Record` otherwise; or `Empty`
//! for a `let` of a name not in scope. Each step selects a field by name or
//! an element at the next position, and reads what it selects, creates it
//! as the empty record where the static type lacks it, or, for the last,
//! sets it. A record or list along the path that the checker marked to be
//! awaited (see [`crate::check::Resolution::path_awaits`]) is awaited where
//! it is read: the variable's by an `Await`, of kind `Record` or `List`, of
//! its read, of kind [`ValueKind::Deferred`], spanning its name, as the
//! record of a field access is, and any other by its step, which awaits
//! what it reads, as an `Await` spanning the path up to its segment would.
//! So `let a.b.c = e`, with `a: {b: {c: Int}}` in scope, lowers to
//! `Store(a', Update[.b.c](e, Load(a)))`, with `a: {x: Int}` to
//! `Store(a', Update[.b+.c](e, Load(a)))`, the `+` marking the record it
//! creates, with `a: Future<{b: Future<{c: Int}>}>` to `Store(a',
//! Update[.b!.c](e, Await(Load(a))))`, and `xs[i].up := e` to `Store(xs,
//! Update[[].up](e, Load(xs), Load(i)))`. Every child is evaluated once,
//! and the value first (see [`crate::eval`]). The text of each name a step
//! selects is recorded in the IR, for the evaluator to show a field it
//! adds.
//!
//! A dotted field, which extends a record its block is building and never
//! awaits or indexes, lowers instead to a `Store` of a chain of
//! `UpdateField` nodes, one for each segment of its path after the first,
//! from the leaf outwards, each setting its segment in the record at the
//! path before it (`Lowerer::lower_field_update`). That record is the
//! field's slot, loaded, for the first segment, and a `Field` of the record
//! before it for each later one that the static type has; a record the
//! path creates is `Empty`, as is the first record of a dotted field's
//! first definition. So `a.b.c = e`, extending `a`, which has `b`, lowers
//! to `Store(a, UpdateField(Load(a), b, UpdateField(Field(Load(a), b), c,
//! e)))`, where each record node is shared by the update that builds on it
//! and the access into it: the record is read before the value, which
//! cannot change the field's slot, so the order cannot be observed. A
//! dotted field stores back into its field's one slot. Each record block's
//! field is loaded for its record once, from the slot its first definition
//! declared. That is so only in inner blocks: the fields of the input's
//! outermost block are cells, below.
//!
//! The top-level fields of an input (see the checker's "Top-level
//! fields") are *cells*, one slot of the top level's frame for each node of
//! the tree of their paths, which the checker gives
//! ([`crate::check::Resolution::field_cells`]), each written once. Before
//! each item of the outermost block, and after the last, the cells whose
//! point is there are stored, in the order of
//! [`crate::check::Resolution::field_schedule`], dependencies first
//! (`Lowerer::store_cells`): a leaf's cell gets its initialiser's value, an
//! interior node's the `MakeRecord` of `Load`s of its children's cells in
//! the order of their first items (`Lowerer::assemble`), and a `def`'s
//! cell is its function's slot (a dotted `def`'s, its leaf's own, which is
//! that function's), stored as a top-level function's is, where its closure
//! is made, at its own item or before it: an interior node above it is
//! assembled after its last child's item, or, if a mention pulls it
//! earlier, only once the closure exists, which the checker makes sure of. A
//! field item lowers to nothing at its own line; no `UpdateField` builds a
//! top-level root item by item, so a root of N dotted items costs N stores.
//! A reference to a field, the longest declared prefix of a `NAME.f.g`
//! chain, is a `Load` of its cell from top-level code and an
//! [`IrKind::Top`] of it, of the cell's kind, from a function or `lazy` of
//! the input, followed by `Field`s for the segments beyond the prefix. The
//! record of the input is a `MakeRecord` of `Load`s of the roots' cells and
//! of the slots of the soft fields that nothing overrides, in order of first
//! definition; a top-level spread stores nothing for a name that is a
//! top-level declaration.
//!
//! In a workspace, an `import` lowers to nothing where it stands (see the
//! checker's [Imports](crate::check#imports)). A reference through an
//! imported name is lowered as a reference to the node it reaches is: a
//! `Load` of the cell from top-level code, an [`IrKind::Top`] of it from a
//! function or a `lazy`. Every read of a `def` reached through an import,
//! as a member, a path or a field of a record read whole, is of its
//! function's slot, which its file stores; never of its merged node's
//! slot, which nothing writes. Each record the file reads whole through an
//! import or its view (a directory, a file, a file's part of a shared
//! node, a node of its view without a leaf of the file under it) is stored
//! into the file's own private slot for it before its first item, nested
//! records first, each the `MakeRecord` of `Load`s of its fields' cells,
//! which the files that own them have written, having run before this one
//! (`Lowerer::store_imported_records`). A node of its view with a leaf of
//! the file under it, read whole, is stored at its point in the schedule
//! instead, as an interior node's cell is, the `MakeRecord` of `Load`s of
//! its children's cells in the view's order, the file's own stored before
//! it ([`crate::check::Resolution::view_record`]). A value file that another
//! file imports stores its result into its value's cell as the last node
//! of its top-level block, and the block's result is a `Load` of that cell.
//!
//! Comparisons are specialised by their operands' kind, and an `if` keeps
//! the empty else block the parser gave it if the source had none. `===`
//! and `!==` lower exactly as `==` and `!=` on every kind but records and
//! lists. `==` and `!=` on records and lists compare over the [`Shape`] (a
//! [`ListShape`] for a list) of the operands' static type, read from the
//! type table: one shape per distinct record or list type met in such a
//! comparison, nested record and list types included, memoised by type and
//! built through a work list rather than recursion.
//!
//! A list literal lowers to a `MakeList` of its elements' expressions, in
//! source order, a spread's operand wrapped in a `Spread`, and an index to
//! an `Index` of the element's kind. A call of a built-in function whose
//! typing is special, whose callee the checker resolved to
//! [`VarRef::Builtin`] (the name of `len` or `show`, or an alias of one:
//! see the checker's "The alias rule, temporary"), lowers to the built-in's
//! own node, of its arguments as the checker bound them, the callee never
//! read: `len` to a `ListLen` of its list, and `show` to a `Show` of its
//! value, of any kind, in the `Await` the checker marked it for, as an
//! interpolated expression is. A call of an ordinary built-in
//! function (`int`, `float`) is a `Call` of its callee, a read of its cell,
//! as a call of any function value is. A member of the `fileroot` module,
//! which the checker resolved to [`VarRef::Fileroot`], lowers to a node of
//! its own, never reading the module: `fileroot.this` and
//! `fileroot.workspace` to a `StrConst` of the file's place as the checker
//! recorded it ([`crate::check::Resolution::place`]), constants of the file
//! and of the run, `fileroot.caller` to a `Caller`, and a call of
//! `fileroot.helper` to a `Helper` (see [`crate::eval`], "Callers and
//! helpers"); and the program carries the index of its file's directory in
//! the run's table ([`Ir::dir`]). A `lazy` whose body reads
//! `fileroot.caller`, calls `fileroot.helper()` or makes a `lazy` so marked,
//! the bodies of the functions it makes not counting, lowers to a
//! `MakeFuture` marked `reads_caller`, whose making the memoisation counts
//! as a read of the caller (see [`crate::eval`], "Memoised functions"). A
//! range lowers to a `Range` of its bounds,
//! flagged if it is inclusive, and a `for` to a `For` on the slot the
//! checker gave its variable (see [`crate::check::Resolution::for_slot`]),
//! whose parts are its iterable, its guard, if it has one, and its body,
//! each as it is used, the body wrapped in an `Await` where the checker
//! marked it to fit the element type expected of the `for`. A statement,
//! an expression item that is not its block's result, is `{}` or a
//! unit-like list (see [`crate::check`]); a `for` statement whose type is
//! unit-like as it is, through any parentheses, discards its body's values
//! instead of collecting them, since its list would be dropped as soon as
//! it was built, and any other list statement is evaluated and dropped.
//! Each
//! string literal, and each literal piece of an interpolated string,
//! becomes a `StrConst` whose text is copied from the AST's string table
//! into the IR's own, so the AST's table is not needed after lowering. A
//! literal piece has no span of its own in the AST, so its node has its
//! whole interpolated string's span.
//!
//! A condition loop lowers to a `Loop` of its condition, `true` for `for {
//! ... }`, its body and the `Store`s of its header's items after the
//! condition, in a `Block` of the `Store`s of the items before it, if it
//! has any, and a `do` loop to a `Loop` that does not test first; each
//! collects or discards as a `for` does (`Lowerer::lower_for`). A `break`
//! or `continue` lowers to a `Break` or a `Continue` of its value, as it is
//! used (awaited where the checker marked it to fit its loop's element
//! type), that counts the loops it leaves before the one the checker found
//! it is for: the lowerer keeps the loops whose bodies it is lowering, of
//! the function it is lowering, on a stack, around each body only, never
//! its iterable, guard, condition or header, and starts each function's
//! body with an empty one, as the checker's frames are kept.
//!
//! A `return` lowers to a `Return` of its value, as it is used (awaited
//! where the checker marked it to fit its target), naming what it leaves
//! by a [`CatchId`]: [`CatchId::FUNCTION`] for the function or `lazy`
//! around it, and otherwise the catch of the labelled block or loop the
//! checker found it is for ([`Resolution::target`]). A labelled block that
//! some `return` leaves ([`Resolution::is_returned_block`]) lowers to a
//! `Catch` of its `Block`, the `Catch` of the block's whole kind, the
//! join's, and the `Block` of its own result's; and a loop that some
//! `return` leaves ([`Resolution::loop_of`]) to a `Catch` of its loop
//! node, so that a block or loop no `return` leaves lowers as it always
//! did. A `return` cannot name its catch by node, since a catch is built
//! after what it holds: each catch is numbered as it starts, from 0, once
//! in the program (`Lowerer::start_catch`), and the lowerer keeps the
//! catches whose blocks or loops it is lowering, of the function it is
//! lowering, on a stack, as it keeps the loops, with what each is for, on
//! which a `return` finds its number; each function's body starts with an
//! empty one. A loop that cannot end (see [`crate::check`]) is a loop
//! node that collects nothing, whatever its type, and of any kind where
//! it stands, `Never` or a `return`'s join; a statement of a union all of
//! whose members are unit-like, of kind `Mixed`, is lowered as it is, as a
//! unit-like list statement is.
//!
//! A node of type `Never` is of kind [`ValueKind::Never`], which may stand
//! where a node of any kind is expected (`Lowerer::lower_operand`): a
//! `break`, a `continue`, a block ending in one, an `if` both of whose
//! branches are such. An operation of type `Never` because an operand that
//! always runs first is (see [`crate::check`]), which the type-specialised
//! node of the operation could not hold, lowers instead to a `Block` whose
//! items are the operands that run before that one, for their effects, and
//! whose result is that operand (`Lowerer::lower_unreached`); the operands
//! after it never run, and are lowered only so that the functions in them
//! have their code, their nodes left out of the tree. A loop whose iterable
//! or first condition is `Never` lowers so too, to the items before its
//! condition and the iterable or condition. A load, field access or index
//! of type `Never` that no operand makes so is lowered as it is, of kind
//! `Never`: it is only ever in code that does not run. So is a call of type
//! `Never` that no operand makes so, which does run when its callee's
//! result is `Never`, a call of `panic`: a `Call` of kind `Never`, whose
//! callee and arguments are evaluated as any call's and which always
//! unwinds, with the panic's error (see [`crate::eval`]).
//!
//! A `lazy` lowers like a function expression: its body, any expression,
//! into the function table, as code of [`FnKind::Future`], and the
//! expression to a `MakeFuture` that captures as a `MakeClosure` does.
//!
//! A definition made with a def-type (see the checker's "Def-types") is a
//! top-level function named by the path of its namespace's key and its
//! own, lowered as a dotted `def` is: its closure stored into its leaf's
//! cell where it is made, its code shown in the dump by its def-type and
//! path, `fn #0 package dev.peios.testpkg`, and of [`FnKind::Memo`] if the
//! function type its def-type gives it is memoised, which its function
//! expression, declaring none, does not say. It is in no record. A
//! reference to one, `DT PATH`, lowers to a `MakeClosure` of a function
//! lowering makes for it, past the tree's in the function table
//! ([`crate::ast::FnId::lowered`]), that captures the def-type and the
//! definition, each read where the reference is, and whose body, of the
//! def-type's invocation parameters, is a `Block` of a `Helper`, so that
//! the def-type runs for the caller the closure is called by, and then the
//! `Call` of the def-type, captured, with the definition's path as written,
//! a `StrConst`, the definition, captured, and its own parameters, in order
//! (`Lowerer::lower_def_type_ref`). So a closure is made each time a
//! reference is evaluated, and a call of a reference is a call of it.
//!
//! A definition's meta, `defmeta DT PATH { ... }` (see the checker's
//! "Metas"), is a top-level function too, lowered as a definition is: its
//! closure stored into its leaf's cell where it is made, its code shown by
//! `defmeta`, its def-type and its path, `fn #0 defmeta package
//! dev.peios.testpkg`, and of [`FnKind::Memo`] if its marker is `META->>`,
//! which its function type, of no parameters, says. The program records,
//! for each definition whose def-type declares a meta, the cell holding its
//! meta's closure (`Ir::set_metas`, from [`Resolution::metas`]), so that a
//! closure of it names its meta. A read of a meta, `func.versions`, lowers
//! to the `Field` of a `Call` of no arguments of a `Meta` of the function,
//! which yields its meta's closure, the call of the meta's type's kind,
//! awaited, as the checker marked it, before its field is read
//! (`Lowerer::lower_meta_read`): so a read is a call, its caller, its
//! memoisation and the nesting budget those of any call.
//!
//! A reference whose def-type declares a meta carries its definition's: the
//! program records its function too, its meta that of the closure's capture
//! of the definition (`Ir::set_metas`, [`MetaAt::Capture`]), so that
//! `(package dev.peios.testpkg).versions`, or a read of a reference stored,
//! is a read of the definition's meta. A reference whose path goes on past
//! its definition, `package dev.peios.testpkg.versions` (see the checker's
//! [`crate::check::Types::reference_past`]), makes no closure: it lowers to
//! its read, whose accesses past the definition the checker typed as the
//! reads of the meta of the definition's cell they are, so it is the read of
//! the meta of the definition itself, which is the same meta.
//!
//! An expression the checker marked to be awaited (see
//! [`crate::check::Resolution::await_of`]) is lowered as it would be
//! otherwise, of the kind of its type before the await, which is
//! [`ValueKind::Deferred`] for a future or an either, and wrapped in an
//! `Await` whose kind is that of its type once awaited: its type stripped of
//! every future layer at its top, whose kind is the same as its type's once
//! awaited deeply. A deep await carries a [`LazyShape`] of that type, listing
//! the record fields to await, or for a list what to do with each element,
//! made once per type, as record shapes are,
//! and built from the leaves up through a work list. An explicit `await e`
//! is that `Await` itself, spanning the `await` expression, rather than an
//! `Await` of `e`; one of a present value is just its operand. Everywhere a
//! node of kind `Deferred` is expected, as a branch of an `if` or the result
//! of a block of such a type, a node of any kind is accepted, since a
//! present value may be where a future may.
//!
//! A union's nodes are of the kind [`TypeTable::value_kind`] gives it:
//! its members' one kind, or [`ValueKind::Mixed`] for members of different
//! kinds. Where a node of kind `Mixed` is expected, a node of any kind but
//! `Deferred` is accepted, since a value of any member's kind may be where
//! the union may, but a future may not (`Lowerer::lower_operand`). `===`
//! and `!==` of two `Mixed` operands lower to a `ValueSame` or a
//! `ValueNotSame`. A deep await of a union carries the lazy shapes of its
//! members (see [`Ir::lazy_union`]), made in the same work list as a
//! record's and a list's. An `is` lowers to an `Is` and an `as` to an `As`
//! of its operand, awaited where the checker marked it, with the test the
//! checker planned added to the IR's table, and the text of each field name
//! the plan looks at recorded for the evaluator's failure labels; an `Is`
//! that narrows has the slot the checker gave it (see
//! [`crate::check::Resolution::test_of`]).
//!
//! Once every node is built, one post-pass computes what the evaluator's
//! nesting budget needs (see [`crate::MAX_DEPTH`]): each `Call`'s and each
//! `Await`'s depth in the body of its frame, the body node being at depth 0,
//! and each function's body height, the number of nodes on its longest path
//! to a leaf. It walks the root and then each function's body, depth first
//! and iteratively, with an explicit stack, since a body can be `MAX_DEPTH`
//! deep: a call or an await gets its depth on the way down, and every node
//! its height, in one vector indexed by node, on the way up. A
//! `MakeClosure`'s or a `MakeFuture`'s captures are its children, loads in
//! the creating frame; the function it makes is not, since that body is
//! another frame, walked on its own. An `Update` gets its depth too, for
//! the awaits of its steps, and is two levels taller than its tallest child
//! for each step of its path, as the parser counts a path's segments. The
//! nodes of a frame form a tree, except the record nodes a dotted field's
//! update chain shares between the update that builds on a record and the
//! access into it: such a node's depth is counted from the first path the
//! walk reaches it by. The shared nodes are loads, field accesses and
//! `Empty`, never calls or awaits.
//!
//! Lowering cannot fail on tables produced by [`crate::check::check`]. Any
//! combination the checker should have ruled out is reported as an
//! [`InternalError`] rather than a panic.

use std::collections::HashMap;

use crate::ast::{
    ArgRange, Ast, BinaryOp, DefTypeId, Effect, ElementRange, ExprId, ExprKind, FnId, ForHead,
    ForId, ItemId, ItemKind, ItemRange, Jump, LabelId, PartRange, PathRange, PathSegment,
    SegmentKind, StrPart, UnaryOp, path_span, symbol_span,
};
use crate::check::{
    AwaitMark, Builtin, Checked, Constant, FieldSchedule, Fileroot, JumpTarget, MadeAt,
    PathResolution, Resolution, Slot, Types, VarRef,
};
use crate::mentions::{self, FieldNode};

/// How a field of a record block is loaded: its slot, the kind of its value
/// and the span of its definition.
type FieldLoad = (Slot, ValueKind, Span);

/// What an item of the outermost block contributes to the record of the
/// input as a top-level field (see `Lowerer::root_field`).
enum RootField {
    /// It is not an item the tree of fields took: lowered as any field.
    Other,
    /// A dotted item that is not the first under its root, which defines
    /// nothing more.
    Extends,
    /// The first item of its root: the root's name and how it is loaded,
    /// from the root's cell.
    First(Name, FieldLoad),
}
use crate::diagnostic::InternalError;
use crate::ir::{
    CatchId, Cmp, FnCode, FnKind, Ir, IrBuilder, IrId, IrKind, IrRange, LayoutId, LazyField,
    LazyShape, LazyShapeId, ListShape, MetaAt, Operands, Shape, ShapeId, ShapedOperands, StepAt,
    StepMode, TestCode, TestId, UpdateStep,
};
use crate::names::Name;
use crate::source::Span;

/// The capture of the closure of a def-type reference that holds the
/// definition, after the def-type's (`Lowerer::lower_def_type_ref`).
const REFERENCE_DEFINITION: u32 = 1;
use crate::types::{Laziness, TyId, TypeTable, ValueTest};
use crate::value::{Layout, ValueKind, int_to_float};

/// Lowers `ast`, whose types and resolution are `checked`, with its types
/// interned into `types`, to IR, recording that it is session input number
/// `input` (0 for a file; see [`Ir::input`]).
///
/// # Errors
///
/// Returns an [`InternalError`] if `checked` was not produced by checking
/// `ast` against `types`, or is otherwise inconsistent with it: a bug in
/// the caller or the checker, never an error in the user's program.
pub fn lower(
    ast: &Ast,
    checked: &Checked,
    types: &TypeTable,
    input: u32,
) -> Result<Ir, InternalError> {
    let Checked {
        types: expr_types,
        resolution,
    } = checked;
    let root_span = ast.root_span();
    if expr_types.len() != ast.len() {
        return Err(InternalError {
            message: "type table does not belong to this AST",
            span: root_span,
        });
    }
    if resolution.expr_len() != ast.len()
        || resolution.item_len() != ast.item_count()
        || resolution.function_len() != ast.fn_count()
        || resolution.arg_len() != ast.arg_count()
        || resolution.for_len() != ast.for_count()
    {
        return Err(InternalError {
            message: "resolution does not belong to this AST",
            span: root_span,
        });
    }
    let mut lowerer = Lowerer {
        ast,
        expr_types,
        table: types,
        resolution,
        // One IR node per AST node, less parentheses, plus any conversions,
        // stores, empty records and field loads.
        builder: IrBuilder::with_capacity(ast.len()),
        item_stack: Vec::new(),
        steps: Vec::new(),
        layouts: HashMap::new(),
        layout_names: Vec::new(),
        record_loads: Vec::new(),
        next_cell: 0,
        shapes: HashMap::new(),
        shape_pending: Vec::new(),
        lazy_shapes: HashMap::new(),
        lazy_pending: Vec::new(),
        functions: vec![None; ast.fn_count()],
        loops: Vec::new(),
        catches: Vec::new(),
        next_catch: 0,
        reads_caller: Vec::new(),
        meta_captures: Vec::new(),
    };
    let root = ast.root();
    let kind = lowerer.body_kind(root)?;
    let kind = lowerer.lower_body((None, root), root_span, kind)?;
    let root = lowerer.builder.alloc(kind, root_span);
    lowerer.stub_shadowed_functions(root_span);
    // A definition's meta is in its cell; a reference's, its definition's.
    let metas: Vec<(FnId, MetaAt)> = resolution
        .metas()
        .iter()
        .map(|&(function, slot)| (function, MetaAt::Cell(slot)))
        .chain(
            lowerer
                .meta_captures
                .iter()
                .map(|&function| (function, MetaAt::Capture(REFERENCE_DEFINITION))),
        )
        .collect();
    // Every function expression is reachable from the root, so each has
    // been lowered, but those of a shadowed declaration, stubbed.
    let mut functions = lowerer
        .functions
        .into_iter()
        .collect::<Option<Vec<FnCode>>>()
        .ok_or(InternalError {
            message: "a function expression was never lowered",
            span: root_span,
        })?;
    lowerer.builder.measure(root, &mut functions);
    let mut ir = lowerer.builder.finish_program(
        root,
        (resolution.frame_size(), resolution.slot_names().to_vec()),
        functions,
        (input, resolution.place().dir),
    );
    ir.set_paths(dotted_paths(ast, resolution));
    ir.set_metas(metas);
    Ok(ir)
}

/// Each top-level function of `ast` named by a dotted path, as `resolution`
/// lists them, with its path: what the dump shows its code by (see
/// [`Ir::set_paths`]).
fn dotted_paths(ast: &Ast, resolution: &Resolution) -> Vec<(FnId, Box<[Name]>)> {
    resolution
        .top_functions()
        .iter()
        .filter_map(|&(item, _)| {
            // A definition made with a def-type is named by its path under
            // its namespace's key, and a definition's meta by its leaf, the
            // meta key and its path as one name.
            let (key, path, value) = match ast.item(item).kind {
                ItemKind::Field { path, value, .. } => (None, path, value),
                ItemKind::DefTypeDef { def, value } => {
                    let def = ast.def_type(def);
                    (Some(def.key?), def.path, value)
                }
                ItemKind::DefMeta { def, value } => {
                    let (key, leaf) = ast.def_type(def).meta?;
                    let ExprKind::Fn(function) = ast.get(value).kind else {
                        return None;
                    };
                    return Some((function, Box::from([key, leaf])));
                }
                _ => return None,
            };
            let ExprKind::Fn(function) = ast.get(value).kind else {
                return None;
            };
            if key.is_none() && ast.path(path).len() < 2 {
                return None;
            }
            let names = key
                .into_iter()
                .chain(ast.path(path).iter().filter_map(PathSegment::name))
                .collect();
            Some((function, names))
        })
        .collect()
}

struct Lowerer<'a> {
    ast: &'a Ast,
    expr_types: &'a Types,
    table: &'a TypeTable,
    resolution: &'a Resolution,
    builder: IrBuilder,
    /// The items of the blocks being lowered, innermost last, moved to the
    /// builder's pool as each block ends, as the parser does for the AST.
    /// A record's values, and an update's parts, are collected here in the
    /// same way.
    item_stack: Vec<IrId>,
    /// The steps of the paths of the updates being lowered, innermost last,
    /// moved to the builder's table as each update ends: a position may
    /// hold a block with updates of its own.
    steps: Vec<UpdateStep>,
    /// The layout already made for each sequence of field names.
    layouts: HashMap<Box<[Name]>, LayoutId>,
    /// Reused for collecting the field names of each record block.
    layout_names: Vec<Name>,
    /// Reused for collecting how each field of each record block is
    /// loaded, beside its name in `layout_names`: its slot, the kind of its
    /// value and the span of its definition.
    record_loads: Vec<(Slot, ValueKind, Span)>,
    /// How many nodes of the schedule of the input's top-level fields
    /// ([`FieldSchedule`]) have had their cells stored: the next one to
    /// store is at that index of its order.
    next_cell: usize,
    /// The shape already made for each record and list type.
    shapes: HashMap<TyId, ShapeId>,
    /// Reused as the work list of record and list types whose shapes have
    /// ids but no fields, or element's shape, yet.
    shape_pending: Vec<TyId>,
    /// The lazy shape already made for each record and list type, or
    /// [`LazyShapeId::NONE`] if it has no lazy values at any depth.
    lazy_shapes: HashMap<TyId, LazyShapeId>,
    /// Reused as the work list of record and list types whose lazy shapes
    /// are being made, each with whether the types inside it have been
    /// queued.
    lazy_pending: Vec<(TyId, bool)>,
    /// Each function lowered so far, indexed by [`FnId`].
    functions: Vec<Option<FnCode>>,
    /// The loops whose bodies are being lowered, of the function being
    /// lowered, outermost first: how many loops out a `break` or
    /// `continue` goes is counted on it.
    loops: Vec<ForId>,
    /// The catches of the labelled blocks and loops that some `return`
    /// leaves whose items or bodies are being lowered, of the function
    /// being lowered, outermost first, each with what it catches for: a
    /// `return`'s number is found on it.
    catches: Vec<(JumpTarget, CatchId)>,
    /// The number of the next catch: one more than the last given, so that
    /// no two catches of the program share one.
    next_catch: u32,
    /// For each function or `lazy` body being lowered, innermost last,
    /// whether it can read the caller it runs with: it reads
    /// `fileroot.caller`, calls `fileroot.helper()`, or makes a `lazy` that
    /// can, a nested function's body not counting. Only a `lazy`'s is kept,
    /// as its `MakeFuture`'s mark (see [`IrKind::MakeFuture`]); empty at the
    /// top level, where nothing is kept.
    reads_caller: Vec<bool>,
    /// The functions made for def-type references that carry their
    /// definitions' metas, whose closures' metas are those of the
    /// definitions they capture (see [`MetaAt::Capture`]).
    meta_captures: Vec<FnId>,
}

impl Lowerer<'_> {
    /// The type of expression `id` where it is used: its type once awaited
    /// at the top, if it is awaited, and its type otherwise.
    fn used_type(&self, id: ExprId) -> TyId {
        let ty = self.expr_types.get(id);
        match self.resolution.await_of(id) {
            AwaitMark::None => ty,
            AwaitMark::Top | AwaitMark::Deep => self.table.strip(ty),
        }
    }

    /// The kind of the value of expression `id` where it is used: once
    /// awaited, if it is awaited. A deep await changes a record's fields,
    /// not its kind, so the type stripped at the top gives it.
    fn kind(&self, id: ExprId) -> Result<ValueKind, InternalError> {
        self.kind_of_type(id, self.used_type(id))
    }

    /// The kind of the value of expression `id` before any await: what the
    /// node it lowers to yields.
    fn plain_kind(&self, id: ExprId) -> Result<ValueKind, InternalError> {
        self.kind_of_type(id, self.expr_types.get(id))
    }

    /// The kind of the values of `ty`, the type of expression `id`, or an
    /// internal error if it has none.
    fn kind_of_type(&self, id: ExprId, ty: TyId) -> Result<ValueKind, InternalError> {
        let span = self.ast.get(id).span;
        match ty {
            TyId::UNTYPED_INT => Err(InternalError {
                message: "an untyped integer survived type checking",
                span,
            }),
            TyId::ERROR => Err(InternalError {
                message: "a type error survived type checking",
                span,
            }),
            ty => self.table.value_kind(ty).ok_or(InternalError {
                message: "a type with no values survived type checking",
                span,
            }),
        }
    }

    /// The kind of a body's value: a record for a record block, its
    /// result's kind, or a record (`{}`) if it has none.
    fn body_kind(&self, body: ItemRange) -> Result<ValueKind, InternalError> {
        if self.is_record_body(body) {
            return Ok(ValueKind::Record);
        }
        match self
            .ast
            .items(body)
            .last()
            .map(|&id| self.ast.item(id).kind)
        {
            Some(ItemKind::Result(value) | ItemKind::Expr(value)) => self.kind(value),
            Some(ItemKind::Break(_) | ItemKind::Continue(_) | ItemKind::Return(_)) => {
                Ok(ValueKind::Never)
            }
            Some(
                ItemKind::Let { .. }
                | ItemKind::Rebind { .. }
                | ItemKind::Field { .. }
                | ItemKind::Spread(_)
                | ItemKind::Import(_)
                | ItemKind::Type { .. }
                | ItemKind::DefTypeDef { .. }
                | ItemKind::DefMeta { .. },
            )
            | None => Ok(ValueKind::Record),
            Some(ItemKind::Error) => Err(unparsed(self.ast.root_span())),
        }
    }

    /// Whether `value` is directly an `await` expression.
    fn is_await(&self, value: ExprId) -> bool {
        matches!(
            self.ast.get(value).kind,
            ExprKind::Unary {
                op: UnaryOp::Await,
                ..
            }
        )
    }

    /// Whether `body` has a field or spread item, which makes it a record
    /// block.
    fn is_record_body(&self, body: ItemRange) -> bool {
        self.ast.items(body).iter().any(|&id| {
            matches!(
                self.ast.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        })
    }

    /// Lowers a body whose value is of kind `kind` to a `Block`, which the
    /// caller allocates: the body of the block `block`, or the root's for
    /// `None`. A `{}` the checker gave fields their defaults (see the
    /// checker's "Field defaults") is a record of them, as a record block
    /// given them is.
    fn lower_body(
        &mut self,
        (block, body): (Option<ExprId>, ItemRange),
        span: Span,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let items = self.ast.items(body);
        let is_record = self.is_record_body(body)
            || block.is_some_and(|block| !self.resolution.record_fills(block).is_empty());
        let base = self.item_stack.len();
        let mut result = None;
        let is_root = body == self.ast.root();
        if is_root {
            self.store_imported_records(span);
            self.store_top_closures(None)?;
        }
        for (index, &id) in items.iter().enumerate() {
            // A record block's final expression is a statement.
            let is_result = !is_record && index + 1 == items.len();
            let item = self.ast.item(id);
            if is_root {
                self.store_cells(index)?;
            }
            // A shadowed declaration is never run (see
            // [`crate::mentions`], "Shadowed declarations").
            if is_root && self.is_shadowed(index) {
                continue;
            }
            if is_root && self.is_stored_elsewhere(id) {
                self.store_top_closures(Some(id))?;
                continue;
            }
            if is_root && self.is_cell_item(index) {
                // Its leaf was stored at its point, at this item or before.
                self.store_top_closures(Some(id))?;
                continue;
            }
            match item.kind {
                // A definition made with a def-type, and a definition's
                // meta, is a top-level function, whose closure is stored
                // into its leaf's cell here if it is made here.
                ItemKind::Let { .. }
                | ItemKind::Rebind { .. }
                | ItemKind::Field { .. }
                | ItemKind::DefTypeDef { .. }
                | ItemKind::DefMeta { .. } => {
                    let store = self.lower_store(id)?;
                    self.item_stack.push(store);
                }
                ItemKind::Spread(value) => self.lower_spread(id, value, item.span)?,
                // An import computes nothing at its point: what it names
                // is read where it is mentioned. Nor does a `type`, which
                // has no existence at runtime.
                ItemKind::Import(_) | ItemKind::Type { .. } => {}
                ItemKind::Error => return Err(unparsed(item.span)),
                ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
                    let node = self.lower_jump(id, jump)?;
                    if is_result {
                        result = Some(node);
                    } else {
                        self.item_stack.push(node);
                    }
                }
                ItemKind::Result(value) | ItemKind::Expr(value) if is_result => {
                    result = Some(self.lower_operand(value, kind)?);
                }
                ItemKind::Result(_) => {
                    return Err(InternalError {
                        message: "a result before the end of its block, or in a record block, \
                                  survived checking",
                        span: item.span,
                    });
                }
                ItemKind::Expr(value) => {
                    // Evaluated for effect only, so it must be `{}` or a
                    // unit-like list, unless it is a bare `await`, whose
                    // value is discarded whatever it is. A unit-like `for`
                    // discards its body's values rather than build a list.
                    let statement = if self.is_await(value) {
                        self.lower_expr(value)?
                    } else if let Some((for_id, span)) = self.discarded_for(value) {
                        let kind = self.kind(value)?;
                        let kind = self.lower_for(for_id, (kind, false), span)?;
                        self.builder.alloc(kind, span)
                    } else if matches!(self.kind(value)?, ValueKind::List | ValueKind::Mixed) {
                        // A unit-like list, or a union of unit-like members.
                        self.lower_expr(value)?
                    } else {
                        self.lower_operand(value, ValueKind::Record)?
                    };
                    self.item_stack.push(statement);
                }
            }
            if is_root {
                self.store_top_closures(Some(id))?;
            }
        }
        if is_root {
            self.store_cells(items.len())?;
            self.store_own_records()?;
        }
        let mut result = match result {
            Some(result) => result,
            None if kind != ValueKind::Record => {
                return Err(InternalError {
                    message: "a block without a result is not a record",
                    span,
                });
            }
            None if is_record => self.make_record((block, items), is_root, span)?,
            None => self.builder.alloc(IrKind::Empty, span),
        };
        // An imported value file's value is a cell: stored as the block
        // ends, the last item of the top level (see [Imports](crate::check#imports)).
        let value_cell = self.resolution.value_cell();
        if is_root && value_cell != Slot::NONE {
            let store = self.builder.alloc(IrKind::Store(value_cell, result), span);
            self.item_stack.push(store);
            result = self.builder.alloc(IrKind::Load(value_cell, kind), span);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Block { items, result })
    }

    /// Gives each function expression and `lazy` of the root block's
    /// shadowed declarations, which lower to nothing and so are never made
    /// (see [`crate::mentions`], "Shadowed declarations"), code that does
    /// nothing, at `span`: the function table has an entry for every one.
    fn stub_shadowed_functions(&mut self, span: Span) {
        let ast = self.ast;
        let root = ast.items(ast.root());
        for (position, &id) in root.iter().enumerate() {
            let (ItemKind::Field { value, .. } | ItemKind::DefTypeDef { value, .. }) =
                ast.item(id).kind
            else {
                continue;
            };
            if !self.is_shadowed(position) {
                continue;
            }
            for expr in ast.subtree(value) {
                let (function, kind) = match ast.get(expr).kind {
                    ExprKind::Fn(function) => (function, self.fn_kind(expr, function)),
                    ExprKind::Lazy(function) => (function, FnKind::Future),
                    _ => continue,
                };
                let params = u32::try_from(ast.params(ast.fn_decl(function).params).len())
                    .unwrap_or(u32::MAX);
                let body = self.builder.alloc(IrKind::Empty, span);
                if let Some(entry @ None) = self.functions.get_mut(function.index()) {
                    *entry = Some(FnCode {
                        body,
                        kind,
                        frame_size: params,
                        param_count: params,
                        capture_count: 0,
                        height: 1,
                        name: None,
                        display: Box::from(""),
                        slot_names: Box::from([]),
                    });
                }
            }
        }
    }

    /// Whether the item at `position` of the root block is a declaration a
    /// later one shadows, which lowers to nothing and is in no record.
    fn is_shadowed(&self, position: usize) -> bool {
        self.resolution
            .field_schedule()
            .tree
            .shadowed
            .get(position)
            .copied()
            .unwrap_or(false)
    }

    /// Whether the root item `id` is a top-level function whose closure is
    /// made somewhere other than at its own declaration (see
    /// [`MadeAt`]): it then lowers to nothing where it is written, the
    /// `Store` of its closure being placed by
    /// [`Lowerer::store_top_closures`]. A `def`'s field is still where it
    /// is written in its record, which reads the slot when it is made.
    fn is_stored_elsewhere(&self, id: ItemId) -> bool {
        self.resolution
            .top_functions()
            .iter()
            .any(|&(item, made)| item == id && made != MadeAt::Own)
    }

    /// Pushes, as items of the root block, the `Store`s of the closures of
    /// the top-level functions made at `at`, in source order: those made
    /// as the input starts for `None`, and those made just after the root
    /// item `item` for `Some(item)` (see [`MadeAt`]).
    fn store_top_closures(&mut self, at: Option<ItemId>) -> Result<(), InternalError> {
        let resolution = self.resolution;
        for &(item, made) in resolution.top_functions() {
            let here = match (made, at) {
                (MadeAt::Start, None) => true,
                (MadeAt::After(after), Some(at)) => after == at,
                _ => false,
            };
            if here {
                let store = self.lower_store(item)?;
                self.item_stack.push(store);
            }
        }
        Ok(())
    }

    /// Whether the item at `position` of the root block is a top-level
    /// field item whose leaf has a cell of its own: it then lowers to
    /// nothing where it is written, its `Store` being placed at its point by
    /// [`Lowerer::store_cells`]. A `def` is not one: its cell is its
    /// function's slot, stored as a top-level function's is.
    fn is_cell_item(&self, position: usize) -> bool {
        let tree = &self.resolution.field_schedule().tree;
        tree.item_leaf
            .get(position)
            .copied()
            .flatten()
            .and_then(|node| tree.nodes.get(node as usize))
            .and_then(|node| node.leaf)
            .is_some_and(|leaf| mentions::top_level_function(self.ast, leaf.item).is_none())
    }

    /// Pushes, as items of the root block, the `Store`s of the cells of the
    /// top-level field nodes whose point is at or before the item at
    /// `point` (the number of items for after the last), in the order the
    /// schedule computes them, each once (see [Cells](crate::ir#cells)): a
    /// leaf's initialiser, or the record of an interior node's children,
    /// loaded from their cells in the order of their first items; for a
    /// node of a workspace file's view, the record of its children in the
    /// view's order, if the file reads it whole.
    fn store_cells(&mut self, point: usize) -> Result<(), InternalError> {
        let resolution = self.resolution;
        let FieldSchedule { tree, schedule } = resolution.field_schedule();
        let root = self.ast.items(self.ast.root());
        while let Some(&index) = schedule.order.get(self.next_cell) {
            let at = schedule
                .point
                .get(index as usize)
                .copied()
                .unwrap_or(usize::MAX);
            if at > point {
                break;
            }
            self.next_cell += 1;
            if let Some((slot, first, fields)) = resolution.view_record(index) {
                if *slot != Slot::NONE {
                    let span = root
                        .get(*first)
                        .map_or_else(|| self.ast.root_span(), |&item| self.ast.item(item).span);
                    self.store_record(*slot, fields, span);
                }
                continue;
            }
            let slot = resolution.field_cells().get(index as usize).copied();
            let (Some(node), Some(slot)) = (tree.nodes.get(index as usize), slot) else {
                continue;
            };
            if slot == Slot::NONE {
                continue;
            }
            let value = match node.leaf {
                Some(leaf) if mentions::top_level_function(self.ast, leaf.item).is_some() => {
                    continue;
                }
                Some(leaf) => self.lower_expr(leaf.value)?,
                None => self.assemble(node, index, root, false)?,
            };
            let span = self.node_span(node, root);
            let store = self.builder.alloc(IrKind::Store(slot, value), span);
            self.item_stack.push(store);
        }
        Ok(())
    }

    /// In a file of a workspace, before its first item: stores each record
    /// it reads whole through an import into its private slot, nested
    /// records first, each a `MakeRecord` of `Load`s of the fields' cells,
    /// which the files that own them, run before this one, have written (a
    /// `def`'s cell being its function's slot; see
    /// [Imports](crate::check#imports)).
    fn store_imported_records(&mut self, span: Span) {
        let resolution = self.resolution;
        for (slot, fields) in resolution.imported_records() {
            self.store_record(*slot, fields, span);
        }
    }

    /// Pushes, as an item of the root block, the `Store` into `slot` of
    /// the `MakeRecord` of `fields`, a `Load` of each one's cell at its
    /// name, of the kind of its type, shown at `span`: a record a file of a
    /// workspace assembles of other files' cells, and of its own computed
    /// before it (see [Imports](crate::check#imports)).
    fn store_record(&mut self, slot: Slot, fields: &[(Name, Slot, TyId)], span: Span) {
        self.layout_names.clear();
        self.record_loads.clear();
        for &(name, cell, ty) in fields {
            let kind = self.table.value_kind(ty).unwrap_or(ValueKind::Record);
            self.define_field(name, (cell, kind, span), false);
        }
        let base = self.item_stack.len();
        for &(cell, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(cell, kind), field_span);
            self.item_stack.push(load);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let record = self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span);
        let store = self.builder.alloc(IrKind::Store(slot, record), span);
        self.item_stack.push(store);
    }

    /// In a file of a workspace, after its last item: stores, children
    /// first, the file's own record of each node it shares with other
    /// files, which its value holds in place of the shared node's cell,
    /// whose value has the other files' parts (see [Directory
    /// scope](crate::check#directory-scope)).
    fn store_own_records(&mut self) -> Result<(), InternalError> {
        let resolution = self.resolution;
        let tree = &resolution.field_schedule().tree;
        let root = self.ast.items(self.ast.root());
        for (index, &slot) in resolution.own_records().iter().enumerate().rev() {
            let Some(node) = tree.nodes.get(index).filter(|_| slot != Slot::NONE) else {
                continue;
            };
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            let value = self.assemble(node, index, root, true)?;
            let span = self.node_span(node, root);
            let store = self.builder.alloc(IrKind::Store(slot, value), span);
            self.item_stack.push(store);
        }
        Ok(())
    }

    /// The `MakeRecord` of the interior node `node` of the tree of the
    /// input's top-level fields, whose root block's items are `root`: a
    /// `Load` of each child's cell, at its name, in the order of their
    /// first items, a leaf's of the kind of its initialiser and an interior
    /// node's as a record. The node is `index` of the tree. In a file of a
    /// workspace, a node shared with other files has their children too,
    /// after its own, unless `own`, which builds the file's own record of
    /// it, from its own children's own records (see [Directory
    /// scope](crate::check#directory-scope)).
    fn assemble(
        &mut self,
        node: &FieldNode,
        index: u32,
        root: &[ItemId],
        own: bool,
    ) -> Result<IrId, InternalError> {
        let resolution = self.resolution;
        let tree = &resolution.field_schedule().tree;
        self.layout_names.clear();
        self.record_loads.clear();
        let shared = resolution.shared_children(index);
        let merged = !own && !shared.is_empty();
        // A shared node's children of other files, in the merged node's
        // order, its own among them.
        if merged {
            let span = self.node_span(node, root);
            for &(name, slot, ty) in shared {
                let leaf = node
                    .children
                    .iter()
                    .filter_map(|&child| tree.nodes.get(child as usize))
                    .find(|child| child.name == name)
                    .map(|child| child.leaf);
                let kind = match leaf {
                    Some(Some(leaf)) => self.kind(leaf.value)?,
                    Some(None) => ValueKind::Record,
                    None => self.table.value_kind(ty).unwrap_or(ValueKind::Record),
                };
                self.define_field(name, (slot, kind, span), false);
            }
        }
        for &child in node.children.iter().filter(|_| !merged) {
            let (Some(child_node), Some(&slot)) = (
                tree.nodes.get(child as usize),
                resolution.field_cells().get(child as usize),
            ) else {
                continue;
            };
            let kind = match child_node.leaf {
                Some(leaf) => self.kind(leaf.value)?,
                None => ValueKind::Record,
            };
            let slot = match resolution.own_records().get(child as usize) {
                Some(&own_slot) if own && own_slot != Slot::NONE => own_slot,
                _ => slot,
            };
            let span = self.node_span(child_node, root);
            self.define_field(child_node.name, (slot, kind, span), false);
        }
        let base = self.item_stack.len();
        for &(slot, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(slot, kind), field_span);
            self.item_stack.push(load);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let span = self.node_span(node, root);
        Ok(self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span))
    }

    /// The span of the first item at or under `node`, a node of the tree of
    /// the input's top-level fields, whose root block's items are `root`:
    /// where its cell's `Store` and its loads are shown.
    fn node_span(&self, node: &FieldNode, root: &[ItemId]) -> Span {
        root.get(node.first)
            .map_or(Span::new(0, 0), |&id| self.ast.item(id).span)
    }

    /// Lowers the declaration `id`, a `let`, a field or a `:=`, to the
    /// `Store` of its value (see [`Lowerer::lower_declaration`]) into the
    /// slot the checker gave it.
    fn lower_store(&mut self, id: ItemId) -> Result<IrId, InternalError> {
        let item = self.ast.item(id);
        // A definition made with a def-type, or a definition's meta, has no
        // path of its own to update: its closure is stored in its leaf's
        // cell.
        let (path, value) = match item.kind {
            ItemKind::Let { path, value, .. }
            | ItemKind::Rebind { path, value }
            | ItemKind::Field { path, value, .. } => (Some(path), value),
            ItemKind::DefTypeDef { value, .. } | ItemKind::DefMeta { value, .. } => (None, value),
            _ => {
                return Err(InternalError {
                    message: "an item that declares nothing lowered as a declaration",
                    span: item.span,
                });
            }
        };
        let slot = self.resolution.item_slot(id);
        if slot == Slot::NONE {
            return Err(InternalError {
                message: "a declaration has no slot",
                span: item.span,
            });
        }
        let value = match path {
            Some(path) => self.lower_declaration(id, path, value)?,
            None => self.lower_expr(value)?,
        };
        Ok(self.builder.alloc(IrKind::Store(slot, value), item.span))
    }

    /// Lowers the `break` or `continue` item `id`, whose value, if any, is
    /// `jump`'s, to a `Break` or a `Continue` of its value as it is used,
    /// counting the loops it leaves before the one the checker found it is
    /// for (see [`Resolution::jump_target`]), which is among those whose
    /// bodies are being lowered.
    fn lower_jump(&mut self, id: ItemId, jump: Jump) -> Result<IrId, InternalError> {
        let item = self.ast.item(id);
        if let ItemKind::Return(_) = item.kind {
            return self.lower_return(id, jump);
        }
        let target = self.resolution.jump_target(id);
        let Some(at) = self
            .loops
            .iter()
            .rposition(|&loop_id| Some(loop_id) == target)
        else {
            return Err(InternalError {
                message: "a `break` or `continue` is not in the body of its loop",
                span: item.span,
            });
        };
        let loops = u32::try_from(self.loops.len() - 1 - at).unwrap_or(u32::MAX);
        let value = jump.value.map(|value| self.lower_expr(value)).transpose()?;
        let kind = match item.kind {
            ItemKind::Break(_) => IrKind::Break { loops, value },
            _ => IrKind::Continue { loops, value },
        };
        Ok(self.builder.alloc(kind, item.span))
    }

    /// Lowers the `return` item `id`, whose value, if any, is `jump`'s, to
    /// a `Return` of its value as it is used (awaited where the checker
    /// marked it, to fit its target), naming what the checker found it
    /// leaves (see [`Resolution::target`]): [`CatchId::FUNCTION`] for the
    /// function or `lazy` around it, or the number of the catch of the
    /// labelled block or loop it names, which is among those being
    /// lowered, on [`Lowerer::catches`].
    fn lower_return(&mut self, id: ItemId, jump: Jump) -> Result<IrId, InternalError> {
        let item = self.ast.item(id);
        let to = match self.resolution.target(id) {
            Some(JumpTarget::Function) => CatchId::FUNCTION,
            Some(target) => self
                .catches
                .iter()
                .rev()
                .find(|&&(around, _)| around == target)
                .map(|&(_, catch)| catch)
                .ok_or(InternalError {
                    message: "a `return` is not in the block or loop it leaves",
                    span: item.span,
                })?,
            None => {
                return Err(InternalError {
                    message: "a `return` has nothing to leave",
                    span: item.span,
                });
            }
        };
        let value = jump.value.map(|value| self.lower_expr(value)).transpose()?;
        Ok(self.builder.alloc(IrKind::Return { to, value }, item.span))
    }

    /// Starts a catch for `target`, a labelled block or a loop that some
    /// `return` leaves: numbers it, the next number of the program, and
    /// puts it on [`Lowerer::catches`] for its block's items or its loop's
    /// body, which the caller lowers and then ends it with
    /// [`Lowerer::end_catch`].
    fn start_catch(&mut self, target: JumpTarget) -> CatchId {
        let id = CatchId::new(self.next_catch);
        self.next_catch = self.next_catch.saturating_add(1);
        self.catches.push((target, id));
        id
    }

    /// Ends the catch `id`, started by [`Lowerer::start_catch`], whose
    /// block or loop lowered to `body`, spanning `span`: an
    /// [`IrKind::Catch`] of it, of kind `kind`, the whole's.
    fn end_catch(&mut self, id: CatchId, (body, span): (IrKind, Span), kind: ValueKind) -> IrKind {
        self.catches.pop();
        let body = self.builder.alloc(body, span);
        IrKind::Catch { body, id, kind }
    }

    /// Lowers the value that the declaration `id`, whose path is `path`,
    /// stores: its expression `value` for a plain path, for a dotted field
    /// the chain of field updates that sets the path to it, and for a dotted
    /// `let` or `:=` the update that does.
    fn lower_declaration(
        &mut self,
        id: ItemId,
        path: PathRange,
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        match (self.ast.item(id).kind, self.ast.path(path)) {
            (_, [_]) => self.lower_expr(value),
            // A top-level function named by a dotted path: its closure,
            // stored in its leaf's cell.
            (ItemKind::Field { function: true, .. }, _) if self.is_top_function(id) => {
                self.lower_expr(value)
            }
            (ItemKind::Field { .. }, segments) => self.lower_field_update(id, segments, value),
            (_, segments) => self.lower_update(id, segments, value),
        }
    }

    /// Whether the item `id` is one of the input's top-level functions.
    fn is_top_function(&self, id: ItemId) -> bool {
        self.resolution
            .top_functions()
            .iter()
            .any(|&(item, _)| item == id)
    }

    /// Lowers the dotted field `id`, whose path is `segments`, to the chain
    /// of `UpdateField`s setting the path to `value`, from the leaf
    /// outwards, each on the record at the path before its segment: the
    /// field's slot or a `Field` of the record before it where they exist,
    /// and `Empty` where the path creates a record. A dotted field extends a
    /// record its block is building, which is never awaited.
    fn lower_field_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        let table = self.resolution;
        let resolution = table.item_path(id);
        let existing = resolution.existing as usize;
        if !table
            .path_awaits(resolution)
            .is_some_and(<[bool]>::is_empty)
        {
            return Err(InternalError {
                message: "a dotted field's record is awaited",
                span: path_span(segments),
            });
        }
        let base = self.item_stack.len();
        // The record at each proper prefix of the path, `segments[..=end]`,
        // on the item stack: the record that setting `segments[end + 1]`
        // builds on.
        for end in 0..segments.len().saturating_sub(1) {
            let span = path_span(&segments[..=end]);
            let kind = if end >= existing {
                IrKind::Empty
            } else if end == 0 {
                read(resolution.source, ValueKind::Record).ok_or(InternalError {
                    message: "an update of an existing record has no source",
                    span,
                })?
            } else {
                let (Some(&record), Some(name)) =
                    (self.item_stack.get(base + end - 1), segments[end].name())
                else {
                    return Err(InternalError {
                        message: "an update's record is missing",
                        span,
                    });
                };
                IrKind::Field {
                    record,
                    name,
                    kind: ValueKind::Record,
                }
            };
            let record = self.builder.alloc(kind, span);
            self.item_stack.push(record);
        }
        let mut update = self.lower_expr(value)?;
        for (level, segment) in segments.iter().enumerate().skip(1).rev() {
            let (Some(&record), Some(name)) =
                (self.item_stack.get(base + level - 1), segment.name())
            else {
                return Err(InternalError {
                    message: "an update's record is missing",
                    span: path_span(segments),
                });
            };
            // The evaluator shows a field it adds by this text.
            self.builder.add_name_text(name, self.ast.names().get(name));
            update = self.builder.alloc(
                IrKind::UpdateField {
                    record,
                    name,
                    value: update,
                },
                path_span(&segments[..=level]),
            );
        }
        self.item_stack.truncate(base);
        Ok(update)
    }

    /// Lowers the dotted `let` or `:=` `id`, whose path is `segments`, to
    /// an `Update` setting the path to `value`, spanning the path: its
    /// parts are `value`, then the record or list updated, the variable's
    /// slot or capture read, wrapped in an `Await` if the checker marked it
    /// to be awaited, or `Empty` for a `let` of a name not in scope, and
    /// then the position of each index segment, in order; and its steps,
    /// one for each segment after the first, select a field or an element
    /// and read it, read and await it where the checker marked it, create
    /// it where the static type lacks it, or, for the last, set it.
    fn lower_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        value: ExprId,
    ) -> Result<IrId, InternalError> {
        let table = self.resolution;
        let resolution = table.item_path(id);
        let existing = resolution.existing as usize;
        let span = path_span(segments);
        let awaits = table.path_awaits(resolution).ok_or(InternalError {
            message: "an update's awaited records are missing",
            span,
        })?;
        let base = self.item_stack.len();
        // The value is evaluated first.
        let value = self.lower_expr(value)?;
        self.item_stack.push(value);
        let root = self.update_root(segments, resolution, awaits)?;
        self.item_stack.push(root);
        let steps = self.steps.len();
        for (level, segment) in segments.iter().enumerate().skip(1) {
            let at = match segment.kind {
                SegmentKind::Name(name) => {
                    // The evaluator shows a field it adds by this text.
                    self.builder.add_name_text(name, self.ast.names().get(name));
                    StepAt::Field(name)
                }
                SegmentKind::Index { position, .. } => {
                    let position = self.lower_operand(position, ValueKind::Int)?;
                    self.item_stack.push(position);
                    StepAt::Element
                }
            };
            // The record or list the step selects is the one at
            // `segments[..=level]`.
            let mode = if level + 1 == segments.len() {
                StepMode::Set
            } else if level >= existing {
                StepMode::Create
            } else if awaits.get(level).copied().unwrap_or(false) {
                StepMode::Await
            } else {
                StepMode::Read
            };
            self.steps.push(UpdateStep {
                at,
                mode,
                span: path_span(&segments[..=level]),
            });
        }
        let path = self
            .builder
            .push_update_path(self.steps.get(steps..).unwrap_or_default());
        self.steps.truncate(steps);
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let kind = updated_kind(segments);
        Ok(self.builder.alloc(
            IrKind::Update {
                parts,
                path,
                kind,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }

    /// The node reading the record or list that the dotted `let` or `:=`
    /// whose path is `segments`, and which resolves as `resolution` says,
    /// updates, spanning its first segment: a read of its variable's slot
    /// or capture, as a use of the variable would read it, wrapped in an
    /// `Await` if `awaits`, the checker's marks for the records and lists
    /// along the path, says so for it, or `Empty` if no variable is
    /// updated.
    fn update_root(
        &mut self,
        segments: &[PathSegment],
        resolution: PathResolution,
        awaits: &[bool],
    ) -> Result<IrId, InternalError> {
        let span = segments.first().map_or(Span::new(0, 0), |first| first.span);
        if resolution.existing == 0 {
            return Ok(self.builder.alloc(IrKind::Empty, span));
        }
        let kind = updated_kind(segments);
        // A record or list that may be a future is read as it is and
        // awaited, as the record of a field access is.
        let awaited = awaits.first().copied().unwrap_or(false);
        let read_kind = if awaited { ValueKind::Deferred } else { kind };
        let read = read(resolution.source, read_kind).ok_or(InternalError {
            message: "an update of an existing record has no source",
            span,
        })?;
        let read = self.builder.alloc(read, span);
        if !awaited {
            return Ok(read);
        }
        Ok(self.builder.alloc(
            IrKind::Await {
                value: read,
                kind,
                shape: LazyShapeId::NONE,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }

    /// Lowers the spread item `id`, whose operand is `value`, spanning
    /// `span`, onto the item stack: a `Store` of the operand, awaited if the
    /// checker marked it to be, into the item's own slot, so that it is
    /// evaluated once, and then, for each field of the operand's static
    /// type once awaited, in the type's order, a `Store` into the field's
    /// slot (see [`Resolution::spread_slots`]) of a `Field` of a `Load` of
    /// the item's slot, which reads the field by name. A field the runtime
    /// value holds but the static type hides is not read.
    fn lower_spread(&mut self, id: ItemId, value: ExprId, span: Span) -> Result<(), InternalError> {
        let (table, resolution) = (self.table, self.resolution);
        let held = resolution.item_slot(id);
        if held == Slot::NONE {
            return Err(InternalError {
                message: "a spread has no slot",
                span,
            });
        }
        let operand = self.lower_expr(value)?;
        let store = self.builder.alloc(IrKind::Store(held, operand), span);
        self.item_stack.push(store);
        // An operand of type `Never` never gives a record to read.
        if self.kind(value)? == ValueKind::Never {
            return Ok(());
        }
        let operand_span = self.ast.get(value).span;
        let fields = table
            .record_fields(table.strip(self.used_type(value)))
            .ok_or(InternalError {
                message: "a spread of a value that is not a record survived checking",
                span: operand_span,
            })?;
        let slots = resolution.spread_slots(id);
        if slots.len() != fields.len() {
            return Err(InternalError {
                message: "a spread's fields have no slots",
                span,
            });
        }
        // A field whose name is a top-level field of the input has no slot:
        // the spread writes nothing for it, whichever is written first.
        for (&(name, ty), &slot) in fields.iter().zip(slots) {
            if slot == Slot::NONE {
                continue;
            }
            let kind = table.value_kind(ty).ok_or(InternalError {
                message: "a type with no values survived type checking",
                span: operand_span,
            })?;
            let record = self
                .builder
                .alloc(IrKind::Load(held, ValueKind::Record), operand_span);
            let field = self
                .builder
                .alloc(IrKind::Field { record, name, kind }, operand_span);
            let store = self.builder.alloc(IrKind::Store(slot, field), span);
            self.item_stack.push(store);
        }
        Ok(())
    }

    /// Builds the `MakeRecord` of the record block of `items`, spanning
    /// `span`, whose fields have been stored: a `Load` of each field's slot,
    /// at its name, in the order the fields were first defined, of the kind
    /// of its definition that stands. An open field is loaded once, from
    /// the slot its first definition declared, as a record. A soft field,
    /// one a spread defines, is loaded as the type its spread's operand
    /// gives it, unless a later item overrides it; since only a spread
    /// makes a field that may be defined again, the names defined so far
    /// are searched for the one being defined only from the block's first
    /// spread on.
    ///
    /// In the root block (`is_root`), a top-level field is loaded from the
    /// cell of its root (see [Cells](crate::ir#cells)), at the first item
    /// under that root, and a spread's field that a top-level declaration
    /// of its name overrides, below it or above, takes the declaration's
    /// load.
    ///
    /// The fields the checker gave the block, `block` (`None` for the root,
    /// which is given none), their defaults (see the checker's "Field
    /// defaults", [`crate::check::Resolution::record_fills`]) follow those
    /// it defines, in the order of their names' text, each its constant's
    /// nodes (`Lowerer::lower_constant`) at the block's closing brace; a
    /// `{}` given fields has only those.
    fn make_record(
        &mut self,
        (block, items): (Option<ExprId>, &[ItemId]),
        is_root: bool,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let (table, resolution) = (self.table, self.resolution);
        self.layout_names.clear();
        self.record_loads.clear();
        let mut spread = false;
        for (position, &id) in items.iter().enumerate() {
            if is_root && self.is_shadowed(position) {
                continue;
            }
            let (path, value) = match self.ast.item(id).kind {
                ItemKind::Field { path, value, .. } => (path, value),
                ItemKind::Spread(operand) => {
                    spread = true;
                    let operand_span = self.ast.get(operand).span;
                    let fields = table
                        .record_fields(table.strip(self.used_type(operand)))
                        .unwrap_or_default();
                    // A type holds its fields in the order their names
                    // were interned, which is no order a reader knows: the
                    // fields take their places in the order of their names'
                    // text.
                    let names = self.ast.names();
                    let mut defined: Vec<_> = fields
                        .iter()
                        .zip(resolution.spread_slots(id))
                        .map(|(&(name, ty), &slot)| (name, ty, slot))
                        .collect();
                    defined.sort_unstable_by_key(|&(name, _, _)| names.get(name));
                    for (name, ty, slot) in defined {
                        let kind = table.value_kind(ty).ok_or(InternalError {
                            message: "a type with no values survived type checking",
                            span: operand_span,
                        })?;
                        // A top-level declaration of the name stands
                        // wherever it is: the spread wrote no slot for it.
                        if slot == Slot::NONE && self.layout_names.contains(&name) {
                            continue;
                        }
                        self.define_field(name, (slot, kind, operand_span), spread);
                    }
                    continue;
                }
                _ => continue,
            };
            // A `def` of one name is its own root, whose slot is its
            // function's; one named by a dotted path is a leaf under its
            // root, as any dotted field is.
            if is_root
                && mentions::top_level_function(self.ast, id)
                    .is_none_or(|(name, _, _)| name.is_none())
            {
                match self.root_field(id)? {
                    RootField::Other => {}
                    RootField::Extends => continue,
                    RootField::First(name, load) => {
                        self.define_field(name, load, spread);
                        continue;
                    }
                }
            }
            let (kind, field) = match self.ast.path(path) {
                [field] => (self.kind(value)?, field),
                // A later definition of an open field extends it.
                [field, ..] if resolution.item_path(id).source == VarRef::None => {
                    (ValueKind::Record, field)
                }
                _ => continue,
            };
            // A field's path's segments are all names.
            let Some(name) = field.name() else {
                continue;
            };
            let slot = resolution.item_slot(id);
            self.define_field(name, (slot, kind, field.span), spread);
        }
        let base = self.item_stack.len();
        for &(slot, kind, field_span) in &self.record_loads {
            let load = self.builder.alloc(IrKind::Load(slot, kind), field_span);
            self.item_stack.push(load);
        }
        let close = Span::new(span.end.saturating_sub(1), span.end);
        let fills = block.map_or(&[][..], |block| resolution.record_fills(block));
        for (name, constant) in fills {
            let value = self.lower_constant(constant, close);
            self.item_stack.push(value);
            self.layout_names.push(*name);
        }
        let layout = self.layout();
        let values = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(self
            .builder
            .alloc(IrKind::MakeRecord { layout, values }, span))
    }

    /// The field of the record of the input that the root item `id` defines,
    /// as a [`RootField`].
    fn root_field(&self, id: ItemId) -> Result<RootField, InternalError> {
        let resolution = self.resolution;
        let tree = &resolution.field_schedule().tree;
        let root = self.ast.items(self.ast.root());
        let Some(position) = root.iter().position(|&item| item == id) else {
            return Ok(RootField::Other);
        };
        let Some(mut at) = tree.item_leaf.get(position).copied().flatten() else {
            return Ok(RootField::Other);
        };
        let leaf = at;
        while let Some(parent) = tree.nodes.get(at as usize).and_then(|node| node.parent) {
            at = parent;
        }
        let (Some(node), Some(&slot)) = (
            tree.nodes.get(at as usize),
            resolution.field_cells().get(at as usize),
        ) else {
            return Ok(RootField::Other);
        };
        if node.first != position {
            return Ok(RootField::Extends);
        }
        // A root shared with other files of a workspace: the file's own
        // record of it.
        let slot = match resolution.own_records().get(at as usize) {
            Some(&own) if own != Slot::NONE => own,
            _ => slot,
        };
        let kind = match (leaf == at, node.leaf) {
            (true, Some(leaf)) => self.kind(leaf.value)?,
            _ => ValueKind::Record,
        };
        // Shown at the root's name in its first item, as a field's load is.
        let span = root
            .get(node.first)
            .and_then(|&first| match self.ast.item(first).kind {
                ItemKind::Field { path, .. } => self.ast.path(path).first().map(|s| s.span),
                _ => None,
            })
            .unwrap_or_else(|| self.node_span(node, root));
        Ok(RootField::First(node.name, (slot, kind, span)))
    }

    /// Adds the field `name` to the record being built by
    /// [`Lowerer::make_record`], to be loaded as `load` says: its slot, the
    /// kind of its value and the span of its definition. If `redefinable`,
    /// the block has had a spread, and a field of the name already there is
    /// overridden instead, keeping its place.
    fn define_field(&mut self, name: Name, load: (Slot, ValueKind, Span), redefinable: bool) {
        if redefinable && let Some(at) = self.layout_names.iter().position(|&known| known == name) {
            if let Some(entry) = self.record_loads.get_mut(at) {
                *entry = load;
            }
            return;
        }
        self.layout_names.push(name);
        self.record_loads.push(load);
    }

    /// The layout for the names in `layout_names`, made if this is the
    /// first record block with those names in that order.
    fn layout(&mut self) -> LayoutId {
        let names = std::mem::take(&mut self.layout_names);
        let id = self.layout_of(&names);
        self.layout_names = names;
        id
    }

    /// The layout for `names`, made if this is the first record with those
    /// names in that order.
    fn layout_of(&mut self, names: &[Name]) -> LayoutId {
        if let Some(&id) = self.layouts.get(names) {
            return id;
        }
        let names: Box<[Name]> = names.into();
        let texts = names
            .iter()
            .map(|&name| Box::from(self.ast.names().get(name)))
            .collect();
        let id = self.builder.push_layout(Layout {
            names: names.clone(),
            texts,
        });
        self.layouts.insert(names, id);
        id
    }

    /// Lowers expression `id`, wrapped in an `Await` if the checker marked
    /// it to be awaited where it is used. An operation of kind `Never`
    /// because what it asks of an operand that always runs is `Never`,
    /// which nothing awaits, is lowered by [`Lowerer::lower_unreached`].
    fn lower_expr(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        match self.resolution.await_of(id) {
            AwaitMark::None => {
                let kind = self.kind(id)?;
                if kind == ValueKind::Never
                    && let Some(unreached) = self.lower_unreached(id)?
                {
                    return Ok(self.builder.alloc(unreached, self.ast.get(id).span));
                }
                self.lower_plain(id, kind)
            }
            mark => self.lower_awaited(id, mark, self.ast.get(id).span),
        }
    }

    /// Lowers expression `id`, which the checker marked to be awaited as
    /// `mark` says, to an `Await` spanning `span` of the node it lowers to
    /// otherwise: at the top, or deeply through the lazy shape of its type.
    fn lower_awaited(
        &mut self,
        id: ExprId,
        mark: AwaitMark,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let plain = self.plain_kind(id)?;
        let value = self.lower_plain(id, plain)?;
        let kind = self.kind(id)?;
        let shape = match mark {
            AwaitMark::Deep => self.lazy_shape(self.used_type(id)),
            AwaitMark::Top | AwaitMark::None => LazyShapeId::NONE,
        };
        Ok(self.builder.alloc(
            IrKind::Await {
                value,
                kind,
                shape,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        ))
    }

    /// Lowers expression `id`, of kind `kind`, as it is, without any await
    /// the checker marked it for.
    fn lower_plain(&mut self, id: ExprId, kind: ValueKind) -> Result<IrId, InternalError> {
        let expr = *self.ast.get(id);
        let span = expr.span;
        let lowered = match (expr.kind, kind) {
            (ExprKind::Paren(inner) | ExprKind::Ascribe { expr: inner, .. }, _) => {
                return self.lower_operand(inner, kind);
            }
            (
                ExprKind::Unary {
                    op: UnaryOp::Await,
                    operand,
                },
                _,
            ) => {
                // An await of a present value does nothing; any other is
                // the operand's own await, spanning the whole expression.
                return match self.resolution.await_of(operand) {
                    AwaitMark::None => self.lower_operand(operand, kind),
                    mark => self.lower_awaited(operand, mark, span),
                };
            }
            (ExprKind::Int(value), ValueKind::Int) => IrKind::IntConst(value),
            (ExprKind::Int(value), ValueKind::Float) => IrKind::FloatConst(int_to_float(value)),
            (ExprKind::Float(value), ValueKind::Float) => IrKind::FloatConst(value),
            (ExprKind::Bool(value), ValueKind::Bool) => IrKind::BoolConst(value),
            (ExprKind::Str(text), ValueKind::String) => {
                IrKind::StrConst(self.builder.push_string(self.ast.strings().get(text)))
            }
            (ExprKind::Interp(parts), ValueKind::String) => {
                self.lower_interpolation(parts, span)?
            }
            (
                ExprKind::If {
                    cond, then, else_, ..
                },
                _,
            ) => IrKind::If {
                cond: self.lower_operand(cond, ValueKind::Bool)?,
                then: self.lower_operand(then, kind)?,
                else_: self.lower_operand(else_, kind)?,
            },
            (ExprKind::Unary { op, operand }, _) => self.lower_unary(op, operand, kind, span)?,
            (
                ExprKind::Binary {
                    op,
                    op_start,
                    lhs,
                    rhs,
                },
                _,
            ) => self.lower_binary(op, op_start, (lhs, rhs), kind, id)?,
            (
                ExprKind::Name(_) | ExprKind::Fn(_) | ExprKind::Call { .. } | ExprKind::Lazy(_),
                _,
            ) => self.lower_use_or_function(id, kind)?,
            (ExprKind::Block { items, label }, _) => self.lower_block(id, (items, label), kind)?,
            // A reference to a node of the tree of the input's top-level
            // fields reads the node's cell (see [Cells](crate::ir#cells)).
            (ExprKind::Field { expr: record, name }, _) => {
                self.lower_field_access(id, (record, name), kind)?
            }
            (ExprKind::Index { expr: list, index }, _) => IrKind::Index {
                list: self.lower_operand(list, ValueKind::List)?,
                index: self.lower_operand(index, ValueKind::Int)?,
                kind,
            },
            (ExprKind::Is { .. } | ExprKind::As { .. }, _) => self.lower_type_test(id, kind)?,
            (ExprKind::List(_) | ExprKind::For(_), _) => {
                self.lower_listing(expr.kind, kind, span)?
            }
            (ExprKind::DefTypeRef(def), _) if self.expr_types.reference_past(id).is_some() => {
                return self.lower_reference_past(def, kind, span);
            }
            (ExprKind::DefTypeRef(def), _) => self.lower_def_type_ref(id, def, kind)?,
            (ExprKind::Error | ExprKind::NoField(_) | ExprKind::Unclosed { .. }, _) => {
                return Err(unparsed(span));
            }
            (
                ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Bool(_)
                | ExprKind::Str(_)
                | ExprKind::Interp(_),
                _,
            ) => return Err(mistyped(expr.kind, kind, span)),
        };
        Ok(self.builder.alloc(lowered, span))
    }

    /// Lowers the field access `id`, `record.name`, of kind `kind`: a read
    /// of a cell if the checker resolved it to a node of the tree of the
    /// input's top-level fields (see [Cells](crate::ir#cells)), and an
    /// ordinary `Field` of the lowered record otherwise.
    fn lower_field_access(
        &mut self,
        id: ExprId,
        (record, name): (ExprId, Name),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        if let VarRef::Fileroot(member) = self.resolution.expr_var(id) {
            return self.lower_fileroot(member, kind, self.ast.get(id).span);
        }
        if let Some(meta) = self.resolution.meta_read(id) {
            return self.lower_meta_read(id, (record, name), meta, kind);
        }
        Ok(match read(self.resolution.expr_var(id), kind) {
            Some(cell) => cell,
            None => IrKind::Field {
                record: self.lower_operand(record, ValueKind::Record)?,
                name,
                kind,
            },
        })
    }

    /// Lowers the field access `id`, `func.name`, of kind `kind`, which the
    /// checker found reads the meta of `func`, of type `meta_ty`, awaited as
    /// `mark` says (see the checker's "Metas"): to a `Field` of the call of
    /// no arguments of the `Meta` of `func`, which yields its meta's closure,
    /// awaited as marked, so that reading a meta is calling its closure, the
    /// caller, the memoisation of a `META->>` meta and the nesting budget
    /// all as for any call. Apart from `lower_plain`, which is on the
    /// recursion path of every expression.
    fn lower_meta_read(
        &mut self,
        id: ExprId,
        (func, name): (ExprId, Name),
        (meta_ty, mark): (TyId, AwaitMark),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        let mistyped = InternalError {
            message: "a meta typed as something no value has",
            span,
        };
        let function = self.lower_operand(func, ValueKind::Fn)?;
        let meta = self.builder.alloc(IrKind::Meta(function), span);
        let args = self.builder.push_item_list(&[]);
        let call = self.builder.alloc(
            IrKind::Call {
                callee: meta,
                args,
                kind: self.table.value_kind(meta_ty).ok_or(mistyped)?,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        );
        let record = match mark {
            AwaitMark::None => call,
            AwaitMark::Top | AwaitMark::Deep => {
                let present = self.table.strip(meta_ty);
                self.builder.alloc(
                    IrKind::Await {
                        value: call,
                        kind: self.table.value_kind(present).ok_or(mistyped)?,
                        shape: LazyShapeId::NONE,
                        // Set by `IrBuilder::measure` once every node is built.
                        depth: 0,
                    },
                    span,
                )
            }
        };
        Ok(IrKind::Field { record, name, kind })
    }

    /// Lowers the read of `member` of the `fileroot` module spanning `span`,
    /// of kind `kind`, a `String`'s (see the checker's "Imports"):
    /// `fileroot.this` and `fileroot.workspace` to a `StrConst` of the
    /// file's place, which the checker resolved, constants of the file and
    /// of the run; `fileroot.caller` to a `Caller`, read where it runs. Any
    /// other is an internal error: `helper` is only ever called, and lowered
    /// as its call ([`Lowerer::lower_use_or_function`]).
    fn lower_fileroot(
        &mut self,
        member: Fileroot,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let place = self.resolution.place();
        match (member, kind) {
            (Fileroot::This, ValueKind::String) => {
                Ok(IrKind::StrConst(self.builder.push_string(&place.this)))
            }
            (Fileroot::Workspace, ValueKind::String) => {
                Ok(IrKind::StrConst(self.builder.push_string(&place.workspace)))
            }
            (Fileroot::Caller, ValueKind::String) => {
                self.read_caller();
                Ok(IrKind::Caller)
            }
            (Fileroot::Helper, _) => Err(InternalError {
                message: "`fileroot.helper` read as a value",
                span,
            }),
            _ => Err(InternalError {
                message: "a member of `fileroot` typed as something other than a String",
                span,
            }),
        }
    }

    /// Lowers the block `id`, whose items are `items` and whose label, if
    /// it has one, is `label`, of kind `kind`, to a `Block`; or, if it is a
    /// labelled block that some `return` leaves, to a `Catch` of kind
    /// `kind` of the `Block`, which is of its result's own kind, its items
    /// lowered with the catch on [`Lowerer::catches`]. Apart from
    /// `lower_plain`, which is on the recursion path of every expression.
    fn lower_block(
        &mut self,
        id: ExprId,
        (items, label): (ItemRange, Option<LabelId>),
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        if label.is_none() || !self.resolution.is_returned_block(id) {
            return self.lower_body((Some(id), items), span, kind);
        }
        let catch = self.start_catch(JumpTarget::Block(id));
        let own = self.body_kind(items)?;
        let block = self.lower_body((Some(id), items), span, own)?;
        Ok(self.end_catch(catch, (block, span), kind))
    }

    /// Lowers the list literal or `for`, as `expr` is, of kind `kind`,
    /// spanning `span`: an internal error unless `kind` is a list's, but
    /// for a loop that cannot end, or that a `return` leaves, which may be
    /// of any kind (see [`Lowerer::lower_for`]). Apart from `lower_plain`,
    /// which is on the recursion path of every expression.
    fn lower_listing(
        &mut self,
        expr: ExprKind,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        match (expr, kind) {
            (ExprKind::List(elements), ValueKind::List) => self.lower_list(elements),
            (ExprKind::For(for_id), ValueKind::List) => self.lower_for(for_id, (kind, true), span),
            (ExprKind::For(for_id), _)
                if self.resolution.loop_of(for_id).endless
                    || self.resolution.loop_of(for_id).returned =>
            {
                self.lower_for(for_id, (kind, true), span)
            }
            _ => Err(InternalError {
                message: "a list literal or a `for` typed as something other than a list",
                span,
            }),
        }
    }

    /// Lowers the expression `id`, of type `Never` because what it asks of
    /// an operand that always runs first is `Never` (see [`crate::check`]),
    /// to a `Block` whose items are its operands that run before that one,
    /// in order, each as it is used, for their effects, and whose result is
    /// that operand, which never completes; the operands after it never run,
    /// and are lowered only for the functions in them, their nodes left
    /// out. A loop whose iterable or first condition is `Never` is lowered
    /// so too, by [`Lowerer::lower_unreached_loop`]. `None` for any other
    /// expression, and for one of type `Never` only because of the value it
    /// reads, a field of a record type's that is `Never`, say, which no
    /// value has: it is lowered as it is otherwise.
    fn lower_unreached(&mut self, id: ExprId) -> Result<Option<IrKind>, InternalError> {
        let expr = *self.ast.get(id);
        let children: Vec<ExprId> = match expr.kind {
            ExprKind::Unary { op, operand } if op != UnaryOp::Await => vec![operand],
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Index {
                expr: lhs,
                index: rhs,
            } => vec![lhs, rhs],
            ExprKind::Field { expr, .. }
            | ExprKind::Is { expr, .. }
            | ExprKind::As { expr, .. } => {
                vec![expr]
            }
            ExprKind::Call { callee, args } => {
                let values = self.ast.args(args).iter().map(|arg| arg.value);
                match self.resolution.expr_var(callee) {
                    VarRef::Builtin(_) | VarRef::Fileroot(_) => values.collect(),
                    _ => std::iter::once(callee).chain(values).collect(),
                }
            }
            ExprKind::Interp(parts) => self
                .ast
                .parts(parts)
                .iter()
                .filter_map(|&part| match part {
                    StrPart::Expr(expr) => Some(expr),
                    StrPart::Lit(_) => None,
                })
                .collect(),
            ExprKind::If {
                cond, then, else_, ..
            } if self.kind(cond)? == ValueKind::Never => vec![cond, then, else_],
            ExprKind::For(for_id) => return self.lower_unreached_loop(for_id),
            _ => return Ok(None),
        };
        let mut first = None;
        for (index, &child) in children.iter().enumerate() {
            if self.kind(child)? == ValueKind::Never {
                first = Some(index);
                break;
            }
        }
        let Some(first) = first else {
            return Ok(None);
        };
        let base = self.item_stack.len();
        let mut result = None;
        for (index, &child) in children.iter().enumerate() {
            let node = self.lower_expr(child)?;
            match index.cmp(&first) {
                std::cmp::Ordering::Less => self.item_stack.push(node),
                std::cmp::Ordering::Equal => result = Some(node),
                // Never run.
                std::cmp::Ordering::Greater => {}
            }
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(result.map(|result| IrKind::Block { items, result }))
    }

    /// Lowers the loop `for_id`, of type `Never` because its iterable, or
    /// its condition, which runs before the first iteration, is: to a
    /// `Block` of the `Store`s of the items before a condition, which run
    /// first, whose result is the iterable or the condition. Its guard, its
    /// items after the condition and its body never run, and are lowered
    /// only for the functions in them. `None` for any other loop.
    fn lower_unreached_loop(&mut self, for_id: ForId) -> Result<Option<IrKind>, InternalError> {
        let decl = *self.ast.for_decl(for_id);
        let (before, first, after, guard) = match decl.head {
            ForHead::Each {
                iterable, guard, ..
            } if self.kind(iterable)? == ValueKind::Never => (None, iterable, None, guard),
            ForHead::While {
                before,
                cond: Some(cond),
                after,
            } if self.kind(cond)? == ValueKind::Never => (Some(before), cond, Some(after), None),
            _ => return Ok(None),
        };
        let ast = self.ast;
        let base = self.item_stack.len();
        for &item in before.map_or(&[][..], |before| ast.items(before)) {
            let store = self.lower_store(item)?;
            self.item_stack.push(store);
        }
        let result = self.lower_expr(first)?;
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        // Never run.
        if let Some(guard) = guard {
            self.lower_expr(guard)?;
        }
        for &item in after.map_or(&[][..], |after| ast.items(after)) {
            self.lower_store(item)?;
        }
        // The `return`s for the loop need its catch, though it is never
        // made.
        let returned = self.resolution.loop_of(for_id).returned;
        if returned {
            self.start_catch(JumpTarget::Loop(for_id));
        }
        self.lower_loop_body(for_id, decl.body)?;
        if returned {
            self.catches.pop();
        }
        Ok(Some(IrKind::Block { items, result }))
    }

    /// Lowers the `is` or `as` expression `id`, of kind `kind`: an `Is` of
    /// its operand, a `Bool`, writing the value it tests to the slot the
    /// checker gave it if it narrows an `if`'s branches, or an `As` of its
    /// operand, of the kind of its narrowed type; each with the test the
    /// checker planned (see [`Lowerer::lower_test`]). Apart from
    /// `lower_plain`, which is on the recursion path of every expression.
    fn lower_type_test(&mut self, id: ExprId, kind: ValueKind) -> Result<IrKind, InternalError> {
        let expr = *self.ast.get(id);
        match (expr.kind, kind) {
            (ExprKind::Is { expr: operand, .. }, ValueKind::Bool) => {
                let (value, test) = self.lower_test(id, operand)?;
                let slot = self
                    .resolution
                    .test_of(id)
                    .map_or(Slot::NONE, |resolution| resolution.slot);
                Ok(IrKind::Is { value, test, slot })
            }
            (ExprKind::As { expr: operand, .. }, _) => {
                let (value, test) = self.lower_test(id, operand)?;
                Ok(IrKind::As { value, test, kind })
            }
            _ => Err(InternalError {
                message: "an `is` typed as something other than a Bool",
                span: expr.span,
            }),
        }
    }

    /// Lowers `id`, of kind `kind`, which is a variable use, a function
    /// expression, a call or a `lazy`. Apart from `lower_plain`, which is on
    /// the recursion path of every expression, so that its frame does not
    /// hold the temporaries of these cases.
    fn lower_use_or_function(
        &mut self,
        id: ExprId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let expr = *self.ast.get(id);
        match (expr.kind, kind) {
            (ExprKind::Name(_), _) => {
                read(self.resolution.expr_var(id), kind).ok_or(InternalError {
                    message: "a variable use has no slot",
                    span: expr.span,
                })
            }
            (ExprKind::Fn(function), ValueKind::Fn) => self.lower_fn(id, function),
            (ExprKind::Lazy(function), ValueKind::Deferred) => self.lower_lazy(id, function),
            (ExprKind::Lazy(_), _) => Err(InternalError {
                message: "a `lazy` typed as something other than a future",
                span: expr.span,
            }),
            (ExprKind::Call { callee, args }, _) => match self.resolution.expr_var(callee) {
                VarRef::Builtin(builtin) => self.lower_builtin(builtin, args, kind, expr.span),
                // `fileroot.helper()`, which takes no argument, is its own
                // node, never reading its callee; the calls after it pass on
                // the caller the body runs with.
                VarRef::Fileroot(Fileroot::Helper) if kind == ValueKind::Record => {
                    self.read_caller();
                    Ok(IrKind::Helper)
                }
                VarRef::Fileroot(_) => Err(InternalError {
                    message: "a call of a member of `fileroot` other than `helper`, or of a kind \
                              other than a record's",
                    span: expr.span,
                }),
                _ => Ok(IrKind::Call {
                    callee: self.lower_operand(callee, ValueKind::Fn)?,
                    args: self.lower_args(id, args, expr.span)?,
                    kind,
                    // Set by `IrBuilder::measure` once every node is built.
                    depth: 0,
                }),
            },
            (ExprKind::Fn(_), _) => Err(InternalError {
                message: "a function expression typed as a non-function type",
                span: expr.span,
            }),
            _ => Err(InternalError {
                message: "an expression lowered as a variable, function or call is none",
                span: expr.span,
            }),
        }
    }

    /// Lowers the function expression `id`, whose details are `function`:
    /// its body into the function table, as code of [`FnKind::Memo`] if it
    /// is declared with `->>` and of [`FnKind::Function`] otherwise, and the
    /// expression to the `MakeClosure` that captures, in the current frame,
    /// what the checker found the function captures.
    fn lower_fn(&mut self, id: ExprId, function: FnId) -> Result<IrKind, InternalError> {
        let kind = self.fn_kind(id, function);
        // A function's caller is its callers' file, whatever the frame that
        // makes it: what its body reads of the caller is no `lazy`'s.
        self.reads_caller.push(false);
        let captures = self.lower_function(id, function, kind);
        self.reads_caller.pop();
        Ok(IrKind::MakeClosure {
            function,
            captures: captures?,
        })
    }

    /// Lowers a def-type reference of `def`, spanning `span`, whose path goes
    /// on past its definition, `package dev.peios.testpkg.versions` (see the
    /// checker's "Metas"), of kind `kind`: to its read, whose accesses past
    /// the definition the checker typed as reads of the meta of the
    /// definition's cell and fields of them, so that it makes no closure.
    fn lower_reference_past(
        &mut self,
        def: DefTypeId,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrId, InternalError> {
        let read = self.ast.def_type(def).read.ok_or(InternalError {
            message: "a def-type reference without a definition",
            span,
        })?;
        self.lower_operand(read, kind)
    }

    /// Lowers the def-type reference `id`, `DT PATH`, of `def` (see the
    /// checker's "Def-types"), to the `MakeClosure` of a function of its
    /// own, made here, past the tree's ([`FnId::lowered`]), which captures
    /// the def-type and the definition, read where the reference is, and
    /// whose body, run with its parameters, those of the def-type after its
    /// second, sets its helper flag, so that the def-type is called with the
    /// caller the closure is called with, and calls the def-type with the
    /// definition's path, as written, the definition and its own
    /// parameters, in order: so a closure is made each time the reference
    /// is evaluated. If the reference carries its definition's meta, the
    /// function's meta is recorded as its capture of the definition's
    /// ([`MetaAt::Capture`]). It is of kind `kind`, which can only be a
    /// function's.
    fn lower_def_type_ref(
        &mut self,
        id: ExprId,
        def: DefTypeId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        let def = *self.ast.def_type(def);
        let mistyped = InternalError {
            message: "a def-type reference without a definition or a function type",
            span,
        };
        if kind != ValueKind::Fn {
            return Err(mistyped);
        }
        let read = def.read.ok_or(mistyped)?;
        let sig = self
            .table
            .fn_sig(self.expr_types.get(id))
            .cloned()
            .ok_or(mistyped)?;
        let def_type = self.lower_expr(def.dt)?;
        let definition = self.lower_expr(read)?;
        let base = self.item_stack.len();
        self.item_stack.push(def_type);
        self.item_stack.push(definition);
        let captures = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        // The body: the helper flag set, then the call of the def-type.
        let path = self
            .ast
            .show_definition_path(self.ast.path(def.path))
            .to_string();
        let path = self.builder.push_string(&path);
        let callee = self
            .builder
            .alloc(IrKind::LoadCapture(0, ValueKind::Fn), span);
        let name = self.builder.alloc(IrKind::StrConst(path), span);
        let definition = self.builder.alloc(
            IrKind::LoadCapture(REFERENCE_DEFINITION, ValueKind::Fn),
            span,
        );
        let base = self.item_stack.len();
        for (param, value) in [(0, name), (1, definition)] {
            let arg = self.builder.alloc(IrKind::Arg { param, value }, span);
            self.item_stack.push(arg);
        }
        for (at, &(_, ty)) in (0u32..).zip(sig.params.iter()) {
            let kind = self.table.value_kind(ty).ok_or(mistyped)?;
            let value = self.builder.alloc(IrKind::Load(Slot::at(at), kind), span);
            let arg = self.builder.alloc(
                IrKind::Arg {
                    param: at.saturating_add(2),
                    value,
                },
                span,
            );
            self.item_stack.push(arg);
        }
        let args = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let kind = self.table.value_kind(sig.ret).ok_or(mistyped)?;
        let call = self.builder.alloc(
            IrKind::Call {
                callee,
                args,
                kind,
                // Set by `IrBuilder::measure` once every node is built.
                depth: 0,
            },
            span,
        );
        let helper = self.builder.alloc(IrKind::Helper, span);
        let items = self.builder.push_item_list(&[helper]);
        let body = self.builder.alloc(
            IrKind::Block {
                items,
                result: call,
            },
            span,
        );
        let params = u32::try_from(sig.params.len()).unwrap_or(u32::MAX);
        let function = FnId::lowered(u32::try_from(self.functions.len()).unwrap_or(u32::MAX));
        // A reference carries its definition's meta: its closure's is the
        // definition's, its second capture.
        if sig.meta.is_some() {
            self.meta_captures.push(function);
        }
        self.functions.push(Some(FnCode {
            body,
            kind: FnKind::Function,
            frame_size: params,
            param_count: params,
            capture_count: 2,
            // Set by `IrBuilder::measure` once every node is built.
            height: 0,
            name: None,
            display: self
                .table
                .display(self.expr_types.get(id), self.ast.names())
                .to_string()
                .into_boxed_str(),
            slot_names: sig
                .params
                .iter()
                .map(|&(name, _)| name)
                .collect::<Option<Box<[Name]>>>()
                .unwrap_or_default(),
        }));
        Ok(IrKind::MakeClosure { function, captures })
    }

    /// The kind of the code of the function expression `id`, of function
    /// `function`: [`FnKind::Memo`] if it is declared with `->>`, and
    /// [`FnKind::Function`] otherwise. A definition made with a def-type
    /// declares nothing: its function type, its def-type's to give, says
    /// (see the checker's "Def-types").
    fn fn_kind(&self, id: ExprId, function: FnId) -> FnKind {
        let effect = match self.ast.get(id).kind {
            ExprKind::Fn(_) if self.ast.fn_decl(function).ret.is_none() => self
                .table
                .fn_sig(self.expr_types.get(id))
                .map_or(Effect::Impure, |sig| sig.effect),
            _ => self.ast.fn_decl(function).effect,
        };
        match effect {
            Effect::Memo => FnKind::Memo,
            Effect::Pure | Effect::Impure => FnKind::Function,
        }
    }

    /// Lowers the `lazy` `id`, whose body is function `function`: the body
    /// into the function table, and the expression to the `MakeFuture` that
    /// captures, in the current frame, what the checker found it captures,
    /// marked if the body can read the caller it captures (see
    /// [`IrKind::MakeFuture`]), which marks the body around it too.
    fn lower_lazy(&mut self, id: ExprId, function: FnId) -> Result<IrKind, InternalError> {
        self.reads_caller.push(false);
        let captures = self.lower_function(id, function, FnKind::Future);
        let reads_caller = self.reads_caller.pop().unwrap_or(false);
        if reads_caller {
            self.read_caller();
        }
        Ok(IrKind::MakeFuture {
            function,
            captures: captures?,
            reads_caller,
        })
    }

    /// Notes that the function or `lazy` body being lowered can read the
    /// caller it runs with (see [`Lowerer::reads_caller`]'s field).
    fn read_caller(&mut self) {
        if let Some(reads) = self.reads_caller.last_mut() {
            *reads = true;
        }
    }

    /// Lowers the body of function `function`, of kind `kind`, the function
    /// expression or `lazy` `id`, into the function table, and returns the
    /// list of the loads, in the current frame, of what it captures. On the
    /// recursion path of nested functions and lazies, so the work after the
    /// body is in a helper whose temporaries this frame does not hold.
    fn lower_function(
        &mut self,
        id: ExprId,
        function: FnId,
        kind: FnKind,
    ) -> Result<IrRange, InternalError> {
        // A function's body starts with no loop or labelled block around
        // it: no `break` leaves it, and no labelled `return`.
        let outer = (
            std::mem::take(&mut self.loops),
            std::mem::take(&mut self.catches),
        );
        let body = self.lower_expr(self.ast.fn_decl(function).body);
        (self.loops, self.catches) = outer;
        self.enter_function(id, function, (kind, body?))
    }

    /// Enters function `function`, of kind `kind`, the function expression
    /// or `lazy` `id`, whose body lowered to `body`, into the function
    /// table, and returns the list of the loads, in the current frame, of
    /// what it captures.
    fn enter_function(
        &mut self,
        id: ExprId,
        function: FnId,
        (kind, body): (FnKind, IrId),
    ) -> Result<IrRange, InternalError> {
        let span = self.ast.get(id).span;
        let resolution = self.resolution.function(function);
        let display = self
            .table
            .display(self.expr_types.get(id), self.ast.names())
            .to_string()
            .into_boxed_str();
        let code = FnCode {
            body,
            kind,
            frame_size: resolution.frame_size,
            param_count: resolution.param_count,
            capture_count: u32::try_from(resolution.captures.len()).unwrap_or(u32::MAX),
            // Set by `IrBuilder::measure` once every node is built.
            height: 0,
            name: resolution.callee,
            display,
            slot_names: resolution.slot_names.clone().into_boxed_slice(),
        };
        let Some(entry) = self.functions.get_mut(function.index()) else {
            return Err(InternalError {
                message: "a function expression has no entry in the function table",
                span,
            });
        };
        *entry = Some(code);
        let base = self.item_stack.len();
        for &(var, ty) in &resolution.captures {
            let kind = self.table.value_kind(ty).ok_or(InternalError {
                message: "a captured value has a type with no values",
                span,
            })?;
            let read = read(var, kind).ok_or(InternalError {
                message: "a captured value is not a variable",
                span,
            })?;
            let load = self.builder.alloc(read, span);
            self.item_stack.push(load);
        }
        let captures = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(captures)
    }

    /// TEMPORARY (alias rule): lowers the call spanning `span` of the
    /// built-in function `builtin`, whose typing is special, with the
    /// arguments `args`, of kind `kind`, to the built-in's own node: `len` to
    /// a `ListLen` of its list, and `show` to a `Show` of its value, of any
    /// kind, awaited as the checker marked it. (With polymorphic types for
    /// them, R11 of `docs/INFERENCE_REQS.md`, a call of one is an ordinary
    /// call, or this stays as a specialisation of a known callee.)
    fn lower_builtin(
        &mut self,
        builtin: Builtin,
        args: ArgRange,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        match builtin {
            Builtin::Len if kind == ValueKind::Int => {
                let list = self.builtin_argument(args, 0, span)?;
                Ok(IrKind::ListLen(self.lower_operand(list, ValueKind::List)?))
            }
            Builtin::Len => Err(InternalError {
                message: "a call of `len` typed as something other than an Int",
                span,
            }),
            Builtin::Show if kind == ValueKind::String => {
                let value = self.builtin_argument(args, 0, span)?;
                Ok(IrKind::Show(self.lower_expr(value)?))
            }
            Builtin::Show => Err(InternalError {
                message: "a call of `show` typed as something other than a String",
                span,
            }),
            // An ordinary built-in is called as any function value is.
            Builtin::Int
            | Builtin::Float
            | Builtin::Exec
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsTempDir
            | Builtin::FsWrite
            | Builtin::SortVersions
            | Builtin::ExecLive => Err(InternalError {
                message: "a call of a built-in function of a function type resolved as special",
                span,
            }),
        }
    }

    /// The argument, among `args`, of the call spanning `span` of a
    /// built-in function, that the checker bound to its parameter number
    /// `param`.
    fn builtin_argument(
        &self,
        args: ArgRange,
        param: u32,
        span: Span,
    ) -> Result<ExprId, InternalError> {
        self.ast
            .args(args)
            .iter()
            .zip(self.resolution.arg_params(args))
            .find(|&(_, &bound)| bound == param)
            .map(|(arg, _)| arg.value)
            .ok_or(InternalError {
                message: "a parameter of a built-in function is bound to no argument",
                span,
            })
    }

    /// Lowers the elements `elements` of a list literal to the items of a
    /// `MakeList`, in source order: each element's expression, as it is
    /// used, and a `Spread` of each spread's list.
    fn lower_list(&mut self, elements: ElementRange) -> Result<IrKind, InternalError> {
        let ast = self.ast;
        let base = self.item_stack.len();
        let mut spreads = false;
        for element in ast.elements(elements) {
            let item = if element.spread.is_some() {
                spreads = true;
                let list = self.lower_operand(element.value, ValueKind::List)?;
                self.builder.alloc(IrKind::Spread(list), element.span(ast))
            } else {
                self.lower_expr(element.value)?
            };
            self.item_stack.push(item);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::MakeList { items, spreads })
    }

    /// Lowers the loop `for_id`, spanning `span`, which collects what its
    /// iterations contribute into a list if `collect`, and discards it
    /// otherwise. An iteration lowers to a `For` on its variable's slot,
    /// whose parts are its iterable, a list, as it is used, its guard, if
    /// it has one, a `Bool`, and its body, as it is used, which may be
    /// awaited to fit the element type expected of the loop. A condition
    /// loop lowers to a `Loop` whose parts are its condition, a `Bool`, or
    /// `true` for `for { ... }`, its body and the `Store`s of the items
    /// after its condition, in a `Block` whose items are the `Store`s of
    /// those before it, if it has any; a `do` loop to a `Loop` of its
    /// condition and its body that does not test first. Each body is
    /// lowered with its loop on [`Lowerer::loops`].
    ///
    /// A loop that cannot end (see [`crate::check::LoopResolution`]) never
    /// collects, whatever `collect` says: nothing could read its list. A
    /// loop that a `return` leaves is wrapped in a `Catch` of kind `kind`,
    /// the whole loop's, its body lowered with the catch on
    /// [`Lowerer::catches`].
    fn lower_for(
        &mut self,
        for_id: ForId,
        (kind, collect): (ValueKind, bool),
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let found = self.resolution.loop_of(for_id);
        let collect = collect && !found.endless;
        if !found.returned {
            return self.lower_loop_node(for_id, collect, span);
        }
        let catch = self.start_catch(JumpTarget::Loop(for_id));
        let looped = self.lower_loop_node(for_id, collect, span)?;
        Ok(self.end_catch(catch, (looped, span), kind))
    }

    /// Lowers the loop `for_id`, spanning `span`, as [`Lowerer::lower_for`]
    /// says, without the catch around it.
    fn lower_loop_node(
        &mut self,
        for_id: ForId,
        collect: bool,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let decl = *self.ast.for_decl(for_id);
        let (before, cond, after) = match decl.head {
            ForHead::Each {
                iterable, guard, ..
            } => return self.lower_each(for_id, (iterable, guard, decl.body), collect),
            ForHead::While {
                before,
                cond,
                after,
            } => (Some(before), cond, Some(after)),
            ForHead::Do { cond } => (None, Some(cond), None),
        };
        let ast = self.ast;
        let base = self.item_stack.len();
        for &item in before.map_or(&[][..], |before| ast.items(before)) {
            let store = self.lower_store(item)?;
            self.item_stack.push(store);
        }
        let stores = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let cond = if let Some(cond) = cond {
            self.lower_operand(cond, ValueKind::Bool)?
        } else {
            let keyword = symbol_span("for", span.start);
            self.builder.alloc(IrKind::BoolConst(true), keyword)
        };
        self.item_stack.push(cond);
        let body = self.lower_loop_body(for_id, decl.body)?;
        self.item_stack.push(body);
        for &item in after.map_or(&[][..], |after| ast.items(after)) {
            let store = self.lower_store(item)?;
            self.item_stack.push(store);
        }
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        let looped = IrKind::Loop {
            parts,
            collect,
            test_first: before.is_some(),
        };
        if before.is_none_or(|before| ast.items(before).is_empty()) {
            return Ok(looped);
        }
        let result = self.builder.alloc(looped, span);
        Ok(IrKind::Block {
            items: stores,
            result,
        })
    }

    /// Lowers the iteration `for_id`, whose iterable, guard, if any, and
    /// body are `iterable`, `guard` and `body`, to a `For` (see
    /// [`Lowerer::lower_for`]).
    fn lower_each(
        &mut self,
        for_id: ForId,
        (iterable, guard, body): (ExprId, Option<ExprId>, ExprId),
        collect: bool,
    ) -> Result<IrKind, InternalError> {
        let slot = self.resolution.for_slot(for_id);
        if slot == Slot::NONE {
            return Err(InternalError {
                message: "a `for` has no slot for its variable",
                span: self.ast.get(body).span,
            });
        }
        let base = self.item_stack.len();
        let iterable = self.lower_operand(iterable, ValueKind::List)?;
        self.item_stack.push(iterable);
        if let Some(guard) = guard {
            let guard = self.lower_operand(guard, ValueKind::Bool)?;
            self.item_stack.push(guard);
        }
        let body = self.lower_loop_body(for_id, body)?;
        self.item_stack.push(body);
        let parts = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::For {
            slot,
            parts,
            collect,
        })
    }

    /// Lowers `body`, the body of the loop `for_id`, as it is used, with
    /// the loop on [`Lowerer::loops`], so that the `break`s and `continue`s
    /// in it count the loops they leave.
    fn lower_loop_body(&mut self, for_id: ForId, body: ExprId) -> Result<IrId, InternalError> {
        self.loops.push(for_id);
        let body = self.lower_expr(body);
        self.loops.pop();
        body
    }

    /// The `for` expression that the statement `value` is, through any
    /// parentheses, with its span, if its type is unit-like as it is, so
    /// that nothing awaits it: its list would be dropped as soon as it was
    /// built, so it is not built.
    fn discarded_for(&self, value: ExprId) -> Option<(ForId, Span)> {
        if self.resolution.await_of(value) != AwaitMark::None
            || !self.table.is_unit_like(self.expr_types.get(value))
        {
            return None;
        }
        let mut node = value;
        loop {
            let expr = self.ast.get(node);
            match expr.kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::For(for_id) => return Some((for_id, expr.span)),
                _ => return None,
            }
        }
    }

    /// Lowers the arguments `args` of the call `id` spanning `span` to `Arg`
    /// nodes in source order, each with the parameter the checker bound it
    /// to, followed by one for each parameter the call gives its default
    /// ([`crate::check::Resolution::call_defaults`]), in parameter order,
    /// each the constant's nodes, at the call's closing parenthesis, and
    /// returns their list.
    fn lower_args(
        &mut self,
        id: ExprId,
        args: ArgRange,
        span: Span,
    ) -> Result<IrRange, InternalError> {
        let (ast, resolution) = (self.ast, self.resolution);
        let params = resolution.arg_params(args);
        let base = self.item_stack.len();
        for (arg, &param) in ast.args(args).iter().zip(params) {
            if param == u32::MAX {
                return Err(InternalError {
                    message: "an argument of a call is bound to no parameter",
                    span,
                });
            }
            let value = self.lower_expr(arg.value)?;
            let node = self
                .builder
                .alloc(IrKind::Arg { param, value }, arg.span(ast));
            self.item_stack.push(node);
        }
        let close = ast.get(id).close_span().unwrap_or(span);
        for (param, constant) in resolution.call_defaults(id) {
            let value = self.lower_constant(constant, close);
            let node = self.builder.alloc(
                IrKind::Arg {
                    param: *param,
                    value,
                },
                close,
            );
            self.item_stack.push(node);
        }
        let list = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(list)
    }

    /// Lowers `constant`, a parameter's default or a field's (see the
    /// checker's "Field defaults"), to nodes spanning `span`
    /// that build it: an `IntConst`, `FloatConst`, `BoolConst` or
    /// `StrConst`, a `MakeList` of its elements' nodes, a `MakeRecord` of its
    /// fields', in the order they are written, with the layout of their
    /// names, shared as a record block's is, or `Empty` for `{}`.
    ///
    /// Recurses once per level of list and record nesting.
    fn lower_constant(&mut self, constant: &Constant, span: Span) -> IrId {
        let kind = match constant {
            Constant::Int(value) => IrKind::IntConst(*value),
            Constant::Float(bits) => IrKind::FloatConst(f64::from_bits(*bits)),
            Constant::Bool(value) => IrKind::BoolConst(*value),
            Constant::Str(text) => IrKind::StrConst(self.builder.push_string(text)),
            Constant::Empty => IrKind::Empty,
            Constant::List(elements) => {
                let items: Vec<IrId> = elements
                    .iter()
                    .map(|element| self.lower_constant(element, span))
                    .collect();
                IrKind::MakeList {
                    items: self.builder.push_item_list(&items),
                    spreads: false,
                }
            }
            Constant::Record(fields) => {
                let values: Vec<IrId> = fields
                    .iter()
                    .map(|(_, value)| self.lower_constant(value, span))
                    .collect();
                let names: Vec<Name> = fields.iter().map(|&(name, _)| name).collect();
                IrKind::MakeRecord {
                    layout: self.layout_of(&names),
                    values: self.builder.push_item_list(&values),
                }
            }
        };
        self.builder.alloc(kind, span)
    }

    /// Lowers the prefix operator `op`, of kind `kind` and spanning `span`,
    /// applied to `operand`.
    fn lower_unary(
        &mut self,
        op: UnaryOp,
        operand: ExprId,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        Ok(match (op, kind) {
            (UnaryOp::Neg, ValueKind::Int) => {
                IrKind::IntNeg(self.lower_operand(operand, ValueKind::Int)?)
            }
            (UnaryOp::Neg, ValueKind::Float) => {
                IrKind::FloatNeg(self.lower_operand(operand, ValueKind::Float)?)
            }
            (UnaryOp::Not, ValueKind::Bool) => {
                IrKind::Not(self.lower_operand(operand, ValueKind::Bool)?)
            }
            _ => {
                return Err(InternalError {
                    message: "prefix operator has a result type it cannot have",
                    span,
                });
            }
        })
    }

    /// Lowers the parts of an interpolated string spanning `span` to an
    /// `Interp` whose items are a `StrConst` for each literal piece and the
    /// lowered expressions, in order.
    fn lower_interpolation(
        &mut self,
        parts: PartRange,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let base = self.item_stack.len();
        for &part in self.ast.parts(parts) {
            let item = match part {
                StrPart::Lit(text) => {
                    let text = self.builder.push_string(self.ast.strings().get(text));
                    self.builder.alloc(IrKind::StrConst(text), span)
                }
                StrPart::Expr(expr) => self.lower_expr(expr)?,
            };
            self.item_stack.push(item);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Interp(items))
    }

    /// Lowers the binary operator `op`, whose symbol starts at `op_start`,
    /// of kind `kind`, the expression `id`.
    fn lower_binary(
        &mut self,
        op: BinaryOp,
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        if let Some(cmp) = comparison(op) {
            return self.lower_comparison((op, cmp), op_start, (lhs, rhs), kind, id);
        }
        let (make, operand_kind): (fn(Operands) -> IrKind, ValueKind) = match (op, kind) {
            (BinaryOp::Add, ValueKind::String) => (IrKind::StrConcat, ValueKind::String),
            (BinaryOp::And, ValueKind::Bool) => (IrKind::And, ValueKind::Bool),
            (BinaryOp::Or, ValueKind::Bool) => (IrKind::Or, ValueKind::Bool),
            (BinaryOp::Add, ValueKind::Int) => (IrKind::IntAdd, ValueKind::Int),
            (BinaryOp::Sub, ValueKind::Int) => (IrKind::IntSub, ValueKind::Int),
            (BinaryOp::Mul, ValueKind::Int) => (IrKind::IntMul, ValueKind::Int),
            (BinaryOp::FloorDiv, ValueKind::Int) => (IrKind::IntFloorDiv, ValueKind::Int),
            (BinaryOp::Rem, ValueKind::Int) => (IrKind::IntRem, ValueKind::Int),
            (BinaryOp::Range, ValueKind::List) => {
                (|operands| IrKind::Range(false, operands), ValueKind::Int)
            }
            (BinaryOp::RangeInclusive, ValueKind::List) => {
                (|operands| IrKind::Range(true, operands), ValueKind::Int)
            }
            (BinaryOp::Add, ValueKind::Float) => (IrKind::FloatAdd, ValueKind::Float),
            (BinaryOp::Sub, ValueKind::Float) => (IrKind::FloatSub, ValueKind::Float),
            (BinaryOp::Mul, ValueKind::Float) => (IrKind::FloatMul, ValueKind::Float),
            (BinaryOp::Div, ValueKind::Float) => {
                let lhs = self.lower_as_float(lhs)?;
                let rhs = self.lower_as_float(rhs)?;
                return Ok(IrKind::FloatDiv(Operands { lhs, rhs, op_start }));
            }
            _ => {
                return Err(InternalError {
                    message: "operator has a result type it cannot have",
                    span: self.ast.get(id).span,
                });
            }
        };
        let lhs = self.lower_operand(lhs, operand_kind)?;
        let rhs = self.lower_operand(rhs, operand_kind)?;
        Ok(make(Operands { lhs, rhs, op_start }))
    }

    /// Lowers the comparison `op`, which tests `cmp` on every kind but
    /// records, of kind `kind`, to the node for its operands' kind, which
    /// the checker has made the same on both sides.
    fn lower_comparison(
        &mut self,
        (op, cmp): (BinaryOp, Cmp),
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        if kind != ValueKind::Bool {
            return Err(InternalError {
                message: "operator has a result type it cannot have",
                span,
            });
        }
        let operand_kind = self.kind(lhs)?;
        // Typed equality on records and lists compares over the operands'
        // type, once awaited.
        let shape = match (operand_kind, op) {
            (ValueKind::Record | ValueKind::List, BinaryOp::Eq | BinaryOp::Ne) => {
                Some(self.shape(self.used_type(lhs), span)?)
            }
            _ => None,
        };
        let lhs = self.lower_operand(lhs, operand_kind)?;
        let rhs = self.lower_operand(rhs, operand_kind)?;
        let operands = Operands { lhs, rhs, op_start };
        Ok(match (operand_kind, op, shape) {
            (ValueKind::Int, ..) => IrKind::IntCmp(cmp, operands),
            (ValueKind::Float, ..) => IrKind::FloatCmp(cmp, operands),
            (ValueKind::String, ..) => IrKind::StrCmp(cmp, operands),
            (ValueKind::Bool, BinaryOp::Eq | BinaryOp::Same, _) => IrKind::BoolEq(true, operands),
            (ValueKind::Bool, BinaryOp::Ne | BinaryOp::NotSame, _) => {
                IrKind::BoolEq(false, operands)
            }
            (ValueKind::Record, BinaryOp::Eq, Some(shape)) => {
                IrKind::RecordEq(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::Record, BinaryOp::Ne, Some(shape)) => {
                IrKind::RecordNe(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::Record, BinaryOp::Same, _) => IrKind::RecordSame(operands),
            (ValueKind::Record, BinaryOp::NotSame, _) => IrKind::RecordNotSame(operands),
            (ValueKind::List, BinaryOp::Eq, Some(shape)) => {
                IrKind::ListEq(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::List, BinaryOp::Ne, Some(shape)) => {
                IrKind::ListNe(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::List, BinaryOp::Same, _) => IrKind::ListSame(operands),
            (ValueKind::List, BinaryOp::NotSame, _) => IrKind::ListNotSame(operands),
            (ValueKind::Mixed, BinaryOp::Same, _) => IrKind::ValueSame(operands),
            (ValueKind::Mixed, BinaryOp::NotSame, _) => IrKind::ValueNotSame(operands),
            (
                ValueKind::Bool
                | ValueKind::Record
                | ValueKind::List
                | ValueKind::Fn
                | ValueKind::Deferred
                | ValueKind::Mixed
                | ValueKind::Never,
                ..,
            ) => {
                return Err(InternalError {
                    message: "comparison of a type it cannot compare",
                    span,
                });
            }
        })
    }

    /// The shape of the record or list type `ty`, made, with the shapes of
    /// the record and list types nested in it, if this is the first
    /// comparison of it. Each shape is given its id when it is first met
    /// and its fields, or its element's shape, when it is taken off the work
    /// list, so that nesting does not recurse. A field or an element that
    /// may be a future is compared as its value once awaited, which the
    /// deep await of the operands has done, so the shape follows its type
    /// stripped.
    fn shape(&mut self, ty: TyId, span: Span) -> Result<ShapeId, InternalError> {
        if let Some(&id) = self.shapes.get(&ty) {
            return Ok(id);
        }
        let table = self.table;
        let id = self.new_shape(ty);
        while let Some(ty) = self.shape_pending.pop() {
            if let Some(element) = table.list_element(ty) {
                let element = self.nested_shape(table.strip(element));
                if let Some(&shape) = self.shapes.get(&ty) {
                    self.builder.set_list_shape(shape, ListShape { element });
                }
                continue;
            }
            let Some(fields) = table.record_fields(ty) else {
                self.shape_pending.clear();
                return Err(InternalError {
                    message: "a record or list comparison of a type that is neither",
                    span,
                });
            };
            let mut shaped = Vec::with_capacity(fields.len());
            for &(name, field_ty) in fields {
                let nested = self.nested_shape(table.strip(field_ty));
                shaped.push((name, nested));
            }
            let Some(&shape) = self.shapes.get(&ty) else {
                continue;
            };
            self.builder.set_shape(
                shape,
                Shape {
                    fields: shaped.into_boxed_slice(),
                },
            );
        }
        Ok(id)
    }

    /// The shape a record's field or a list's element of type `ty`, stripped,
    /// is compared through: its own if it is a record or a list type, made
    /// and queued if it has none yet, and none otherwise.
    fn nested_shape(&mut self, ty: TyId) -> Option<ShapeId> {
        if !self.is_container(ty) {
            return None;
        }
        Some(match self.shapes.get(&ty) {
            Some(&nested) => nested,
            None => self.new_shape(ty),
        })
    }

    /// Whether `ty` is a record or a list type, which can hold values of
    /// other types.
    fn is_container(&self, ty: TyId) -> bool {
        self.table.is_record(ty) || self.table.list_element(ty).is_some()
    }

    /// Gives the record or list type `ty` a shape with no fields yet, and
    /// queues it to be given its fields or its element's shape.
    fn new_shape(&mut self, ty: TyId) -> ShapeId {
        let id = self.builder.push_shape(Shape {
            fields: Box::default(),
        });
        self.shapes.insert(ty, id);
        self.shape_pending.push(ty);
        id
    }

    /// The lazy shape of `ty`, a type present at its top: for a record type,
    /// each field whose type is a future or an either, with the lazy shape
    /// of its type stripped, and each present record or list field with
    /// lazy values somewhere inside, with its own; for a list type, what is
    /// done with each element, in the same way, if its type is lazy or has
    /// lazy values inside; for a union, the lazy shapes of its members that
    /// have one (see [`Ir::lazy_union`]); [`LazyShapeId::NONE`] if it has no
    /// such field, element or member, or is none of those. Made once per
    /// type, and the shapes of the record, list and union types inside it
    /// first, through a work list rather than recursion, since record and
    /// list types can nest as deeply as a program is long.
    fn lazy_shape(&mut self, ty: TyId) -> LazyShapeId {
        let table = self.table;
        if let Some(&id) = self.lazy_shapes.get(&ty) {
            return id;
        }
        self.lazy_pending.push((ty, false));
        while let Some((container, queued)) = self.lazy_pending.pop() {
            if self.lazy_shapes.contains_key(&container) {
                continue;
            }
            let element = table.list_element(container);
            let fields = table.record_fields(container).unwrap_or_default();
            let members = table.union_members(container).unwrap_or_default();
            if !queued {
                // The record, list and union types inside it first, then it
                // again.
                self.lazy_pending.push((container, true));
                let inside = fields
                    .iter()
                    .map(|&(_, field_ty)| field_ty)
                    .chain(element)
                    .chain(members.iter().copied());
                for inner in inside {
                    let inner = table.strip(inner);
                    if (self.is_container(inner) || table.union_members(inner).is_some())
                        && !self.lazy_shapes.contains_key(&inner)
                    {
                        self.lazy_pending.push((inner, false));
                    }
                }
                continue;
            }
            if !members.is_empty() {
                let shapes: Box<[LazyShapeId]> = members
                    .iter()
                    .filter_map(|member| self.lazy_shapes.get(member).copied())
                    .filter(|&shape| shape != LazyShapeId::NONE)
                    .collect();
                let id = if shapes.is_empty() {
                    LazyShapeId::NONE
                } else {
                    self.builder.push_lazy_union(shapes)
                };
                self.lazy_shapes.insert(container, id);
                continue;
            }
            // What a deep await does with a value of type `ty` inside it.
            let action = |lazy_shapes: &HashMap<TyId, LazyShapeId>, ty: TyId| {
                let inner = lazy_shapes
                    .get(&table.strip(ty))
                    .copied()
                    .unwrap_or(LazyShapeId::NONE);
                if table.laziness(ty) != Laziness::Present {
                    Some(LazyField::Await(inner))
                } else if inner != LazyShapeId::NONE {
                    Some(LazyField::Within(inner))
                } else {
                    None
                }
            };
            let id = if let Some(element) = element {
                match action(&self.lazy_shapes, element) {
                    Some(each) => self.builder.push_lazy_list(each),
                    None => LazyShapeId::NONE,
                }
            } else {
                let lazy: Vec<(Name, LazyField)> = fields
                    .iter()
                    .filter_map(|&(name, field_ty)| {
                        action(&self.lazy_shapes, field_ty).map(|field| (name, field))
                    })
                    .collect();
                if lazy.is_empty() {
                    LazyShapeId::NONE
                } else {
                    self.builder.push_lazy_shape(LazyShape {
                        fields: lazy.into_boxed_slice(),
                    })
                }
            };
            self.lazy_shapes.insert(container, id);
        }
        self.lazy_shapes
            .get(&ty)
            .copied()
            .unwrap_or(LazyShapeId::NONE)
    }

    /// Lowers an operand that must already be of kind `expected`, where it
    /// is used. Where a value of kind `Deferred` is expected, a value of
    /// any kind is accepted, since a present value may be where a future
    /// may; where one of kind `Mixed` is, a union's, a value of any kind but
    /// `Deferred` is, since a value of any member's kind may be where the
    /// union may, but a future may not.
    fn lower_operand(&mut self, id: ExprId, expected: ValueKind) -> Result<IrId, InternalError> {
        let kind = self.kind(id)?;
        // A node of kind `Never` never yields a value, so it may stand where
        // one of any kind is expected: every walker passes on what it does.
        let accepted = match expected {
            _ if kind == ValueKind::Never => true,
            ValueKind::Deferred => true,
            ValueKind::Mixed => kind != ValueKind::Deferred,
            _ => kind == expected,
        };
        if accepted {
            self.lower_expr(id)
        } else {
            Err(InternalError {
                message: "operand type does not match its operator",
                span: self.ast.get(id).span,
            })
        }
    }

    /// Lowers the operand `operand` of the `is` or `as` expression `id`,
    /// awaited where the checker marked it, which must not then be of kind
    /// [`ValueKind::Deferred`], and adds the test the checker planned for
    /// it to the IR, with the text of each field name the test looks at;
    /// returns the operand's node and the test.
    fn lower_test(&mut self, id: ExprId, operand: ExprId) -> Result<(IrId, TestId), InternalError> {
        let span = self.ast.get(id).span;
        let Some(resolution) = self.resolution.test_of(id) else {
            return Err(InternalError {
                message: "an `is` or `as` has no test",
                span,
            });
        };
        if self.kind(operand)? == ValueKind::Deferred {
            return Err(InternalError {
                message: "the operand of an `is` or `as` may be a future",
                span,
            });
        }
        let code = TestCode {
            test: resolution.test.clone(),
            subject: resolution.subject.clone(),
            target: resolution.target.clone(),
        };
        let mut pending = vec![&resolution.test];
        while let Some(test) = pending.pop() {
            match test {
                ValueTest::Pass | ValueTest::Kind(_) => {}
                ValueTest::Record(fields) => {
                    for (name, field) in fields {
                        self.builder
                            .add_name_text(*name, self.ast.names().get(*name));
                        pending.push(field);
                    }
                }
                ValueTest::List(element) => pending.push(element),
                ValueTest::Any(alternatives) => pending.extend(alternatives),
            }
        }
        let value = self.lower_expr(operand)?;
        Ok((value, self.builder.push_test(code)))
    }

    /// Lowers an operand of `/`, converting a concrete `Int` to `Float`.
    fn lower_as_float(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        let span = self.ast.get(id).span;
        match self.used_type(id) {
            TyId::FLOAT => self.lower_expr(id),
            TyId::INT => {
                let operand = self.lower_expr(id)?;
                Ok(self.builder.alloc(IrKind::IntToFloat(operand), span))
            }
            TyId::UNTYPED_INT => Err(InternalError {
                message: "an untyped integer survived type checking",
                span,
            }),
            TyId::ERROR => Err(InternalError {
                message: "a type error survived type checking",
                span,
            }),
            _ => Err(InternalError {
                message: "operand type does not match its operator",
                span,
            }),
        }
    }
}

/// The kind of the record or list that a dotted `let` or `:=` whose path is
/// `segments` updates: a list if its second segment is an index, and a
/// record otherwise.
fn updated_kind(segments: &[PathSegment]) -> ValueKind {
    match segments.get(1).map(|segment| segment.kind) {
        Some(SegmentKind::Index { .. }) => ValueKind::List,
        _ => ValueKind::Record,
    }
}

/// The node reading a value of kind `kind` from `var`, or `None` if it is
/// not a variable, a built-in function being none.
const fn read(var: VarRef, kind: ValueKind) -> Option<IrKind> {
    match var {
        VarRef::Local(slot) if slot.get() != Slot::NONE.get() => Some(IrKind::Load(slot, kind)),
        VarRef::Capture(index) => Some(IrKind::LoadCapture(index, kind)),
        VarRef::Fn(slot) => Some(IrKind::Top(slot, kind)),
        VarRef::Callee => Some(IrKind::LoadCallee),
        VarRef::Local(_) | VarRef::Builtin(_) | VarRef::Fileroot(_) | VarRef::None => None,
    }
}

/// The relation a comparison operator tests on values other than records,
/// where `===` and `!==` are `==` and `!=`, or `None` for any other
/// operator.
const fn comparison(op: BinaryOp) -> Option<Cmp> {
    match op {
        BinaryOp::Eq | BinaryOp::Same => Some(Cmp::Eq),
        BinaryOp::Ne | BinaryOp::NotSame => Some(Cmp::Ne),
        BinaryOp::Lt => Some(Cmp::Lt),
        BinaryOp::Le => Some(Cmp::Le),
        BinaryOp::Gt => Some(Cmp::Gt),
        BinaryOp::Ge => Some(Cmp::Ge),
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::FloorDiv
        | BinaryOp::Rem
        | BinaryOp::And
        | BinaryOp::Or
        | BinaryOp::Range
        | BinaryOp::RangeInclusive => None,
    }
}

/// The internal error of the literal `literal`, at `span`, typed as a value
/// of `kind`, which it cannot be: a float as an `Int`, a number as no
/// number, any other literal as anything but its own type.
const fn mistyped(literal: ExprKind, kind: ValueKind, span: Span) -> InternalError {
    let message = match (literal, kind) {
        (ExprKind::Float(_), ValueKind::Int) => "float literal typed as Int",
        (ExprKind::Int(_) | ExprKind::Float(_), _) => "numeric literal typed as a non-numeric type",
        _ => "literal typed as a type it cannot have",
    };
    InternalError { message, span }
}

/// The internal error of an error node of the parser's at `span` reaching
/// lowering: no tree with a syntax error is lowered.
const fn unparsed(span: Span) -> InternalError {
    InternalError {
        message: "a node the parser could not parse reached lowering",
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AstBuilder, Expr, Item};
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::names::Names;
    use crate::parser::parse;
    use crate::source::SourceFile;
    use crate::strings::Strings;

    fn parse_text(text: &str) -> Ast {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        parse(&tokens, names, strings, &source).unwrap()
    }

    fn lower_text(text: &str) -> Ir {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        lower(&ast, &checked, &types, 0).unwrap()
    }

    fn dump(text: &str) -> String {
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 0).unwrap();
        ir.pretty(ast.names()).to_string()
    }

    /// As [`lower_text`], but checking `text` as a lone file is, beside the
    /// standard library, so that the prelude's names, `len` among them, are
    /// its: what the tests of `len` lower with. Its slots come after the
    /// standard library's.
    fn lower_prelude(text: &str) -> (Ast, Ir) {
        let (ast, types, output) = crate::workspace::check_beside_prelude(text);
        let checked = output.unwrap().checked;
        let ir = lower(&ast, &checked, &types, 0).unwrap();
        (ast, ir)
    }

    /// As [`dump`], with [`lower_prelude`].
    fn dump_prelude(text: &str) -> String {
        let (ast, ir) = lower_prelude(text);
        ir.pretty(ast.names()).to_string()
    }

    /// The node a file of one expression lowers to: its root block's result.
    fn result_of(ir: &Ir) -> IrId {
        let IrKind::Block { items, result } = ir.get(ir.root()).kind else {
            panic!("the root is not a block: {:?}", ir.get(ir.root()));
        };
        assert!(ir.items(items).is_empty());
        result
    }

    /// `7 / 2` built by hand with both literals given the types in `types`,
    /// e.g. both concrete `Int`, the shape variables produce.
    fn concrete_int_division(types: Vec<TyId>) -> (Ast, Checked) {
        let mut builder = AstBuilder::new();
        let lhs = builder.alloc(Expr {
            kind: ExprKind::Int(7),
            span: Span::new(0, 1),
        });
        let rhs = builder.alloc(Expr {
            kind: ExprKind::Int(2),
            span: Span::new(4, 5),
        });
        let div = builder.alloc(Expr {
            kind: ExprKind::Binary {
                op: BinaryOp::Div,
                op_start: 2,
                lhs,
                rhs,
            },
            span: Span::new(0, 5),
        });
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(div),
            span: Span::new(0, 5),
        });
        let root = builder.push_item_list(&[item]);
        let ast = builder.finish(root, Names::new(), Strings::new());
        let checked = Checked {
            types: Types::new(types),
            resolution: Resolution::without_variables(3, 1),
        };
        (ast, checked)
    }

    #[test]
    fn untyped_division_operands_become_float_constants() {
        let ir = lower_text("7 / 2");
        let root = ir.get(result_of(&ir));
        let IrKind::FloatDiv(operands) = root.kind else {
            panic!("expected FloatDiv, got {root:?}");
        };
        assert_eq!(ir.get(operands.lhs).kind, IrKind::FloatConst(7.0));
        assert_eq!(ir.get(operands.rhs).kind, IrKind::FloatConst(2.0));
        assert_eq!(operands.op_start, 2);
        assert_eq!(
            dump("7 / 2"),
            "(block @0..5\n  (float-div @0..5\n    (float 7.0 @0..1)\n    (float 2.0 @4..5)))"
        );
    }

    #[test]
    fn concrete_int_division_operands_are_converted() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::FLOAT]);
        let ir = lower(&ast, &checked, &TypeTable::new(), 0).unwrap();
        assert_eq!(
            ir.pretty(ast.names()).to_string(),
            "(block @0..5\n  (float-div @0..5\n    (int-to-float @0..1\n      (int 7 @0..1))\n    (int-to-float @4..5\n      (int 2 @4..5))))"
        );
    }

    #[test]
    fn concrete_int_from_floor_division_is_converted() {
        assert_eq!(
            dump("(7 // 2) / 2"),
            "(block @0..12\n  (float-div @0..12\n    (int-to-float @0..8\n      (int-floor-div @1..7\n        (int 7 @1..2)\n        (int 2 @6..7)))\n    (float 2.0 @11..12)))"
        );
    }

    #[test]
    fn parentheses_and_ascriptions_are_dropped() {
        // The root block covers the file's items, parentheses included.
        assert_eq!(dump("((1))"), "(block @0..5\n  (int 1 @2..3))");
        assert_eq!(dump("(1 : Float)"), "(block @0..11\n  (float 1.0 @1..2))");
    }

    #[test]
    fn operators_are_specialised_by_type() {
        let ir = lower_text("-(1 + 2 - 3 * 4 // 5 % 6)");
        let result = result_of(&ir);
        assert_eq!(ir.kind_of(result), ValueKind::Int);
        assert_eq!(
            dump("-(1 + 2 - 3 * 4 // 5 % 6)"),
            "(block @0..25\n  (int-neg @0..25\n    (int-sub @2..24\n      (int-add @2..7\n        (int 1 @2..3)\n        (int 2 @6..7))\n      (int-rem @10..24\n        (int-floor-div @10..20\n          (int-mul @10..15\n            (int 3 @10..11)\n            (int 4 @14..15))\n          (int 5 @19..20))\n        (int 6 @23..24)))))"
        );
        assert_eq!(
            dump("-(1.0 + 2 - 3 * 4.0)"),
            "(block @0..20\n  (float-neg @0..20\n    (float-sub @2..19\n      (float-add @2..9\n        (float 1.0 @2..5)\n        (float 2.0 @8..9))\n      (float-mul @12..19\n        (float 3.0 @12..13)\n        (float 4.0 @16..19)))))"
        );
    }

    #[test]
    fn blocks_variables_and_statements() {
        let text = "let x = 1\n\
                    {\n  let y: Float = 2\n  y := y * 2.0\n}\n\
                    x := x + 1\n\
                    = { x }";
        // Slots are named by the last variable assigned to them: slot 1
        // holds `y` inside the block and then the rebound `x` after it, so
        // it shows as `x#1` throughout.
        assert_eq!(
            dump(text),
            "(block @0..66\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (block @10..47\n\
             \x20   (store x#1 @14..30\n\
             \x20     (float 2.0 @29..30))\n\
             \x20   (store y#2 @33..45\n\
             \x20     (float-mul @38..45\n\
             \x20       (load x#1 @38..39)\n\
             \x20       (float 2.0 @42..45)))\n\
             \x20   (empty @10..47))\n\
             \x20 (store x#1 @48..58\n\
             \x20   (int-add @53..58\n\
             \x20     (load x#0 @53..54)\n\
             \x20     (int 1 @57..58)))\n\
             \x20 (block @61..66\n\
             \x20   (load x#1 @63..64)))"
        );
    }

    #[test]
    fn empty_file_is_an_empty_block_with_an_empty_result() {
        assert_eq!(dump(""), "(block @0..0\n  (empty @0..0))");
        assert_eq!(dump("# nothing\n"), "(block @0..0\n  (empty @0..0))");
        let ir = lower_text("");
        assert_eq!(ir.kind_of(ir.root()), ValueKind::Record);
        assert_eq!(ir.frame_size(), 0);
        assert_eq!(ir.layout_count(), 0);
    }

    #[test]
    fn frame_size_comes_from_the_resolution() {
        assert_eq!(
            lower_text("let a = 1\n{ let b = 2; let c = 3 }").frame_size(),
            3
        );
    }

    #[test]
    fn large_untyped_literal_rounds_to_nearest_float() {
        let ir = lower_text("9007199254740993 / 1");
        let IrKind::FloatDiv(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected FloatDiv");
        };
        assert_eq!(
            ir.get(operands.lhs).kind,
            IrKind::FloatConst(9_007_199_254_740_992.0)
        );
    }

    #[test]
    fn untyped_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::UNTYPED_INT, TyId::INT, TyId::FLOAT]);
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "an untyped integer survived type checking");
        assert_eq!(error.span, Span::new(0, 1));
    }

    #[test]
    fn error_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::ERROR, TyId::FLOAT]);
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "a type error survived type checking");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn a_type_from_another_table_is_an_internal_error() {
        let ast = parse_text("{ x = 1 }");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(
            error.message,
            "a type with no values survived type checking"
        );
    }

    #[test]
    fn impossible_result_type_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![TyId::INT, TyId::INT, TyId::INT]);
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "operator has a result type it cannot have");
        assert_eq!(error.span, Span::new(0, 5));
    }

    #[test]
    fn mismatched_operand_type_is_an_internal_error() {
        let ast = parse_text("1 + 2");
        let checked = Checked {
            types: Types::new(vec![TyId::INT, TyId::FLOAT, TyId::INT]),
            resolution: Resolution::without_variables(3, 1),
        };
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "operand type does not match its operator");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn unresolved_variable_is_an_internal_error() {
        let ast = parse_text("let x = 1\nx");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let unresolved = Checked {
            types: checked.types.clone(),
            resolution: Resolution::without_variables(2, 2),
        };
        let error = lower(&ast, &unresolved, &types, 0).unwrap_err();
        assert_eq!(error.message, "a declaration has no slot");
        assert_eq!(error.span, Span::new(0, 9));
    }

    #[test]
    fn interpolations_list_pieces_and_expressions() {
        assert_eq!(
            dump("let n = 1\n\"n = {n}, {n + 1}.\""),
            "(block @0..29\n\
             \x20 (store n#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (interp @10..29\n\
             \x20   (str \"n = \" @10..29)\n\
             \x20   (load n#0 @16..17)\n\
             \x20   (str \", \" @10..29)\n\
             \x20   (int-add @21..26\n\
             \x20     (load n#0 @21..22)\n\
             \x20     (int 1 @25..26))\n\
             \x20   (str \".\" @10..29)))"
        );
    }

    #[test]
    fn comparisons_are_specialised_by_operand_type() {
        assert_eq!(
            dump("1 < 2.5"),
            "(block @0..7\n  (float-cmp < @0..7\n    (float 1.0 @0..1)\n    (float 2.5 @4..7)))"
        );
        assert_eq!(
            dump("1 >= 2"),
            "(block @0..6\n  (int-cmp >= @0..6\n    (int 1 @0..1)\n    (int 2 @5..6)))"
        );
        assert_eq!(
            dump("\"a\" != \"b\""),
            "(block @0..10\n  (str-cmp != @0..10\n    (str \"a\" @0..3)\n    (str \"b\" @7..10)))"
        );
        assert_eq!(
            dump("true != false"),
            "(block @0..13\n  (bool-eq != @0..13\n    (bool true @0..4)\n    (bool false @8..13)))"
        );
        assert_eq!(
            dump("{} == {}"),
            "(block @0..8\n  (record-eq == [] @0..8\n    (block @0..2\n      (empty @0..2))\n    (block @6..8\n      (empty @6..8))))"
        );
    }

    #[test]
    fn if_keeps_both_branches() {
        assert_eq!(
            dump("if true { 1 } else { 2 }"),
            "(block @0..24\n\
             \x20 (if @0..24\n\
             \x20   (bool true @3..7)\n\
             \x20   (block @8..13\n\
             \x20     (int 1 @10..11))\n\
             \x20   (block @19..24\n\
             \x20     (int 2 @21..22))))"
        );
        // The implicit else branch is an empty block yielding `{}`.
        assert_eq!(
            dump("if true {}"),
            "(block @0..10\n\
             \x20 (if @0..10\n\
             \x20   (bool true @3..7)\n\
             \x20   (block @8..10\n\
             \x20     (empty @8..10))\n\
             \x20   (block @10..10\n\
             \x20     (empty @10..10))))"
        );
        let ir = lower_text("if true { 1 } else { 2.5 }");
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Float);
    }

    #[test]
    fn logic_and_concatenation() {
        assert_eq!(
            dump("true and false"),
            "(block @0..14\n  (and @0..14\n    (bool true @0..4)\n    (bool false @9..14)))"
        );
        assert_eq!(
            dump("not true or false"),
            "(block @0..17\n  (or @0..17\n    (not @0..8\n      (bool true @4..8))\n    (bool false @12..17)))"
        );
        assert_eq!(
            dump("\"a\" + r\"b\""),
            "(block @0..10\n  (str-concat @0..10\n    (str \"a\" @0..3)\n    (str \"b\" @6..10)))"
        );
    }

    #[test]
    fn string_constants_are_resolved_into_the_irs_table() {
        let ir = lower_text("\"a{1}b\" + \"c\\n\"");
        let IrKind::StrConcat(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected a concatenation");
        };
        let IrKind::Interp(items) = ir.get(operands.lhs).kind else {
            panic!("expected an interpolation");
        };
        let text = |id: IrId| match ir.get(id).kind {
            IrKind::StrConst(constant) => ir.string(constant).to_string(),
            other => format!("{other:?}"),
        };
        let items: Vec<String> = ir.items(items).iter().map(|&id| text(id)).collect();
        assert_eq!(items, vec!["a", "IntConst(1)", "b"]);
        assert_eq!(text(operands.rhs), "c\n");
    }

    #[test]
    fn tables_of_the_wrong_size_are_an_internal_error() {
        let (ast, _) = concrete_int_division(Vec::new());
        let checked = Checked {
            types: Types::new(vec![TyId::INT]),
            resolution: Resolution::without_variables(3, 1),
        };
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "type table does not belong to this AST");
        let checked = Checked {
            types: Types::new(vec![TyId::INT, TyId::INT, TyId::FLOAT]),
            resolution: Resolution::without_variables(3, 2),
        };
        let error = lower(&ast, &checked, &TypeTable::new(), 0).unwrap_err();
        assert_eq!(error.message, "resolution does not belong to this AST");
    }

    #[test]
    fn a_record_block_stores_its_fields_and_makes_a_record_of_them() {
        // Private `let`s and statements are items too; the record is the
        // fields' slots, loaded in definition order.
        assert_eq!(
            dump("{ x = 1; let t = x; {}; \"a b\" = t }"),
            "(block @0..35\n\
             \x20 (block @0..35\n\
             \x20   (store x#0 @2..7\n\
             \x20     (int 1 @6..7))\n\
             \x20   (store t#1 @9..18\n\
             \x20     (load x#0 @17..18))\n\
             \x20   (block @20..22\n\
             \x20     (empty @20..22))\n\
             \x20   (store \"a b\"#2 @24..33\n\
             \x20     (load t#1 @32..33))\n\
             \x20   (make-record [x \"a b\"] @0..35\n\
             \x20     (load x#0 @2..3)\n\
             \x20     (load \"a b\"#2 @24..29))))"
        );
        // A file can be a record block too.
        assert_eq!(
            dump("y = 2.5\nx = true"),
            "(block @0..16\n\
             \x20 (store y#0 @0..7\n\
             \x20   (float 2.5 @4..7))\n\
             \x20 (store x#1 @8..16\n\
             \x20   (bool true @12..16))\n\
             \x20 (make-record [y x] @0..16\n\
             \x20   (load y#0 @0..1)\n\
             \x20   (load x#1 @8..9)))"
        );
        let ir = lower_text("{ x = 1 }");
        assert_eq!(ir.kind_of(ir.root()), ValueKind::Record);
    }

    #[test]
    fn field_access_and_nested_records() {
        // The inner block's `n` takes the slot that `a`, declared after its
        // initialiser, then reuses, so it shows as `a#0` too.
        assert_eq!(
            dump("{ a = { n = 1.5 } }.a.n"),
            "(block @0..23\n\
             \x20 (field n @0..23\n\
             \x20   (field a @0..21\n\
             \x20     (block @0..19\n\
             \x20       (store a#0 @2..17\n\
             \x20         (block @6..17\n\
             \x20           (store a#0 @8..15\n\
             \x20             (float 1.5 @12..15))\n\
             \x20           (make-record [n] @6..17\n\
             \x20             (load a#0 @8..9))))\n\
             \x20       (make-record [a] @0..19\n\
             \x20         (load a#0 @2..3))))))"
        );
        let ir = lower_text("{ a = { n = 1.5 } }.a.n");
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Float);
        // Record equality and a field of every kind.
        assert_eq!(
            dump("{ x = 1 } != { x = 2 }"),
            "(block @0..22\n\
             \x20 (record-eq != [x] @0..22\n\
             \x20   (block @0..9\n\
             \x20     (store x#0 @2..7\n\
             \x20       (int 1 @6..7))\n\
             \x20     (make-record [x] @0..9\n\
             \x20       (load x#0 @2..3)))\n\
             \x20   (block @13..22\n\
             \x20     (store x#0 @15..20\n\
             \x20       (int 2 @19..20))\n\
             \x20     (make-record [x] @13..22\n\
             \x20       (load x#0 @15..16)))))"
        );
    }

    #[test]
    fn a_literal_checked_against_a_record_type_is_lowered_at_that_type() {
        assert_eq!(
            dump("let p: {x: Float} = { x = 1 }"),
            "(block @0..29\n\
             \x20 (store p#0 @0..29\n\
             \x20   (block @20..29\n\
             \x20     (store p#0 @22..27\n\
             \x20       (float 1.0 @26..27))\n\
             \x20     (make-record [x] @20..29\n\
             \x20       (load p#0 @22..23))))\n\
             \x20 (empty @0..29))"
        );
    }

    #[test]
    fn layouts_are_shared_by_blocks_with_the_same_names() {
        let ir = lower_text(
            "let a = { x = 1; y = 2 }\nlet b = { x = \"s\"; y = true }\n{ y = 1; x = 2 }",
        );
        // `x y` twice, then `y x`, a different order.
        assert_eq!(ir.layout_count(), 2);
        let ir = lower_text("{ x = 1 } == { x = 2 }");
        assert_eq!(ir.layout_count(), 1);
        let IrKind::RecordEq(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected a record comparison");
        };
        let layout_of = |block: IrId| {
            let IrKind::Block { result, .. } = ir.get(block).kind else {
                panic!("expected a block");
            };
            let IrKind::MakeRecord { layout, .. } = ir.get(result).kind else {
                panic!("expected a record");
            };
            layout
        };
        let (lhs, rhs) = (layout_of(operands.lhs), layout_of(operands.rhs));
        assert_eq!(lhs, rhs);
        assert_eq!(&*ir.layout(lhs).texts, &[Box::from("x")]);
    }

    #[test]
    fn a_merged_field_updates_its_slot() {
        // Each leaf is stored in its cell at its own line; each interior node
        // is assembled from its children's cells after its last child, the
        // root last. The field is loaded once for the record, from its root's
        // cell.
        assert_eq!(
            dump("a.x = 1\na.y.z = 2\na.y.w = 3"),
            "(block @0..27\n\
             \x20 (store x#1 @0..7\n\
             \x20   (int 1 @6..7))\n\
             \x20 (store z#3 @8..17\n\
             \x20   (int 2 @16..17))\n\
             \x20 (store w#4 @18..27\n\
             \x20   (int 3 @26..27))\n\
             \x20 (store y#2 @8..17\n\
             \x20   (make-record [z w] @8..17\n\
             \x20     (load z#3 @8..17)\n\
             \x20     (load w#4 @18..27)))\n\
             \x20 (store a#0 @0..7\n\
             \x20   (make-record [x y] @0..7\n\
             \x20     (load x#1 @0..7)\n\
             \x20     (load y#2 @8..17)))\n\
             \x20 (make-record [a] @0..27\n\
             \x20   (load a#0 @0..1)))"
        );
        // The name of every update of an inner block's dotted field is
        // recorded for the evaluator.
        let ast = parse_text("r = { a.x = 1\na.\"y z\".w = 2 }");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 0).unwrap();
        let text = |text: &str| ir.name_text(ast.names().lookup(text).unwrap());
        assert_eq!(text("x"), Some("x"));
        assert_eq!(text("y z"), Some("y z"));
        assert_eq!(text("w"), Some("w"));
        // Not the field's own name, which no update adds.
        assert_eq!(text("a"), None);
    }

    #[test]
    fn a_merged_field_takes_its_place_where_first_mentioned() {
        assert_eq!(
            dump("{ a.x = 1; b = 2; a.y = 3 }"),
            "(block @0..27\n\
             \x20 (block @0..27\n\
             \x20   (store a#0 @2..9\n\
             \x20     (update-field x @2..5\n\
             \x20       (empty @2..3)\n\
             \x20       (int 1 @8..9)))\n\
             \x20   (store b#1 @11..16\n\
             \x20     (int 2 @15..16))\n\
             \x20   (store a#0 @18..25\n\
             \x20     (update-field y @18..21\n\
             \x20       (load a#0 @18..19)\n\
             \x20       (int 3 @24..25)))\n\
             \x20   (make-record [a b] @0..27\n\
             \x20     (load a#0 @2..3)\n\
             \x20     (load b#1 @11..12))))"
        );
    }

    #[test]
    fn a_dotted_let_updates_the_old_record_into_a_new_slot() {
        assert_eq!(
            dump("let a = { b = { c = 1 } }\nlet a.b.c = 2"),
            "(block @0..39\n\
             \x20 (store a#0 @0..25\n\
             \x20   (block @8..25\n\
             \x20     (store a#0 @10..23\n\
             \x20       (block @14..23\n\
             \x20         (store a#0 @16..21\n\
             \x20           (int 1 @20..21))\n\
             \x20         (make-record [c] @14..23\n\
             \x20           (load a#0 @16..17))))\n\
             \x20     (make-record [b] @8..25\n\
             \x20       (load a#0 @10..11))))\n\
             \x20 (store a#1 @26..39\n\
             \x20   (update .b.c @30..35\n\
             \x20     (int 2 @38..39)\n\
             \x20     (load a#0 @30..31)))\n\
             \x20 (empty @0..39))"
        );
        // Where the static type lacks a segment, its record is created.
        assert_eq!(
            dump("let a = 1\nlet b.c.d = a\nb.c := { d = 2 }"),
            "(block @0..40\n\
             \x20 (store a#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store b#1 @10..23\n\
             \x20   (update .c+.d @14..19\n\
             \x20     (load a#0 @22..23)\n\
             \x20     (empty @14..15)))\n\
             \x20 (store b#2 @24..40\n\
             \x20   (update .c @24..27\n\
             \x20     (block @31..40\n\
             \x20       (store b#2 @33..38\n\
             \x20         (int 2 @37..38))\n\
             \x20       (make-record [d] @31..40\n\
             \x20         (load b#2 @33..34)))\n\
             \x20     (load b#1 @24..25)))\n\
             \x20 (empty @0..40))"
        );
    }

    #[test]
    fn a_rebind_of_a_mutable_variable_stores_into_its_slot() {
        // From an inner block, plainly or along a path, the rebind is a
        // store into the variable's one slot, the dotted one of an update
        // of the record loaded from it.
        assert_eq!(
            dump("let mut x = 1\n{ x := 2 }\nlet mut r = { a = x }\nif true { r.a := x + 1 }\nr.a"),
            "(block @0..75\n\
             \x20 (store x#0 @0..13\n\
             \x20   (int 1 @12..13))\n\
             \x20 (block @14..24\n\
             \x20   (store x#0 @16..22\n\
             \x20     (int 2 @21..22))\n\
             \x20   (empty @14..24))\n\
             \x20 (store r#1 @25..46\n\
             \x20   (block @37..46\n\
             \x20     (store r#1 @39..44\n\
             \x20       (load x#0 @43..44))\n\
             \x20     (make-record [a] @37..46\n\
             \x20       (load r#1 @39..40))))\n\
             \x20 (if @47..71\n\
             \x20   (bool true @50..54)\n\
             \x20   (block @55..71\n\
             \x20     (store r#1 @57..69\n\
             \x20       (update .a @57..60\n\
             \x20         (int-add @64..69\n\
             \x20           (load x#0 @64..65)\n\
             \x20           (int 1 @68..69))\n\
             \x20         (load r#1 @57..58)))\n\
             \x20     (empty @55..71))\n\
             \x20   (block @71..71\n\
             \x20     (empty @71..71)))\n\
             \x20 (field a @72..75\n\
             \x20   (load r#1 @72..73)))"
        );
        // A frame does not grow with the rebinds.
        assert_eq!(
            lower_text("let mut x = 1\nx := 2\nx := 3\nx").frame_size(),
            1
        );
    }

    #[test]
    fn typed_record_equality_compares_over_a_shape() {
        assert_eq!(
            dump("let p = { x = 1; inner = { n = 2 } }\np == p"),
            "(block @0..43\n\
             \x20 (store p#0 @0..36\n\
             \x20   (block @8..36\n\
             \x20     (store p#0 @10..15\n\
             \x20       (int 1 @14..15))\n\
             \x20     (store inner#1 @17..34\n\
             \x20       (block @25..34\n\
             \x20         (store inner#1 @27..32\n\
             \x20           (int 2 @31..32))\n\
             \x20         (make-record [n] @25..34\n\
             \x20           (load inner#1 @27..28))))\n\
             \x20     (make-record [x inner] @8..36\n\
             \x20       (load p#0 @10..11)\n\
             \x20       (load inner#1 @17..22))))\n\
             \x20 (record-eq == [x inner[n]] @37..43\n\
             \x20   (load p#0 @37..38)\n\
             \x20   (load p#0 @42..43)))"
        );
        // The shape is the static type's, not the value's.
        let text = "let q: {x: Int} = { x = 1; y = 2 }\nq != { x = 1 }";
        assert!(dump(text).contains("(record-eq != [x] @"));
        // One shape per record type, nested ones included, shared by every
        // comparison of it.
        let ir = lower_text(
            "let p = { x = 1; inner = { n = 2 } }\n\
             let a = p == p\n\
             let b = p.inner != p.inner\n\
             let c = { n = 3 } == p.inner\n\
             = { e = {} } == { e = {} }",
        );
        assert_eq!(ir.shape_count(), 4);
        // An empty record's shape has no fields.
        assert_eq!(
            dump("{ e = {} } == { e = {} }").lines().nth(1),
            Some("  (record-eq == [e[]] @0..24")
        );
    }

    #[test]
    fn total_equality_lowers_like_typed_equality_but_on_records() {
        assert_eq!(
            dump("{ x = 1 } !== { x = 1 }"),
            "(block @0..23\n\
             \x20 (record-same !== @0..23\n\
             \x20   (block @0..9\n\
             \x20     (store x#0 @2..7\n\
             \x20       (int 1 @6..7))\n\
             \x20     (make-record [x] @0..9\n\
             \x20       (load x#0 @2..3)))\n\
             \x20   (block @14..23\n\
             \x20     (store x#0 @16..21\n\
             \x20       (int 1 @20..21))\n\
             \x20     (make-record [x] @14..23\n\
             \x20       (load x#0 @16..17)))))"
        );
        assert_eq!(
            dump("{} === {}"),
            "(block @0..9\n  (record-same === @0..9\n    (block @0..2\n      (empty @0..2))\n    (block @7..9\n      (empty @7..9))))"
        );
        // Scalars lower exactly as `==` and `!=` do.
        assert_eq!(
            dump("1 === 2"),
            "(block @0..7\n  (int-cmp == @0..7\n    (int 1 @0..1)\n    (int 2 @6..7)))"
        );
        assert_eq!(
            dump("1.5 !== 2"),
            "(block @0..9\n  (float-cmp != @0..9\n    (float 1.5 @0..3)\n    (float 2.0 @8..9)))"
        );
        assert_eq!(
            dump("\"a\" === \"b\""),
            "(block @0..11\n  (str-cmp == @0..11\n    (str \"a\" @0..3)\n    (str \"b\" @8..11)))"
        );
        assert_eq!(
            dump("true !== false"),
            "(block @0..14\n  (bool-eq != @0..14\n    (bool true @0..4)\n    (bool false @9..14)))"
        );
        assert_eq!(lower_text("{ x = 1 } === { x = 2 }").shape_count(), 0);
    }

    #[test]
    fn a_closure_captures_in_the_creating_frame() {
        assert_eq!(
            dump("let x = 1\nlet f = fn(y: Int): Int { x + y }"),
            "(block @0..43\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store f#1 @10..43\n\
             \x20   (make-closure #0 @18..43\n\
             \x20     (load x#0 @18..43)))\n\
             \x20 (empty @0..43))\n\
             fn #0 f (frame 1, params 1, captures 1, height 3)\n\
             \x20 (block @34..43\n\
             \x20   (int-add @36..41\n\
             \x20     (load-capture 0 @36..37)\n\
             \x20     (load y#0 @40..41)))"
        );
    }

    #[test]
    fn a_call_passes_its_arguments_in_source_order() {
        let text = "fn f(a: Int, b: String): Int { a }\nf(b = \"s\", a = 1)";
        assert_eq!(
            dump(text),
            "(block @0..52\n\
             \x20 (store f#0 @0..34\n\
             \x20   (make-closure #0 @0..34))\n\
             \x20 (call @35..52\n\
             \x20   (load f#0 @35..36)\n\
             \x20   (arg 1 @37..44\n\
             \x20     (str \"s\" @41..44))\n\
             \x20   (arg 0 @46..51\n\
             \x20     (int 1 @50..51))))\n\
             fn #0 f (frame 2, params 2, captures 0, height 2)\n\
             \x20 (block @29..34\n\
             \x20   (load a#0 @31..32))"
        );
        // The root's result, after the function's declaration.
        let call = |ir: &Ir| match ir.get(ir.root()).kind {
            IrKind::Block { result, .. } => result,
            other => panic!("the root is not a block: {other:?}"),
        };
        let ir = lower_text(text);
        assert_eq!(ir.kind_of(call(&ir)), ValueKind::Int);
        let IrKind::Call { args, kind, .. } = ir.get(call(&ir)).kind else {
            panic!("expected a call");
        };
        assert_eq!(kind, ValueKind::Int);
        let arg = ir.items(args)[0];
        assert_eq!(ir.kind_of(arg), ValueKind::String);
        // A trailing block is the last argument.
        let ir = lower_text("fn f(a: Int, r: {x: Int}): Int { a }\nf(1) { x = 2 }");
        let IrKind::Call { args, .. } = ir.get(call(&ir)).kind else {
            panic!("expected a call");
        };
        let params: Vec<u32> = ir
            .items(args)
            .iter()
            .map(|&arg| match ir.get(arg).kind {
                IrKind::Arg { param, .. } => param,
                other => panic!("expected an argument, got {other:?}"),
            })
            .collect();
        assert_eq!(params, vec![0, 1]);
    }

    #[test]
    fn a_recursive_function_loads_its_callee() {
        assert_eq!(
            dump("fn fact(n: Int): Int { if n <= 1 { 1 } else { n * fact(n - 1) } }"),
            "(block @0..65\n\
             \x20 (store fact#0 @0..65\n\
             \x20   (make-closure #0 @0..65))\n\
             \x20 (empty @0..65))\n\
             fn #0 fact (frame 1, params 1, captures 0, height 8)\n\
             \x20 (block @21..65\n\
             \x20   (if @23..63\n\
             \x20     (int-cmp <= @26..32\n\
             \x20       (load n#0 @26..27)\n\
             \x20       (int 1 @31..32))\n\
             \x20     (block @33..38\n\
             \x20       (int 1 @35..36))\n\
             \x20     (block @44..63\n\
             \x20       (int-mul @46..61\n\
             \x20         (load n#0 @46..47)\n\
             \x20         (call @50..61\n\
             \x20           (load-callee @50..54)\n\
             \x20           (arg 0 @55..60\n\
             \x20             (int-sub @55..60\n\
             \x20               (load n#0 @55..56)\n\
             \x20               (int 1 @59..60))))))))"
        );
    }

    #[test]
    fn nested_functions_chain_their_captures() {
        // The innermost, pure, function measures `a` as its result, which
        // makes `a` present for `outer`.
        let text = "fn outer(a: Int): fn(): fn(): Int { fn(): fn(): Int { fn(): Int { a } } }";
        assert_eq!(
            dump(text),
            "(block @0..73\n\
             \x20 (store outer#0 @0..73\n\
             \x20   (make-closure #2 @0..73))\n\
             \x20 (empty @0..73))\n\
             fn #0 (frame 0, params 0, captures 1, height 2)\n\
             \x20 (block @64..69\n\
             \x20   (load-capture 0 @66..67))\n\
             fn #1 (frame 0, params 0, captures 1, height 3)\n\
             \x20 (block @52..71\n\
             \x20   (make-closure #0 @54..69\n\
             \x20     (load-capture 0 @54..69)))\n\
             fn #2 outer (frame 1, params 1, captures 0, height 3)\n\
             \x20 (block @34..73\n\
             \x20   (make-closure #1 @36..71\n\
             \x20     (load a#0 @36..71)))"
        );
        // A captured callee is loaded as the callee in the creating frame.
        let ir = lower_text("fn f(): fn(): Int { fn(): Int { f()() } }");
        let outer = ir.function(FnId::new(1));
        let IrKind::Block { result, .. } = ir.get(outer.body).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeClosure { function, captures } = ir.get(result).kind else {
            panic!("expected a closure");
        };
        assert_eq!(function, FnId::new(0));
        let captures: Vec<IrKind> = ir
            .items(captures)
            .iter()
            .map(|&id| ir.get(id).kind)
            .collect();
        assert_eq!(captures, vec![IrKind::LoadCallee]);
    }

    #[test]
    fn a_memoised_function_is_code_of_its_own_kind() {
        let ir = lower_text(
            "fn f(x: Int) ->> Int { x }\n\
             fn g(x: Int) -> Int { x }\n\
             fn h(x: Int): Int { x }\n\
             let l = lazy f(1)",
        );
        let kinds: Vec<FnKind> = (0..4).map(|id| ir.function(FnId::new(id)).kind).collect();
        assert_eq!(
            kinds,
            vec![
                FnKind::Memo,
                FnKind::Function,
                FnKind::Function,
                FnKind::Future
            ]
        );
        assert_eq!(&*ir.function(FnId::new(0)).display, "fn(x: Int) ->> Int");
        // The IR's dump shows it.
        assert_eq!(
            dump("fn f(x: Int) ->> Int { x }"),
            "(block @0..26\n\
             \x20 (store f#0 @0..26\n\
             \x20   (make-closure #0 @0..26))\n\
             \x20 (empty @0..26))\n\
             memo fn #0 f (frame 1, params 1, captures 0, height 2)\n\
             \x20 (block @21..26\n\
             \x20   (load x#0 @23..24))"
        );
    }

    #[test]
    fn the_function_table_describes_each_function() {
        let ast = parse_text(
            "let k = 2\n\
             fn scale(x: Int, by: {f: Float}) -> Float { let y = x * k\ny / by.f }\n\
             let anon = fn(): {} {}",
        );
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 7).unwrap();
        assert_eq!(ir.input(), 7);
        assert_eq!(ir.function_count(), 2);
        let scale = ir.function(FnId::new(0));
        assert_eq!(
            &*scale.display,
            "fn(x: Int | Future<Int>, by: {f: Float | Future<Float>} | Future<{f: Float | \
             Future<Float>}>) -> Float"
        );
        assert_eq!(scale.kind, FnKind::Function);
        assert_eq!(scale.frame_size, 3);
        assert_eq!(scale.param_count, 2);
        assert_eq!(scale.capture_count, 1);
        assert_eq!(scale.name, ast.names().lookup("scale"));
        let slot_names: Vec<&str> = scale
            .slot_names
            .iter()
            .map(|&name| ast.names().get(name))
            .collect();
        assert_eq!(slot_names, vec!["x", "by", "y"]);
        assert_eq!(ir.kind_of(scale.body), ValueKind::Float);
        let anon = ir.find_function(FnId::new(1)).unwrap();
        assert_eq!(&*anon.display, "fn(): {}");
        assert_eq!(anon.name, ast.names().lookup("anon"));
        assert_eq!(
            (anon.frame_size, anon.param_count, anon.capture_count),
            (0, 0, 0)
        );
        assert!(ir.find_function(FnId::new(2)).is_none());
        assert_eq!(lower_text("1").input(), 0);
        // A closure is a function.
        let ir = lower_text("fn f(): fn(): Int { fn(): Int { 1 } }");
        assert_eq!(ir.kind_of(ir.function(FnId::new(1)).body), ValueKind::Fn);
    }

    /// The depth of each call in the frame whose body is `body`, in the
    /// order a depth-first walk meets them.
    fn depths(ir: &Ir, body: IrId) -> Vec<u16> {
        ir.calls_in(body)
            .into_iter()
            .map(|(depth, _)| depth)
            .collect()
    }

    #[test]
    fn a_call_is_as_deep_as_its_path_from_its_frames_body() {
        let g = "fn g(n: Int): Int { n }\n";
        // In a function's body: the block (0), the `+` (1), the `*` (2),
        // the call (3).
        let ir = lower_text(&format!("{g}fn f(n: Int): Int {{ 1 + g(n) * 2 }}"));
        assert_eq!(depths(&ir, ir.function(FnId::new(1)).body), vec![3]);
        // In the file's body: the block (0), the `let`'s store (1), the
        // outer call (2), its argument (3), the inner call (4).
        let ir = lower_text(&format!("{g}let x = g(g(1))\nx"));
        assert_eq!(depths(&ir, ir.root()), vec![2, 4]);
        // A callee is a child of its call: the block (0), the call of the
        // closure (1), the call that makes it (2).
        let ir = lower_text("fn make(): fn(): Int { fn(): Int { 1 } }\nmake()()");
        assert_eq!(depths(&ir, ir.root()), vec![1, 2]);
    }

    #[test]
    fn the_shared_nodes_of_an_update_chain_do_not_disturb_depths() {
        let g = "fn g(n: Int): Int { n }\n";
        // `let a.b.c = g(a.b.c)`: the block (0), the store (1), the update
        // (2), the call (3), the update's value, its first child, which no
        // node of its path shares; the later call's depth is its own path's
        // (1).
        let ir = lower_text(&format!(
            "{g}let a = {{ b = {{ c = 1 }} }}\nlet a.b.c = g(a.b.c)\n= g(a.b.c)"
        ));
        assert_eq!(depths(&ir, ir.root()), vec![3, 1]);
        // In a function's body, where the update's longest path is through
        // the call: the block (1), the store (2), the update, two levels
        // more for each of its path's two steps (7), the call (8), its
        // argument (9), the literal (10).
        let ir = lower_text(&format!(
            "{g}fn u(r: {{b: {{c: Int}}}}): {{b: {{c: Int}}}} {{ let r.b.c = g(1)\nr }}"
        ));
        let u = ir.function(FnId::new(1));
        assert_eq!(depths(&ir, u.body), vec![3]);
        assert_eq!(u.height, 10);
    }

    #[test]
    fn a_closures_body_is_a_frame_of_its_own() {
        let text = "fn g(n: Int): Int { n }\nlet k = { { fn(): Int { g(1) } } }\ng(k())";
        let ir = lower_text(text);
        // In the frame creating the closure: the block (0), the call of `g`
        // (1), its argument (2), the call of `k` (3). The call in the
        // closure's body is not in this frame.
        assert_eq!(depths(&ir, ir.root()), vec![1, 3]);
        // In the closure's own, however deep it was made: the block (0), the
        // call (1). Its body's height: the block, the call, its argument, the
        // literal.
        let closure = ir.function(FnId::new(1));
        assert_eq!(depths(&ir, closure.body), vec![1]);
        assert_eq!(closure.height, 4);
        // A body that makes a closure is as tall as the loads of what it
        // captures, not the closure's body: the block and the closure, which
        // captures nothing, since `g` is a top-level function, read through
        // the top level's frame.
        let ir = lower_text(
            "fn g(n: Int): Int { n }\nfn outer(): fn(): Int { fn(): Int { g(g(g(1))) } }",
        );
        assert_eq!(ir.function(FnId::new(2)).height, 2);
        // The block, three calls and their arguments, and the literal.
        assert_eq!(ir.function(FnId::new(1)).height, 8);
    }

    #[test]
    fn a_function_body_is_as_tall_as_its_longest_path() {
        // The block (1), the `if` (2), the then-block (3), the negations (4
        // and 5), the load (6); the condition and the else branch are
        // shorter.
        let ir = lower_text("fn h(n: Int): Int { if n > 0 { -(-n) } else { 0 } }");
        assert_eq!(ir.function(FnId::new(0)).height, 6);
        // An empty body is a block with an empty result.
        let ir = lower_text("fn e(): {} {}");
        assert_eq!(ir.function(FnId::new(0)).height, 2);
    }

    #[test]
    fn an_argument_bound_to_no_parameter_is_an_internal_error() {
        let ast = parse_text("fn f(a: Int): Int { a }\nf(1)");
        let mut types = TypeTable::new();
        let (mut checked, _) = check(&ast, &mut types).unwrap();
        checked.resolution.unbind_arguments();
        let error = lower(&ast, &checked, &types, 0).unwrap_err();
        assert_eq!(
            error.message,
            "an argument of a call is bound to no parameter"
        );
        // A resolution without the tree's functions does not belong to it.
        let unresolved = Checked {
            types: checked.types.clone(),
            resolution: Resolution::without_variables(ast.len(), ast.item_count()),
        };
        let error = lower(&ast, &unresolved, &types, 0).unwrap_err();
        assert_eq!(error.message, "resolution does not belong to this AST");
    }

    // ----- Futures -----

    #[test]
    fn a_lazy_makes_a_future_capturing_in_the_creating_frame() {
        let text = "let x = 1\nlet f = lazy x + 1";
        assert_eq!(
            dump(text),
            "(block @0..28\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (store f#1 @10..28\n\
             \x20   (make-future #0 @18..28\n\
             \x20     (load x#0 @18..28)))\n\
             \x20 (empty @0..28))\n\
             future #0 (frame 0, params 0, captures 1, height 2)\n\
             \x20 (int-add @23..28\n\
             \x20   (load-capture 0 @23..24)\n\
             \x20   (int 1 @27..28))"
        );
        let ir = lower_text(text);
        let code = ir.function(FnId::new(0));
        assert_eq!(code.kind, FnKind::Future);
        assert_eq!(&*code.display, "Future<Int>");
        assert_eq!(
            (code.param_count, code.capture_count, code.name),
            (0, 1, None)
        );
        // A future's body is any node, of the kind of what it yields.
        assert_eq!(ir.kind_of(code.body), ValueKind::Int);
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        let IrKind::Store(_, future) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert_eq!(ir.kind_of(future), ValueKind::Deferred);
        // A future made in a function body is made in that frame. `n` is
        // only captured by the `lazy`, which does not measure it for `f`,
        // so it is the either shape and its load `Deferred`.
        let ir = lower_text("fn f(n: Int): Future<Int> { lazy n * 2 }");
        assert_eq!(ir.function(FnId::new(0)).kind, FnKind::Future);
        assert_eq!(ir.function(FnId::new(1)).kind, FnKind::Function);
        let IrKind::Block { result, .. } = ir.get(ir.function(FnId::new(1)).body).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeFuture {
            function, captures, ..
        } = ir.get(result).kind
        else {
            panic!("expected a future");
        };
        assert_eq!(function, FnId::new(0));
        assert_eq!(
            ir.items(captures)
                .iter()
                .map(|&id| ir.get(id).kind)
                .collect::<Vec<_>>(),
            vec![IrKind::Load(Slot::new(0), ValueKind::Deferred)]
        );
    }

    #[test]
    fn an_await_wraps_what_is_awaited() {
        // At the top, of the kind of what is awaited once present.
        let text = "let x = lazy 1\nx + 1";
        assert_eq!(
            dump(text),
            "(block @0..20\n\
             \x20 (store x#0 @0..14\n\
             \x20   (make-future #0 @8..14))\n\
             \x20 (int-add @15..20\n\
             \x20   (await @15..16\n\
             \x20     (load x#0 @15..16))\n\
             \x20   (int 1 @19..20)))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 1 @13..14)"
        );
        let ir = lower_text(text);
        let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an addition");
        };
        let IrKind::Await {
            value, kind, shape, ..
        } = ir.get(operands.lhs).kind
        else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::Int);
        assert_eq!(shape, LazyShapeId::NONE);
        assert_eq!(ir.kind_of(operands.lhs), ValueKind::Int);
        assert_eq!(ir.kind_of(value), ValueKind::Deferred);
        // An explicit await is one node, spanning the `await`.
        assert_eq!(
            dump("let x = lazy 1\nawait x").lines().nth(3),
            Some("  (await @15..22")
        );
        let ir = lower_text("let x = lazy 1\nawait x");
        let awaited = result_after_items(&ir);
        let IrKind::Await { value, .. } = ir.get(awaited).kind else {
            panic!("expected an await");
        };
        assert!(matches!(ir.get(value).kind, IrKind::Load(..)));
        // Of a present value, it is its operand.
        assert_eq!(
            dump("let x = 1\nawait x"),
            "(block @0..17\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (load x#0 @16..17))"
        );
        // A parenthesis is dropped, and the await spans it.
        assert_eq!(
            dump("let x = lazy 1\n(x) + 1").lines().nth(4),
            Some("    (await @15..18")
        );
    }

    #[test]
    fn a_call_of_an_inferred_present_result_is_not_awaited() {
        // An impure function whose body returns a present record of present
        // fields returns one: its call is of a concrete kind, and neither it
        // nor its field is awaited.
        let body = "{ a = 1; b = \"x\" }\n";
        let text = format!("fn get() -> {{a: Int, b: String}} {body}get().a + 1");
        let ir = lower_text(&text);
        let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an addition");
        };
        let IrKind::Field { record, kind, .. } = ir.get(operands.lhs).kind else {
            panic!("expected a field access");
        };
        assert_eq!(kind, ValueKind::Int);
        let IrKind::Call { kind, .. } = ir.get(record).kind else {
            panic!("expected a call");
        };
        assert_eq!(kind, ValueKind::Record);
        assert!(!dump(&text).contains("(await"));
        // Written as the either shape, the call may be a future, and so may
        // its field: both are awaited.
        let either = "{a: Int | Future<Int>, b: String | Future<String>}";
        let text = format!("fn get() -> {either} | Future<{either}> {body}get().a + 1");
        let ir = lower_text(&text);
        let IrKind::IntAdd(operands) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an addition");
        };
        let IrKind::Await { value, .. } = ir.get(operands.lhs).kind else {
            panic!("expected an await of the field");
        };
        let IrKind::Field { record, .. } = ir.get(value).kind else {
            panic!("expected a field access");
        };
        let IrKind::Await { value, .. } = ir.get(record).kind else {
            panic!("expected an await of the call");
        };
        let IrKind::Call { kind, .. } = ir.get(value).kind else {
            panic!("expected a call");
        };
        assert_eq!(kind, ValueKind::Deferred);
        assert_eq!(dump(&text).matches("(await").count(), 2);
    }

    #[test]
    fn a_deep_await_carries_a_lazy_shape() {
        let text = "let r = { a = lazy 1; b = { c = lazy 2; d = 3 }; e = 4 }\n\"{r}\"";
        let shown = dump(text);
        assert!(
            shown.contains(
                "\n  (interp @57..62\n\
                 \x20   (str \"\" @57..62)\n\
                 \x20   (await [a! b[c!]] @59..60\n\
                 \x20     (load r#0 @59..60))\n"
            ),
            "{shown}"
        );
        let ast = parse_text(text);
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let ir = lower(&ast, &checked, &types, 0).unwrap();
        assert_eq!(ir.lazy_shape_count(), 2);
        let IrKind::Interp(items) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an interpolation");
        };
        let IrKind::Await { shape, kind, .. } = ir.get(ir.items(items)[1]).kind else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::Record);
        let name = |text: &str| ast.names().lookup(text).unwrap();
        let [
            (a, LazyField::Await(LazyShapeId::NONE)),
            (b, LazyField::Within(inner)),
        ] = *ir.lazy_shape(shape).fields
        else {
            panic!("unexpected shape {:?}", ir.lazy_shape(shape));
        };
        assert_eq!((a, b), (name("a"), name("b")));
        assert_eq!(
            &*ir.lazy_shape(inner).fields,
            &[(name("c"), LazyField::Await(LazyShapeId::NONE))]
        );
        assert_eq!(ir.find_lazy_shape(LazyShapeId::NONE), None);
        // A field that is lazy and descended into, and a present record
        // with lazy fields.
        let shown = dump(
            "let r = { a = lazy { x = lazy 1; y = 2 }; b = { c = { d = lazy 3 } } }\n= \"{r}\"",
        );
        assert!(shown.contains("(await [a![x!] b[c[d!]]] @"), "{shown}");
        // Only the lazy parts are listed; a record with none needs no
        // shape, and is not awaited.
        let shown = dump("let r = { a = 1; b = { c = 2 } }\n\"{r}\"");
        assert!(!shown.contains("await"), "{shown}");
        // Shapes are shared by the deep awaits of one type.
        let ir = lower_text("let r = { a = lazy 1 }\nlet s = \"{r}\"\nlet t = \"{r} {r}\"");
        assert_eq!(ir.lazy_shape_count(), 1);
        // An equality awaits deeply and compares over the present fields;
        // the nested record type, in an impure signature, may be a future
        // itself.
        let shown = dump("fn f(r: {a: {b: Int}}) -> Bool { r == r }");
        assert!(shown.contains("\n    (record-eq == [a[b]] @"), "{shown}");
        assert!(shown.contains("\n      (await [a![b!]] @"), "{shown}");
    }

    #[test]
    fn values_that_may_be_futures_are_of_kind_deferred() {
        // An `if` of such a type, with a present branch.
        let text = "let x = lazy 1\nlet y = if true { x } else { 2 }";
        assert_eq!(
            dump(text),
            "(block @0..47\n\
             \x20 (store x#0 @0..14\n\
             \x20   (make-future #0 @8..14))\n\
             \x20 (store y#1 @15..47\n\
             \x20   (if @23..47\n\
             \x20     (bool true @26..30)\n\
             \x20     (block @31..36\n\
             \x20       (load x#0 @33..34))\n\
             \x20     (block @42..47\n\
             \x20       (int 2 @44..45))))\n\
             \x20 (empty @0..47))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 1 @13..14)"
        );
        let ir = lower_text(text);
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        let IrKind::Store(_, choice) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert_eq!(ir.kind_of(choice), ValueKind::Deferred);
        // A present value stored in a slot of such a type.
        assert_eq!(
            dump("let x: Int | Future<Int> = 1"),
            "(block @0..28\n\
             \x20 (store x#0 @0..28\n\
             \x20   (int 1 @27..28))\n\
             \x20 (empty @0..28))"
        );
        // A pure function passing an either parameter along: a load of
        // kind `Deferred`, stored in a record, never awaited.
        let text = "fn keep(x: Int | Future<Int>): {v: Int | Future<Int>} { v = x }";
        assert_eq!(
            dump(text),
            "(block @0..63\n\
             \x20 (store keep#0 @0..63\n\
             \x20   (make-closure #0 @0..63))\n\
             \x20 (empty @0..63))\n\
             fn #0 keep (frame 2, params 1, captures 0, height 3)\n\
             \x20 (block @54..63\n\
             \x20   (store v#1 @56..61\n\
             \x20     (load x#0 @60..61))\n\
             \x20   (make-record [v] @54..63\n\
             \x20     (load v#1 @56..57)))"
        );
        let ir = lower_text(text);
        let IrKind::Block { items, .. } = ir.get(ir.function(FnId::new(0)).body).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, load) = ir.get(ir.items(items)[0]).kind else {
            panic!("expected a store");
        };
        assert_eq!(
            ir.get(load).kind,
            IrKind::Load(Slot::new(0), ValueKind::Deferred)
        );
        // A bare `await` item of a value that is not `{}`.
        let ir = lower_text("let x = lazy 1\nawait x\n2");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        assert!(matches!(
            ir.get(ir.items(items)[1]).kind,
            IrKind::Await {
                kind: ValueKind::Int,
                ..
            }
        ));
    }

    #[test]
    fn awaits_are_as_deep_as_their_path_from_their_frames_body() {
        // In the file's body: the block (0), the `let`'s store (1), the `+`
        // (2), the await (3). And the explicit await of the result (1).
        let ir = lower_text("let x = lazy 1\nlet y = x + 1\nawait x");
        assert_eq!(
            ir.awaits_in(ir.root())
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![3, 1]
        );
        // In a function's body, which counts the await in its height: the
        // block (1), the `*` (2), the await (3), the load (4).
        let ir = lower_text("fn f(x: Int) -> Int { x * 2 }");
        let code = ir.function(FnId::new(0));
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(code.height, 4);
        // In a future's body: the await (0), the capture (1).
        let ir = lower_text("let x = lazy 1\nlet y = lazy x + 1");
        let code = ir.function(FnId::new(1));
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(code.height, 3);
    }

    #[test]
    fn an_update_awaits_each_record_along_its_path_that_may_be_a_future() {
        // An impure function's parameter, the either shape at every level:
        // each record is awaited, the variable's by an await of its read,
        // spanning its name, and the record at `r.a` by the step that reads
        // it, spanning the path up to it.
        let text = "fn f(r: {a: {b: Int}}) -> {} { let r.a.b = 2 }";
        assert_eq!(
            dump(text),
            "(block @0..46\n\
             \x20 (store f#0 @0..46\n\
             \x20   (make-closure #0 @0..46))\n\
             \x20 (empty @0..46))\n\
             fn #0 f (frame 2, params 1, captures 0, height 9)\n\
             \x20 (block @29..46\n\
             \x20   (store r#1 @31..44\n\
             \x20     (update .a!.b @35..40\n\
             \x20       (int 2 @43..44)\n\
             \x20       (await @35..36\n\
             \x20         (load r#0 @35..36))))\n\
             \x20   (empty @29..46))"
        );
        let ir = lower_text(text);
        let code = ir.function(FnId::new(0));
        let IrKind::Block { items, .. } = ir.get(code.body).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, update) = ir.get(ir.items(items)[0]).kind else {
            panic!("expected a store");
        };
        let IrKind::Update {
            parts,
            path,
            kind: ValueKind::Record,
            ..
        } = ir.get(update).kind
        else {
            panic!("expected an update of a record");
        };
        let &[_, first] = ir.items(parts) else {
            panic!("expected a value and a record");
        };
        // The read is of kind `Deferred`, and the await of a record.
        let IrKind::Await {
            value: load,
            kind: ValueKind::Record,
            shape: LazyShapeId::NONE,
            ..
        } = ir.get(first).kind
        else {
            panic!("expected an await of a record");
        };
        assert_eq!(
            ir.get(load).kind,
            IrKind::Load(Slot::new(0), ValueKind::Deferred)
        );
        let steps = ir.update_path(path).unwrap();
        let modes: Vec<_> = steps.iter().map(|step| (step.mode, step.span)).collect();
        assert_eq!(
            modes,
            vec![
                (StepMode::Await, Span::new(35, 38)),
                (StepMode::Set, Span::new(35, 40))
            ]
        );
        // The block (0), the store (1), the update (2), its record's await
        // (3); the step's await is the update's own.
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![3]
        );
        // A record that is present is read as before, beside one that is
        // not.
        let text = "let r = { a = lazy { b = 1 } }\nlet r.a.b = 2";
        let shown = dump(text);
        assert!(
            shown.contains(
                "  (store r#1 @31..44\n\
                 \x20   (update .a!.b @35..40\n\
                 \x20     (int 2 @43..44)\n\
                 \x20     (load r#0 @35..36)))"
            ),
            "{shown}"
        );
    }

    #[test]
    fn the_shared_nodes_of_a_dotted_fields_chain_do_not_disturb_depths() {
        // `a.b.d = g(a.b.c)`, extending `a`: the block (0), its item's block
        // (1), the store (2), the update of `b` (3), the update of `d` (4),
        // the call (5). The update of `b` shares its load of `a` with the
        // access `a.b` that the update of `d` builds on, so the walk
        // reaches that load twice; the call's depth is its own path's.
        let ir = lower_text("fn g(n: Int): Int { n }\n{ a.b.c = 1; a.b.d = g(a.b.c) }");
        assert_eq!(depths(&ir, ir.root()), vec![5]);
    }

    #[test]
    fn an_index_path_lowers_to_one_update_of_its_list() {
        // The value, the list and each position, in the order they are
        // evaluated, and a step for each segment after the first.
        assert_eq!(
            dump("let mut xs = [1, 2]\nlet i = 0\nxs[i + 1] := 5"),
            "(block @0..44\n\
             \x20 (store xs#0 @0..19\n\
             \x20   (make-list @13..19\n\
             \x20     (int 1 @14..15)\n\
             \x20     (int 2 @17..18)))\n\
             \x20 (store i#1 @20..29\n\
             \x20   (int 0 @28..29))\n\
             \x20 (store xs#0 @30..44\n\
             \x20   (update [] @30..39\n\
             \x20     (int 5 @43..44)\n\
             \x20     (load xs#0 @30..32)\n\
             \x20     (int-add @33..38\n\
             \x20       (load i#1 @33..34)\n\
             \x20       (int 1 @37..38))))\n\
             \x20 (empty @0..44))"
        );
        // Names and indexes mixed, two in a row, and a record updated: the
        // update is of the kind of what it updates.
        let ir = lower_text(
            "let mut conf = { hosts = [[{ up = false }]] }\nconf.hosts[0][1].up := true",
        );
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, update) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        let IrKind::Update {
            parts,
            path,
            kind: ValueKind::Record,
            ..
        } = ir.get(update).kind
        else {
            panic!("expected an update of a record");
        };
        let parts: Vec<_> = ir.items(parts).iter().map(|&id| ir.get(id).kind).collect();
        assert_eq!(
            parts,
            vec![
                IrKind::BoolConst(true),
                IrKind::Load(Slot::new(0), ValueKind::Record),
                IrKind::IntConst(0),
                IrKind::IntConst(1)
            ]
        );
        let steps: Vec<_> = ir
            .update_path(path)
            .unwrap()
            .iter()
            .map(|step| (matches!(step.at, StepAt::Element), step.mode, step.span))
            .collect();
        assert_eq!(
            steps,
            vec![
                (false, StepMode::Read, Span::new(46, 56)),
                (true, StepMode::Read, Span::new(46, 59)),
                (true, StepMode::Read, Span::new(46, 62)),
                (false, StepMode::Set, Span::new(46, 65))
            ]
        );
        // A list updated is of kind `List`, and so is its read.
        let shown = dump("let xs = [[1]]\nxs[0][0] := 2");
        assert!(
            shown.contains(
                "  (store xs#1 @15..28\n\
                 \x20   (update [][] @15..23\n\
                 \x20     (int 2 @27..28)\n\
                 \x20     (load xs#0 @15..17)\n\
                 \x20     (int 0 @18..19)\n\
                 \x20     (int 0 @21..22)))"
            ),
            "{shown}"
        );
        let ir = lower_text("let xs = [[1]]\nxs[0][0] := 2");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, update) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert_eq!(ir.kind_of(update), ValueKind::List);
    }

    #[test]
    fn an_index_path_awaits_the_lists_and_elements_that_may_be_futures() {
        // An impure function's parameter and its elements, the either shape:
        // the list read is awaited, as a list, and so is the element its
        // step reads; the position too, where it is used.
        let text = "fn f(xs: List<{a: Int}>, i: Int) -> {} { xs[i].a := 1 }";
        let shown = dump(text);
        assert!(
            shown.contains(
                "    (store xs#2 @41..53\n\
                 \x20     (update []!.a @41..48\n\
                 \x20       (int 1 @52..53)\n\
                 \x20       (await @41..43\n\
                 \x20         (load xs#0 @41..43))\n\
                 \x20       (await @44..45\n\
                 \x20         (load i#1 @44..45))))"
            ),
            "{shown}"
        );
        let ir = lower_text(text);
        let code = ir.function(FnId::new(0));
        let IrKind::Block { items, .. } = ir.get(code.body).kind else {
            panic!("expected a block");
        };
        let IrKind::Store(_, update) = ir.get(ir.items(items)[0]).kind else {
            panic!("expected a store");
        };
        let IrKind::Update { parts, .. } = ir.get(update).kind else {
            panic!("expected an update");
        };
        let IrKind::Await { kind, value, .. } = ir.get(ir.items(parts)[1]).kind else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::List);
        assert_eq!(
            ir.get(value).kind,
            IrKind::Load(Slot::new(0), ValueKind::Deferred)
        );
        // The update gets its depth, for its steps' awaits: the block (0),
        // the store (1), the update (2); and the awaits below it (3).
        let IrKind::Update { depth, .. } = ir.get(update).kind else {
            panic!("expected an update");
        };
        assert_eq!(depth, 2);
        assert_eq!(
            ir.awaits_in(code.body)
                .into_iter()
                .map(|(depth, _)| depth)
                .collect::<Vec<_>>(),
            vec![3, 3]
        );
        // Two levels for each of its two steps: the update is 1 + 2 + 4
        // tall, the store 8 and the block 9.
        assert_eq!(code.height, 9);
    }

    /// The result of the root block of `ir`, whatever items come before it.
    fn result_after_items(ir: &Ir) -> IrId {
        match ir.get(ir.root()).kind {
            IrKind::Block { result, .. } => result,
            other => panic!("the root is not a block: {other:?}"),
        }
    }

    // ----- Lists -----

    #[test]
    fn a_list_literal_makes_a_list_of_its_elements() {
        assert_eq!(
            dump("[1, lazy 2, ...[3]]"),
            "(block @0..19\n\
             \x20 (make-list @0..19\n\
             \x20   (int 1 @1..2)\n\
             \x20   (make-future #0 @4..10)\n\
             \x20   (spread @12..18\n\
             \x20     (make-list @15..18\n\
             \x20       (int 3 @16..17)))))\n\
             future #0 (frame 0, params 0, captures 0, height 1)\n\
             \x20 (int 2 @9..10)"
        );
        let ir = lower_text("[1, 2]");
        assert!(matches!(
            ir.get(result_of(&ir)).kind,
            IrKind::MakeList { spreads: false, .. }
        ));
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::List);
        let ir = lower_text("[...[1]]");
        assert!(matches!(
            ir.get(result_of(&ir)).kind,
            IrKind::MakeList { spreads: true, .. }
        ));
        // An element awaited to fit its expected type is wrapped in its
        // await, and a spread's list that may be a future too.
        let shown = dump("let xs: List<Present<Int>> = [lazy 1]\nlet ys = lazy [2]\n[...ys]");
        assert!(
            shown.contains(
                "(make-list @29..37\n\
                 \x20     (await @30..36\n\
                 \x20       (make-future #0 @30..36)))"
            ),
            "{shown}"
        );
        assert!(
            shown.contains(
                "(spread @57..62\n\
                 \x20     (await @60..62\n\
                 \x20       (load ys#1 @60..62)))"
            ),
            "{shown}"
        );
    }

    #[test]
    fn an_index_reads_an_element_of_its_kind() {
        assert_eq!(
            dump("let xs = [1.5]\nxs[0] + 1.0"),
            "(block @0..26\n\
             \x20 (store xs#0 @0..14\n\
             \x20   (make-list @9..14\n\
             \x20     (float 1.5 @10..13)))\n\
             \x20 (float-add @15..26\n\
             \x20   (index @15..20\n\
             \x20     (load xs#0 @15..17)\n\
             \x20     (int 0 @18..19))\n\
             \x20   (float 1.0 @23..26)))"
        );
        // An element that may be a future is read as one and awaited where
        // it is used.
        let ir = lower_text("let xs = [lazy 1]\nxs[0]");
        let index = result_after_items(&ir);
        assert!(matches!(
            ir.get(index).kind,
            IrKind::Index {
                kind: ValueKind::Deferred,
                ..
            }
        ));
        let shown = dump("let xs = [1, lazy 2]\nxs[1] * 2");
        assert!(
            shown.contains(
                "(int-mul @21..30\n\
                 \x20   (await @21..26\n\
                 \x20     (index @21..26\n"
            ),
            "{shown}"
        );
    }

    #[test]
    fn len_lowers_to_the_length_of_its_list() {
        assert_eq!(
            dump_prelude("len(list = [1])"),
            "(block @0..15\n\
             \x20 (list-len @0..15\n\
             \x20   (make-list @11..14\n\
             \x20     (int 1 @12..13))))"
        );
        let (_, ir) = lower_prelude("len([1])");
        assert_eq!(ir.kind_of(result_of(&ir)), ValueKind::Int);
        // Its list is awaited if it may be a future.
        let shown = dump_prelude("let xs = lazy [1]\nlen(xs)");
        assert!(
            shown.contains("(list-len @18..25\n    (await @22..24\n"),
            "{shown}"
        );
    }

    #[test]
    fn exec_and_panic_lower_to_calls() {
        // Ordinary built-ins: a `Call` of the value of their cell.
        let (_, ir) = lower_prelude("exec([\"true\"])");
        let call = result_of(&ir);
        let IrKind::Call { kind, .. } = ir.get(call).kind else {
            panic!("not a call: {:?}", ir.get(call));
        };
        assert_eq!(kind, ValueKind::Record);
        // A call of `panic` is a `Call` of kind `Never`, lowered as it is:
        // no operand of it is `Never`.
        for text in ["panic(\"x\")", "let s = \"x\"\npanic(s)"] {
            let (_, ir) = lower_prelude(text);
            let call = result_of_last(&ir);
            let IrKind::Call { kind, .. } = ir.get(call).kind else {
                panic!("{text}: not a call: {:?}", ir.get(call));
            };
            assert_eq!(kind, ValueKind::Never, "{text}");
            assert_eq!(ir.kind_of(call), ValueKind::Never, "{text}");
        }
        // As a branch, it leaves the `if` the other branch's kind.
        let (_, ir) = lower_prelude("let c = true\nif c { panic(\"x\") } else { 1 }");
        assert_eq!(ir.kind_of(result_of_last(&ir)), ValueKind::Int);
        let shown = dump_prelude("let c = true\nif c { panic(\"x\") } else { 1 }");
        assert!(
            shown.contains("(block @18..32\n      (call @20..30\n        (load panic#"),
            "{shown}"
        );
    }

    /// The result of the root block of `ir`, whatever items come before it.
    fn result_of_last(ir: &Ir) -> IrId {
        let IrKind::Block { result, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block: {:?}", ir.get(ir.root()));
        };
        result
    }

    #[test]
    fn list_equality_compares_over_a_list_shape() {
        assert_eq!(
            dump("[1] == [2]"),
            "(block @0..10\n\
             \x20 (list-eq == * @0..10\n\
             \x20   (make-list @0..3\n\
             \x20     (int 1 @1..2))\n\
             \x20   (make-list @7..10\n\
             \x20     (int 2 @8..9))))"
        );
        assert!(dump("[1] !== [2]").contains("(list-same !== @0..11\n"));
        assert!(dump("[1] === [2]").contains("(list-same === @0..11\n"));
        assert!(dump("[1] != [2]").contains("(list-eq != * @0..10\n"));
        // Through records' shapes, and records' fields through lists'.
        let text = "let q: {x: Int} = { x = 1; y = 2 }\n[q] == [q]";
        assert!(dump(text).contains("(list-eq == *[x] @"), "{}", dump(text));
        let text = "let q = { xs = [[{ a = 1 }]]; n = 1 }\nq == q";
        assert!(
            dump(text).contains("(record-eq == [xs**[a] n] @"),
            "{}",
            dump(text)
        );
        // One shape per type, nested ones included.
        let ir = lower_text("let q = [[{ a = 1 }]]\nlet a = q == q\n= q[0] == q[0]");
        assert_eq!(ir.shape_count(), 3);
        let ir = lower_text("[{ a = 1 }] == [{ a = 1 }]");
        let IrKind::ListEq(ShapedOperands { shape, .. }) = ir.get(result_of(&ir)).kind else {
            panic!("expected a list comparison");
        };
        let Some(ListShape {
            element: Some(element),
        }) = ir.list_shape(shape)
        else {
            panic!("expected a list's shape");
        };
        assert_eq!(ir.list_shape(element), None);
        assert_eq!(ir.shape(element).fields.len(), 1);
        assert!(ir.shape(shape).fields.is_empty());
    }

    #[test]
    fn a_deep_await_of_a_list_awaits_each_element() {
        let text = "let xs = [1, lazy 2]\n\"{xs}\"";
        let shown = dump(text);
        assert!(
            shown.contains("(await *! @23..25\n      (load xs#0 @23..25))"),
            "{shown}"
        );
        let ir = lower_text(text);
        let IrKind::Interp(items) = ir.get(result_after_items(&ir)).kind else {
            panic!("expected an interpolation");
        };
        let IrKind::Await { shape, kind, .. } = ir.get(ir.items(items)[1]).kind else {
            panic!("expected an await");
        };
        assert_eq!(kind, ValueKind::List);
        assert_eq!(
            ir.lazy_list(shape),
            Some(LazyField::Await(LazyShapeId::NONE))
        );
        assert!(ir.lazy_shape(shape).fields.is_empty());
        assert_eq!(ir.lazy_list(LazyShapeId::NONE), None);
        // Each element's lazy values, and a record's lazy list.
        for (text, shape) in [
            ("let xs = [{ a = lazy 1 }]\n\"{xs}\"", "*[a!]"),
            ("let xs = [[1, lazy 2]]\n\"{xs}\"", "**!"),
            ("let xs = [lazy [1, lazy 2]]\n\"{xs}\"", "*!*!"),
            ("let r = { xs = [lazy 1]; n = 2 }\n\"{r}\"", "[xs*!]"),
            (
                "let xs = [{ a = lazy [{ b = lazy 1 }] }]\n\"{xs}\"",
                "*[a!*[b!]]",
            ),
        ] {
            let shown = dump(text);
            assert!(
                shown.contains(&format!("(await {shape} @")),
                "{text}: {shown}"
            );
        }
        // A list with nothing lazy inside is not awaited.
        assert!(!dump("let xs = [[1]]\n\"{xs}\"").contains("await"));
    }

    // ----- Spreads -----

    #[test]
    fn a_spread_stores_its_operand_once_and_each_field_by_name() {
        // The operand is held in the item's own slot, which no variable
        // owns, and each field of its static type, in the type's order, is
        // read from it into the field's slot. A top-level field of the
        // input, `a`, has its own slot from the start, which the spread
        // does not write; the field keeps the place the spread gave it in
        // the record, loaded as the definition that stands.
        let text = "let r = { a = 1; b = 2 }\n...r\na = \"s\"";
        assert_eq!(
            dump(text),
            "(block @0..37\n\
             \x20 (store r#1 @0..24\n\
             \x20   (block @8..24\n\
             \x20     (store r#1 @10..15\n\
             \x20       (int 1 @14..15))\n\
             \x20     (store b#2 @17..22\n\
             \x20       (int 2 @21..22))\n\
             \x20     (make-record [a b] @8..24\n\
             \x20       (load r#1 @10..11)\n\
             \x20       (load b#2 @17..18))))\n\
             \x20 (store ?#3 @25..29\n\
             \x20   (load r#1 @28..29))\n\
             \x20 (store b#2 @25..29\n\
             \x20   (field b @28..29\n\
             \x20     (load ?#3 @28..29)))\n\
             \x20 (store a#0 @30..37\n\
             \x20   (str \"s\" @34..37))\n\
             \x20 (make-record [a b] @0..37\n\
             \x20   (load a#0 @30..31)\n\
             \x20   (load b#2 @28..29)))"
        );
        let ir = lower_text(text);
        let IrKind::MakeRecord { values, .. } = ir.get(result_after_items(&ir)).kind else {
            panic!("expected a record");
        };
        let loads: Vec<IrKind> = ir.items(values).iter().map(|&id| ir.get(id).kind).collect();
        assert_eq!(
            loads,
            vec![
                IrKind::Load(Slot::new(0), ValueKind::String),
                IrKind::Load(Slot::new(2), ValueKind::Int)
            ]
        );
        // A later spread overrides a soft field in its slot too.
        let ir = lower_text("{ ...{ a = 1; b = 2 }; ...{ b = \"s\"; c = 3 } }");
        let IrKind::Block { result, .. } = ir.get(result_after_items(&ir)).kind else {
            panic!("expected a block");
        };
        let IrKind::MakeRecord { layout, values } = ir.get(result).kind else {
            panic!("expected a record");
        };
        let names: Vec<&str> = ir.layout(layout).texts.iter().map(AsRef::as_ref).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        let loads: Vec<IrKind> = ir.items(values).iter().map(|&id| ir.get(id).kind).collect();
        assert_eq!(
            loads,
            vec![
                IrKind::Load(Slot::new(0), ValueKind::Int),
                IrKind::Load(Slot::new(1), ValueKind::String),
                IrKind::Load(Slot::new(2), ValueKind::Int)
            ]
        );
    }

    #[test]
    fn a_spreads_operand_that_may_be_a_future_is_awaited_once() {
        assert_eq!(
            dump("let f = lazy { a = 1 }\n...f")
                .lines()
                .collect::<Vec<_>>()[..11],
            [
                "(block @0..27",
                "  (store f#0 @0..22",
                "    (make-future #0 @8..22))",
                "  (store ?#2 @23..27",
                "    (await @26..27",
                "      (load f#0 @26..27)))",
                "  (store a#1 @23..27",
                "    (field a @26..27",
                "      (load ?#2 @26..27)))",
                "  (make-record [a] @0..27",
                "    (load a#1 @26..27)))",
            ]
        );
        // A field that may be a future is read as it is, not awaited.
        let ir = lower_text("...{ a = lazy 1 }");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        let IrKind::Store(_, field) = ir.get(ir.items(items)[1]).kind else {
            panic!("expected a store");
        };
        assert!(matches!(
            ir.get(field).kind,
            IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
        ));
    }

    // ----- Ranges and `for` -----

    #[test]
    fn a_range_lowers_to_a_list_of_its_bounds() {
        assert_eq!(
            dump("-1..=2"),
            "(block @0..6\n\
             \x20 (range-inclusive @0..6\n\
             \x20   (int-neg @0..2\n\
             \x20     (int 1 @1..2))\n\
             \x20   (int 2 @5..6)))"
        );
        let ir = lower_text("0..2");
        let range = result_of(&ir);
        assert_eq!(ir.kind_of(range), ValueKind::List);
        assert_eq!(ir.get(range).operator_span(), Some(Span::new(1, 3)));
        let ir = lower_text("0..=2");
        assert_eq!(
            ir.get(result_of(&ir)).operator_span(),
            Some(Span::new(1, 4))
        );
        // A bound that may be a future is awaited.
        let shown = dump("let n = lazy 2\n0..n");
        assert!(
            shown.contains(
                "(range @15..19\n\
                 \x20   (int 0 @15..16)\n\
                 \x20   (await @18..19\n\
                 \x20     (load n#0 @18..19)))"
            ),
            "{shown}"
        );
    }

    #[test]
    fn a_for_lowers_to_its_slot_and_parts() {
        assert_eq!(
            dump("let xs = [1]\nfor x in xs if x > 0 { x * 2 }"),
            "(block @0..43\n\
             \x20 (store xs#0 @0..12\n\
             \x20   (make-list @9..12\n\
             \x20     (int 1 @10..11)))\n\
             \x20 (for x#1 if @13..43\n\
             \x20   (load xs#0 @22..24)\n\
             \x20   (int-cmp > @28..33\n\
             \x20     (load x#1 @28..29)\n\
             \x20     (int 0 @32..33))\n\
             \x20   (block @34..43\n\
             \x20     (int-mul @36..41\n\
             \x20       (load x#1 @36..37)\n\
             \x20       (int 2 @40..41)))))"
        );
        let ir = lower_text("for x in [1] { x }");
        let for_node = result_of(&ir);
        let IrKind::For {
            slot,
            parts,
            collect,
        } = ir.get(for_node).kind
        else {
            panic!("expected a for, got {:?}", ir.get(for_node));
        };
        assert_eq!(slot, Slot::new(0));
        assert_eq!(ir.items(parts).len(), 2);
        assert!(collect);
        assert_eq!(ir.kind_of(for_node), ValueKind::List);
        assert_eq!(ir.get(for_node).operator_span(), None);
        // The iterable is a range node as it is, through parentheses.
        let ir = lower_text("for i in (0..3) { i }");
        let IrKind::For { parts, .. } = ir.get(result_of(&ir)).kind else {
            panic!("expected a for");
        };
        assert!(matches!(
            ir.get(ir.items(parts)[0]).kind,
            IrKind::Range(false, _)
        ));
        // An iterable that may be a future is awaited once, and an element
        // is read as it is and awaited where it is used.
        let shown = dump("let xs = lazy [lazy 1]\nfor x in xs { x + 1 }");
        assert!(
            shown.contains(
                "(for x#1 @23..44\n\
                 \x20   (await @32..34\n\
                 \x20     (load xs#0 @32..34))\n\
                 \x20   (block @35..44\n\
                 \x20     (int-add @37..42\n\
                 \x20       (await @37..38\n\
                 \x20         (load x#1 @37..38))"
            ),
            "{shown}"
        );
    }

    #[test]
    fn a_body_awaited_to_fit_is_awaited_in_the_for() {
        // (The variable's slot is named by `ys`, the last variable stored
        // in it, as the slot of any variable of a finished block is.)
        let shown = dump("let xs = [lazy 1]\nlet ys: List<Present<Int>> = for x in xs { x }\nys");
        assert!(
            shown.contains(
                "(for ys#1 @47..64\n\
                 \x20     (load xs#0 @56..58)\n\
                 \x20     (await @59..64\n\
                 \x20       (block @59..64\n\
                 \x20         (load ys#1 @61..62))))"
            ),
            "{shown}"
        );
    }

    #[test]
    fn a_unit_like_for_statement_builds_no_list() {
        // As it is: a `for` that discards its body's values and is `{}`.
        let ir = lower_text("for x in [1] { }\nfor y in [2] { for z in [3] { } }\n1");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        for &item in ir.items(items) {
            assert!(
                matches!(ir.get(item).kind, IrKind::For { collect: false, .. }),
                "{:?}",
                ir.get(item)
            );
            assert_eq!(ir.kind_of(item), ValueKind::Record);
        }
        assert_eq!(
            dump("(for x in [1] { })\n1"),
            "(block @0..20\n\
             \x20 (for-each x#0 @1..17\n\
             \x20   (make-list @10..13\n\
             \x20     (int 1 @11..12))\n\
             \x20   (block @14..17\n\
             \x20     (empty @14..17)))\n\
             \x20 (int 1 @19..20))"
        );
        // Unit-like only once awaited: the list is built and awaited
        // deeply.
        let shown = dump("let steps = [lazy {}]\nfor s in steps { s }\n1");
        assert!(
            shown.contains(
                "(await *! @22..42\n\
                 \x20   (for s#1 @22..42\n"
            ),
            "{shown}"
        );
        // Any other unit-like statement is evaluated and dropped, as it is.
        let ir = lower_text("[]\n[{}]\n1");
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        for &item in ir.items(items) {
            assert!(matches!(ir.get(item).kind, IrKind::MakeList { .. }));
        }
        // A `for` that is its block's result builds its list.
        let ir = lower_text("for x in [1] { }");
        assert!(matches!(
            ir.get(result_of(&ir)).kind,
            IrKind::For { collect: true, .. }
        ));
    }

    #[test]
    fn a_for_is_one_level_above_its_parts() {
        let g = "fn g(n: Int): Int { n }\n";
        // In a function's body: the block (0), the `for` (1), its body (2),
        // the call (3); and the body's height: the block, the `for`, its
        // body, the call, its argument, the load.
        let ir = lower_text(&format!(
            "{g}fn f(xs: Int[]): Int[] {{ for x in xs {{ g(x) }} }}"
        ));
        let f = ir.function(FnId::new(1));
        assert_eq!(depths(&ir, f.body), vec![3]);
        assert_eq!(f.height, 6);
        // In the iterable and the guard: the block (0), the `for` (1), the
        // list (2) and the guard's call (2).
        let ir = lower_text(&format!("{g}for x in [g(1)] if g(x) > 0 {{ }}"));
        assert_eq!(depths(&ir, ir.root()), vec![3, 3]);
        // Nested, each `for` and its body are a level each.
        let ir = lower_text(&format!("{g}for x in [1] {{ for y in [2] {{ g(y) }} }}"));
        assert_eq!(depths(&ir, ir.root()), vec![5]);
    }

    /// The result of the root block of `ir`, items or no items.
    fn root_result(ir: &Ir) -> IrId {
        let IrKind::Block { result, .. } = ir.get(ir.root()).kind else {
            panic!("the root is not a block");
        };
        result
    }

    #[test]
    fn a_union_is_of_mixed_kind_unless_its_members_agree() {
        let ir = lower_text("let v = if true { 1 } else { \"one\" }\nv");
        let result = root_result(&ir);
        assert_eq!(
            ir.get(result).kind,
            IrKind::Load(Slot::new(0), ValueKind::Mixed)
        );
        // The `if` holds branches of each member's kind.
        let dumped = dump("let v = if true { 1 } else { \"one\" }\nv");
        assert!(dumped.contains("(int 1 @"), "{dumped}");
        assert!(dumped.contains("(str \"one\" @"), "{dumped}");
        // A union of lists is a list.
        let ir = lower_text("let xs: List<Int> | List<String> = [1]\nxs");
        let result = root_result(&ir);
        assert_eq!(
            ir.get(result).kind,
            IrKind::Load(Slot::new(0), ValueKind::List)
        );
        // Total equality on values of a union of kinds.
        let dumped = dump("let v: Int | String = 1\nv === v");
        assert!(dumped.contains("(value-same === @24..31"), "{dumped}");
        let dumped = dump("let v: Int | String = 1\nv !== v");
        assert!(dumped.contains("(value-same !== @24..31"), "{dumped}");
    }

    #[test]
    fn a_test_shows_its_plan() {
        // Only what the value's type leaves in doubt is looked at.
        for (text, plan) in [
            ("let v: Int | List<Int> = 1\nv is Int", "(is int @"),
            ("let v: Int | List<Int> = 1\nv is List<Int>", "(is list @"),
            (
                "let v: List<Int | String> = []\nv is List<Int>",
                "(is [int] @",
            ),
            (
                "let v: {a: Int | String, b: Int} | Int = 1\nv is {a: Int, b: Int}",
                "(is {a: int} @",
            ),
            (
                "let v: Int | String | Bool = 1\nv is Int | String",
                "(is (int | string) @",
            ),
            ("let v: Int | String = 1\nv as Int", "(as int @"),
        ] {
            let dumped = dump(text);
            assert!(dumped.contains(plan), "{text}: {dumped}");
        }
        // An `as` is of the kind of what it narrows to.
        let ir = lower_text("let v: Int | String = 1\nv as Int");
        let result = root_result(&ir);
        assert_eq!(ir.kind_of(result), ValueKind::Int);
        // A narrowing `is` writes its slot, which its branches read.
        let dumped = dump("let v: Int | String = 1\nif v is Int { v } else { 0 }");
        assert!(
            dumped.contains(
                "(is int v#1 @27..35\n\
                 \x20     (load v#0 @27..28))\n\
                 \x20   (block @36..41\n\
                 \x20     (load v#1 @38..39))"
            ),
            "{dumped}"
        );
    }

    #[test]
    fn a_unions_deep_await_awaits_each_members_shape() {
        let dumped = dump("let u: Int | {a: Int | Future<Int>} | List<Future<Int>> = 1\n\"{u}\"");
        assert!(dumped.contains("(await ([a!] | *!) @"), "{dumped}");
        // A union with nothing lazy inside is not awaited deeply.
        let dumped = dump("let u: Int | {a: Int} = 1\n\"{u}\"");
        assert!(!dumped.contains("(await"), "{dumped}");
    }

    #[test]
    fn a_test_the_checker_did_not_plan_is_an_internal_error() {
        let ast = parse_text("let v: Int | String = 1\nv is Int");
        let mut types = TypeTable::new();
        let (checked, _) = check(&ast, &mut types).unwrap();
        let unplanned = Checked {
            types: checked.types.clone(),
            resolution: Resolution::without_variables(ast.len(), ast.item_count()),
        };
        let error = lower(&ast, &unplanned, &types, 0).unwrap_err();
        assert!(
            matches!(
                error.message,
                "an `is` or `as` has no test" | "a declaration has no slot"
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_for_without_its_slot_is_an_internal_error() {
        let ast = parse_text("for x in [1] { x }");
        let mut types = TypeTable::new();
        let (mut checked, _) = check(&ast, &mut types).unwrap();
        checked.resolution.forget_for_slots();
        let error = lower(&ast, &checked, &types, 0).unwrap_err();
        assert_eq!(error.message, "a `for` has no slot for its variable");
    }

    #[test]
    fn a_condition_loop_lowers_to_a_loop_in_a_block_of_its_header() {
        assert_eq!(
            dump("for let i = 0; i < 2; i := i + 1 { i }"),
            "(block @0..38\n\
             \x20 (block @0..38\n\
             \x20   (store i#0 @4..13\n\
             \x20     (int 0 @12..13))\n\
             \x20   (loop @0..38\n\
             \x20     (int-cmp < @15..20\n\
             \x20       (load i#0 @15..16)\n\
             \x20       (int 2 @19..20))\n\
             \x20     (block @33..38\n\
             \x20       (load i#0 @35..36))\n\
             \x20     (store i#0 @22..32\n\
             \x20       (int-add @27..32\n\
             \x20         (load i#0 @27..28)\n\
             \x20         (int 1 @31..32))))))"
        );
        // `for { }` tests `true`, at its keyword; a `do` tests after.
        assert_eq!(
            dump("for { break 1 }"),
            "(block @0..15\n\
             \x20 (loop @0..15\n\
             \x20   (bool true @0..3)\n\
             \x20   (block @4..15\n\
             \x20     (break 0 @6..13\n\
             \x20       (int 1 @12..13)))))"
        );
        assert_eq!(
            dump("do { continue } for false"),
            "(block @0..25\n\
             \x20 (do @0..25\n\
             \x20   (bool false @20..25)\n\
             \x20   (block @3..15\n\
             \x20     (continue 0 @5..13))))"
        );
        let ir = lower_text("do { 1 } for false");
        let node = result_of(&ir);
        assert_eq!(ir.kind_of(node), ValueKind::List);
        assert_eq!(ir.get(node).operator_span(), None);
    }

    #[test]
    fn a_stop_counts_the_loops_it_leaves() {
        let shown = dump("for'o x in [1] { for y in [2] { if y > x { break'o }; continue } }");
        assert!(shown.contains("(break 1 @"), "{shown}");
        assert!(shown.contains("(continue 0 @"), "{shown}");
        // A function's body starts with no loop around it.
        let shown =
            dump("for x in [1] { let f = fn(): List<Int> { for y in [2] { break y } }; x }");
        assert!(shown.contains("(break 0 @"), "{shown}");
        // A stop is of kind Never, and so is a block ending in one, and an
        // `if` is of the kind of its branch that is not.
        let ir = lower_text("for x in [1] { if x > 0 { break } else { 5 } }");
        let IrKind::For { parts, .. } = ir.get(result_of(&ir)).kind else {
            panic!("expected a for");
        };
        let body = ir.items(parts)[1];
        assert_eq!(ir.kind_of(body), ValueKind::Int);
        let IrKind::Block { result, .. } = ir.get(body).kind else {
            panic!("expected a block");
        };
        let IrKind::If { then, .. } = ir.get(result).kind else {
            panic!("expected an if");
        };
        assert_eq!(ir.kind_of(then), ValueKind::Never);
    }

    #[test]
    fn a_never_operation_lowers_to_its_operands_up_to_the_one_that_stops() {
        // The left operand runs, as a statement, then the right one, which
        // stops; the addition is never made. (`x`'s slot is named by `v`,
        // the last variable stored in it.)
        let shown = dump("let mut n = 0\nlet v = for x in [1] { ({ n := 1; x }) + { break } }");
        assert!(
            shown.contains(
                "(block @37..64\n\
                 \x20         (block @38..51\n\
                 \x20           (store n#0 @40..46\n\
                 \x20             (int 1 @45..46))\n\
                 \x20           (load v#1 @48..49))\n\
                 \x20         (block @55..64\n\
                 \x20           (break 0 @57..62)))"
            ),
            "{shown}"
        );
        // A function in what never runs is still lowered.
        let ir = lower_text("let v = for x in [1] { { break } + (fn(): Int { 1 })() }\nv");
        assert_eq!(ir.function_count(), 1);
        // A loop over a value of type Never is its iterable.
        let shown = dump("let v = for x in [1] { for y in { break } { y } }\nv");
        assert!(!shown.contains("(for y"), "{shown}");
    }

    #[test]
    fn a_return_names_what_it_leaves() {
        // A function's or a `lazy`'s, by `fn`.
        let shown = dump("fn f(n: Int): Int { if n > 0 { return n }; 0 }\nf(1)");
        assert!(shown.contains("(return fn @31..39\n"), "{shown}");
        let shown = dump("let l = lazy { return 1 }\nawait l");
        assert!(shown.contains("(return fn @15..23\n"), "{shown}");
        // A labelled block's, by the number of its catch, around it; the
        // block keeps its own kind, the catch takes the join's.
        let ir = lower_text("let c = true\n{'b if c { return'b 1 }; \"s\" }");
        let IrKind::Catch { body, id, kind } = ir.get(result_after_items(&ir)).kind else {
            panic!("expected a catch: {:?}", ir.get(result_after_items(&ir)));
        };
        assert_eq!(id, CatchId::new(0));
        assert_eq!(kind, ValueKind::Mixed);
        assert_eq!(ir.kind_of(body), ValueKind::String);
        let shown = dump("let c = true\n{'b if c { return'b 1 }; \"s\" }");
        assert!(
            shown.contains("(catch 0 @13..43\n    (block @13..43\n"),
            "{shown}"
        );
        assert!(shown.contains("(return 0 @24..34\n"), "{shown}");
        // A labelled block no `return` leaves is its block, as any.
        let shown = dump("{'b 1 }");
        assert!(!shown.contains("catch"), "{shown}");
        // Nested catches are numbered as they start, and a `return` finds
        // its own.
        let shown = dump("{'a {'b if true { return'a 1 }; return'b 2 } }");
        assert!(shown.contains("(catch 0 @"), "{shown}");
        assert!(shown.contains("(catch 1 @"), "{shown}");
        assert!(shown.contains("(return 0 @"), "{shown}");
        assert!(shown.contains("(return 1 @"), "{shown}");
        // A loop's, around the loop, which keeps collecting.
        let shown = dump("let xs = [1, 7]\nlet v = for'l x in xs { if x > 5 { return'l x }; x }");
        // (`x`'s slot is named by `v`, the last variable stored in it.)
        assert!(
            shown.contains("(catch 0 @24..68\n      (for v#1 @24..68\n"),
            "{shown}"
        );
        assert!(shown.contains("(return 0 @51..61\n"), "{shown}");
    }

    #[test]
    fn a_loop_that_cannot_end_collects_nothing() {
        // Whatever its body's type.
        let shown = dump("let v: Int = for'spin { if true { return'spin 1 }; 5 }\nv");
        assert!(shown.contains("(loop-each @13..54\n"), "{shown}");
        assert!(shown.contains("(catch 0 @13..54\n"), "{shown}");
        // As a statement, and of type Never where an `Int` is wanted.
        let shown = dump("let v = for'o x in [1] { let y: Int = 1 + for { break'o }; y }\nv");
        assert!(shown.contains("(loop-each @"), "{shown}");
        assert!(!shown.contains("catch"), "{shown}");
        // A loop that can end keeps its list.
        let shown = dump("let v = for { break 1 }\nv");
        assert!(shown.contains("(loop @"), "{shown}");
    }

    #[test]
    fn a_statement_of_a_unit_like_union_is_lowered_as_it_is() {
        let ir = lower_text("fn f() -> {} { for'l x in [1] { if x > 0 { return'l } }; {} }\nf()");
        let Some(code) = ir.find_function(FnId::new(0)) else {
            panic!("expected a function");
        };
        let IrKind::Block { items, .. } = ir.get(code.body).kind else {
            panic!("expected a block");
        };
        let [statement] = ir.items(items) else {
            panic!("expected one statement");
        };
        let IrKind::Catch { body, kind, .. } = ir.get(*statement).kind else {
            panic!("expected a catch: {:?}", ir.get(*statement));
        };
        assert_eq!(kind, ValueKind::Mixed);
        // The statement loop discards its values.
        assert!(matches!(
            ir.get(body).kind,
            IrKind::For { collect: false, .. }
        ));
    }

    /// Everything the checker and lowering decide about `text` that does
    /// not depend on where its items are written: the file's type; each
    /// diagnostic's severity and message, sorted; and the lines of the IR
    /// dump with spans, slot numbers and function numbers removed, the
    /// names of a record's fields sorted, and the lines sorted, so that
    /// neither slots nor item order count.
    fn decided(text: &str) -> (String, Vec<String>, Vec<String>) {
        decided_by(text, |text| {
            let ast = parse_text(text);
            let mut types = TypeTable::new();
            let output = crate::check::Checker::new(&ast, &mut types).check();
            (ast, types, output)
        })
    }

    /// What a check of a text gives: its tree, its types, and the outcome.
    type CheckedText = (
        Ast,
        TypeTable,
        Result<crate::check::CheckOutput, Vec<crate::diagnostic::Diagnostic>>,
    );

    /// As [`decided`], `text` checked by `check`.
    fn decided_by(
        text: &str,
        check: fn(&str) -> CheckedText,
    ) -> (String, Vec<String>, Vec<String>) {
        let (ast, types, output) = check(text);
        let output = match output {
            Ok(output) => output,
            Err(diagnostics) => {
                let mut messages: Vec<String> = diagnostics
                    .iter()
                    .map(|d| format!("{:?}: {}", d.severity, d.message))
                    .collect();
                messages.sort();
                return (String::new(), messages, Vec::new());
            }
        };
        let mut messages: Vec<String> = output
            .warnings
            .iter()
            .map(|d| format!("{:?}: {}", d.severity, d.message))
            .collect();
        messages.sort();
        let shown = types.display(output.ty, ast.names()).to_string();
        let ir = lower(&ast, &output.checked, &types, 0).unwrap();
        let mut lines: Vec<String> = ir
            .pretty(ast.names())
            .to_string()
            .lines()
            .map(normalised)
            .collect();
        lines.sort();
        (shown, messages, lines)
    }

    /// A line of an IR dump without its span, the numbers after each `#`,
    /// and with the names in its `[...]` sorted.
    fn normalised(line: &str) -> String {
        let line = line.split(" @").next().unwrap_or(line);
        let mut out = String::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(c);
            if c == '#' {
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
            }
        }
        if let (Some(open), Some(close)) = (out.find('['), out.find(']')) {
            let mut names: Vec<&str> = out[open + 1..close].split(' ').collect();
            names.sort_unstable();
            out = format!("{}[{}]{}", &out[..open], names.join(" "), &out[close + 1..]);
        }
        out
    }

    /// Asserts that `parts`, joined by newlines in the order given and in
    /// the reverse order, decide the same: a program and its reordering
    /// checked against each other. Returns what they decide.
    fn order_free(parts: &[&str]) -> (String, Vec<String>, Vec<String>) {
        let forward = parts.join("\n");
        let backward: Vec<&str> = parts.iter().rev().copied().collect();
        let backward = backward.join("\n");
        let first = decided(&forward);
        assert_eq!(first, decided(&backward), "{forward}\n----\n{backward}");
        first
    }

    /// As [`order_free`], each order checked as a lone file is, beside the
    /// standard library, so that the prelude's names are its.
    fn order_free_beside_prelude(parts: &[&str]) -> (String, Vec<String>, Vec<String>) {
        let forward = parts.join("\n");
        let backward: Vec<&str> = parts.iter().rev().copied().collect();
        let backward = backward.join("\n");
        let check = crate::workspace::check_beside_prelude;
        let first = decided_by(&forward, check);
        assert_eq!(
            first,
            decided_by(&backward, check),
            "{forward}\n----\n{backward}"
        );
        first
    }

    #[test]
    fn the_prelude_s_names_do_not_depend_on_order() {
        // Its names read anywhere, shadowed or not, as the items fall.
        let (shown, messages, lines) = order_free_beside_prelude(&[
            "n = len(xs) + count(xs)",
            "xs = [1, 2, 3]",
            "fn count(v: Int[]): Int { len(v) }",
            "h = half(n)",
            "def float(x: Int): Float { if x > 0 { 0.5 } else { 1.5 } }",
            "f = float(2)",
            "i = int(2.5)",
        ]);
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(
            shown,
            "{f: Float, float: fn(x: Int): Float, h: Float, i: Int, n: Int, xs: List<Int>}"
        );
        assert!(
            lines.iter().any(|line| line.contains("list-len")),
            "{lines:?}"
        );
        // The file's own `float` shadows the prelude's silently: it is 0.5.
        let text = "def float(x: Int): Float { if x > 0 { 0.5 } else { 1.5 } }\nf = float(2)";
        let f = crate::run(&SourceFile::new("t.gcsl", text).unwrap());
        assert_eq!(
            f.unwrap().value.to_string(),
            "{float = fn(x: Int): Float; f = 0.5}"
        );
    }

    #[test]
    fn reads_of_shadowing_declarations_do_not_depend_on_order() {
        // A read above, below or in a function sees the last of a run of
        // shadowable declarations, each run kept in its order as one part.
        let (shown, messages, lines) = order_free(&[
            "x = a + 1",
            "#[shadowable] a = 1\na = 2",
            "fn get(): Int { a }",
            "y = get()",
            "#[shadowable] d = {j = 4}\nd.j = 8",
            "z = d.j",
        ]);
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(shown, "{a: Int, d: {j: Int}, x: Int, y: Int, z: Int}");
        // The shadowed declarations are lowered to nothing: no `4`.
        assert!(
            !lines.iter().any(|line| line.contains("int 4")),
            "{lines:?}"
        );
        // And beside the prelude, whose names a dotted root shadows.
        let (shown, messages, _) =
            order_free_beside_prelude(&["n = len.x", "len.x = 1", "m = trim(\" a \")"]);
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(shown, "{len: {x: Int}, m: String, n: Int}");
    }

    #[test]
    fn the_reordering_helper_sees_a_difference() {
        // A field's position decides nothing it compares, but a `let`'s
        // does: below its user it is undefined.
        assert_eq!(
            normalised("(make-record [x run show] @0..75"),
            "(make-record [run show x]"
        );
        assert_eq!(normalised("  (fn top even#12 @51..55)"), "  (fn top even#");
        assert_ne!(decided("let k = 1\nx = k"), decided("x = k\nlet k = 1"));
    }

    /// Asserts that `parts`, joined by newlines in the order given and in
    /// the reverse order, check to the same type, warnings and errors,
    /// whose messages it returns with the type.
    fn checks_order_free(parts: &[&str]) -> (String, Vec<String>) {
        let forward = parts.join("\n");
        let backward: Vec<&str> = parts.iter().rev().copied().collect();
        let backward = backward.join("\n");
        let (shown, messages, _) = decided(&forward);
        let (again, also, _) = decided(&backward);
        assert_eq!(
            (&shown, &messages),
            (&again, &also),
            "{forward}\n----\n{backward}"
        );
        (shown, messages)
    }

    #[test]
    fn top_level_fields_do_not_depend_on_order() {
        // Valid programs: the same type and, normalised, the same IR.
        let (shown, messages, _) = order_free(&[
            "com.x.app.deps = [com.x.lib]",
            "com.x.app.name = \"app-{version}\"",
            "com.x.lib.name = \"lib\"",
            "version = describe(2)",
            "fn describe(n: Int): String { \"v{n}\" }",
        ]);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(shown.contains("version: String"), "{shown}");
        for parts in [
            &["a.b = 1", "c = a.b + a.d", "a.d = 2"][..],
            &["x = f()", "def f(): Int { y }", "y = 3"][..],
            &["s = lazy { t }", "t.u = 1", "t.v = [t.u]"][..],
        ] {
            let (_, messages, _) = order_free(parts);
            assert!(messages.is_empty(), "{parts:?}: {messages:?}");
        }
        for (parts, expected) in [
            (
                &["a = f()", "fn f(): Int { a }"][..],
                "Error: `a` depends on itself",
            ),
            (
                &["a.b = 1", "a.b = 2"][..],
                "Error: field `a.b` is defined twice",
            ),
            (
                &["a = 1", "let a = 2"][..],
                "Error: `a` is a field of this file",
            ),
            (&["...{a = {b = 1}}", "a.c = 2"][..], ""),
        ] {
            let (_, messages) = checks_order_free(parts);
            assert_eq!(messages.len(), 1, "{parts:?}: {messages:?}");
            assert!(
                expected.is_empty() || messages.first().map(String::as_str) == Some(expected),
                "{parts:?}: {messages:?}"
            );
        }
    }

    #[test]
    fn inferred_parameters_do_not_depend_on_order() {
        // `show` leaves its parameter provisional; `add` measures it.
        let (shown, messages, _) = order_free(&[
            "def show(n: Int): Int { n }",
            "def add(n: Int): Int { n + 1 }",
            "def run(n: Int): Int { add(show(n)) }",
            "x = run(lazy 1)",
        ]);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(shown.contains("x: Int"), "{shown}");
        // The callee written below its caller, and above it.
        let used = "def pass(n: Int): Int { n }";
        let client = "def use(n: Int): Int { pass(n) }";
        let a = order_free(&[used, client, "x = use(lazy 2)"]);
        let b = order_free(&[client, used, "x = use(lazy 2)"]);
        assert_eq!(a, b);
    }

    #[test]
    fn inferred_results_do_not_depend_on_order() {
        // An impure function's result is inferred from its body.
        let used = "def get(n: Int) -> Int { if n == 0 { lazy 0 } else { n } }";
        let client = "def twice(n: Int) -> Int { get(n) + get(n) }";
        let a = order_free(&[used, client, "x = twice(1)"]);
        let b = order_free(&[client, used, "x = twice(1)"]);
        assert_eq!(a, b);
        assert!(a.1.is_empty(), "{:?}", a.1);
        // A pure caller of a pure callee that leaves its parameter
        // provisional: outside a group it sees the callee resolved, so
        // the call needs no `await` and is no error, in either order.
        let used = "def a(b: Int): Int { b }";
        let pure = "def total(): Int { a(1) + 1 }";
        let a = order_free(&[used, pure, "x = total()"]);
        let b = order_free(&[pure, used, "x = total()"]);
        assert_eq!(a, b);
        assert!(a.1.is_empty(), "{:?}", a.1);
    }

    #[test]
    fn narrowed_callbacks_do_not_depend_on_order() {
        // `apply`'s callback parameter is narrowed by its body, so `run`
        // may pass it the pure `double`, wherever the three are written.
        let host = "def apply(f: fn(Int) -> Int) -> Int { f(1) }";
        let user = "def run() -> Int { apply(double) }";
        let double = "fn double(x: Int): Int { x * 2 }";
        let a = order_free(&[double, host, user, "x = run()"]);
        let b = order_free(&[user, host, double, "x = run()"]);
        assert_eq!(a, b);
        assert!(a.1.is_empty(), "{:?}", a.1);
    }

    #[test]
    fn groups_and_their_neighbours_do_not_depend_on_order() {
        // Not a group: `a`'s parameter shadows `b`.
        order_free(&["def a(b: Int) -> Int { lazy b }", "def b() -> Int { a(1) }"]);
        // A group: the conservative view, the same from either member.
        let (shown, _, _) = order_free(&[
            "def a(n: Int) -> Int { if n == 0 { lazy 5 } else { b(n - 1) } }",
            "def b(n: Int) -> Int { a(n) }",
        ]);
        let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
        assert_eq!(shown, format!("{{a: {either}, b: {either}}}"));
        // A nested spread that defines the function's name: the mention is
        // counted (the documented over-approximation), so `f`'s body is
        // checked before the item, and the item reads the spread's field.
        let (shown, messages, _) =
            order_free(&["let r = {f = 2}\nx = { ...r; y = f }", "fn f(): Int { 1 }"]);
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(shown, "{x: {f: Int, y: Int}}");
    }

    #[test]
    fn dotted_defs_do_not_depend_on_order() {
        // The summary in one file, and mutual recursion by path.
        let (shown, messages, _) = order_free(&[
            "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }",
            "def dev.peios.count(n: Int): Int { if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) } }",
            "def dev.peios.name(): String { \"world\" }",
            "dev.peios.version = \"1\"",
            "greeting = dev.peios.hello()",
            "all = dev.peios",
            "def even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
            "def dev.odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }",
        ]);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(shown.contains("greeting: String"), "{shown}");
        // Inferred parameters, inferred results and narrowed callbacks, a
        // prefix read whole among the readers.
        let (shown, messages, _) = order_free(&[
            "def p.show(n: Int): Int { n }",
            "def p.add(n: Int): Int { n + 1 }",
            "def p.run(n: Int): Int { p.add(p.show(n)) }",
            "x = p.run(lazy 1)",
            "def q.get(n: Int) -> Int { if n == 0 { lazy 0 } else { n } }",
            "def q.twice(n: Int) -> Int { q.get(n) + q.get(n) }",
            "y = q.twice(1)",
            "whole = q",
            "def r.apply(f: fn(Int) -> Int) -> Int { f(1) }",
            "def r.run() -> Int { r.apply(r.double) }",
            "def r.double(x: Int): Int { x * 2 }",
            "z = r.run()",
        ]);
        assert!(messages.is_empty(), "{messages:?}");
        assert!(shown.contains("x: Int"), "{shown}");
        // A group by path: the conservative view, from either member.
        let (shown, _, _) = order_free(&[
            "def g.a(n: Int) -> Int { if n == 0 { lazy 5 } else { g.b(n - 1) } }",
            "def g.b(n: Int) -> Int { g.a(n) }",
        ]);
        let either = "fn(n: Int | Future<Int>) -> Int | Future<Int>";
        assert_eq!(shown, format!("{{g: {{a: {either}, b: {either}}}}}"));
    }

    #[test]
    fn a_dotted_def_s_closure_is_stored_in_its_leaf_s_cell() {
        // Made at the start, stored in its leaf's cell, which the prefix's
        // record loads; shown by its path in the dump.
        let text = "def dev.peios.count(n: Int): Int {\n\
                    \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
                    }\nall = dev.peios";
        assert_eq!(
            dump(text),
            "(block @0..108\n\
             \x20 (store count#3 @0..92\n\
             \x20   (make-closure #0 @0..92))\n\
             \x20 (store peios#2 @0..92\n\
             \x20   (make-record [count] @0..92\n\
             \x20     (load count#3 @0..92)))\n\
             \x20 (store dev#0 @0..92\n\
             \x20   (make-record [peios] @0..92\n\
             \x20     (load peios#2 @0..92)))\n\
             \x20 (store all#1 @93..108\n\
             \x20   (load peios#2 @99..108))\n\
             \x20 (make-record [dev all] @0..108\n\
             \x20   (load dev#0 @4..7)\n\
             \x20   (load all#1 @93..96)))\n\
             fn #0 dev.peios.count (frame 1, params 1, captures 0, height 8)\n\
             \x20 (block @33..92\n\
             \x20   (if @39..90\n\
             \x20     (int-cmp == @42..48\n\
             \x20       (load n#0 @42..43)\n\
             \x20       (int 0 @47..48))\n\
             \x20     (block @49..54\n\
             \x20       (int 0 @51..52))\n\
             \x20     (block @60..90\n\
             \x20       (int-add @62..88\n\
             \x20         (int 1 @62..63)\n\
             \x20         (call @66..88\n\
             \x20           (fn top count#3 @66..81)\n\
             \x20           (arg 0 @82..87\n\
             \x20             (int-sub @82..87\n\
             \x20               (load n#0 @82..83)\n\
             \x20               (int 1 @86..87))))))))"
        );
        // Copying a variable, it is made after it is bound.
        let made = dump("let k = 1\ndef a.f(): Int { k }\nx = a.f()");
        assert!(
            made.starts_with(
                "(block @0..40\n\
                 \x20 (store k#3 @0..9\n\
                 \x20   (int 1 @8..9))\n\
                 \x20 (store f#2 @10..30\n\
                 \x20   (make-closure #0 @10..30\n\
                 \x20     (load k#3 @10..30)))\n"
            ),
            "{made}"
        );
    }
}
