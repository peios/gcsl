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
//! A function expression's body is lowered into the IR's function table at its
//! [`crate::ast::FnId`], as code of [`FnKind::Memo`](crate::ir::FnKind::Memo)
//! if it is declared with `->>` and of
//! [`FnKind::Function`](crate::ir::FnKind::Function) otherwise (the IR does not
//! tell a pure function from an impure one, since only the checker cares), with
//! its frame and capture counts from the checker's
//! [`crate::check::FnResolution`] and its type as text, computed once here, for
//! its closures to display as. The expression itself becomes a `MakeClosure`
//! whose captures are the reads, in the frame creating the closure, of each
//! value the function captures. Function bodies nest, and so does their
//! lowering. A call lowers to a `Call` whose `Arg` nodes are its arguments in
//! source order, each with the parameter the checker bound it to, and then one
//! for each parameter the call gives its default (see the checker's "Calls",
//! [`crate::check::Resolution::call_defaults`]), in parameter order, at the
//! call's closing parenthesis, whose value is the default's [`Constant`] built
//! where the call is: an `IntConst`, `FloatConst`, `BoolConst` or `StrConst`, a
//! `MakeList` of its elements, a `MakeRecord` of its fields in the order they
//! are written, its layout shared as a record block's is, or `Empty`
//! (`Lowerer::lower_constant`); or, for a default that is no constant, an
//! [`ArgValue::Default`](crate::ir::ArgValue::Default) of the capture of the
//! callee's closure that keeps its default function, of the parameter's kind by
//! the callee's type. So a function's `MakeClosure` has, after the reads of
//! what it captures, a `MakeClosure` of each of its default functions, in
//! parameter order, each lowered into the function table as a function is
//! (`Lowerer::with_default_fns`). So the callee is given a full list of
//! arguments, as if every one had been written, and a memoised function's key
//! is the same either way.
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
//! checker's [Imports](crate::check::imports#imports)). A reference through an
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
//! lists. `==` and `!=` on records and lists compare over the [`Shape`](crate::ir::Shape) (a
//! [`ListShape`](crate::ir::ListShape) for a list) of the operands' static type, read from the
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
//! unit-like list (see [`crate::check`]), unless it is a bare `await` or a
//! call the checker discards
//! ([`crate::check::Resolution::is_discarded`]), whose value, whatever it
//! is, is evaluated and dropped; a `for` statement whose type is
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
//! into the function table, as code of [`FnKind::Future`](crate::ir::FnKind::Future), and the
//! expression to a `MakeFuture` that captures as a `MakeClosure` does.
//!
//! A definition made with a def-type (see the checker's "Def-types") is a
//! top-level function named by the path of its namespace's key and its
//! own, lowered as a dotted `def` is: its closure stored into its leaf's
//! cell where it is made, its code shown in the dump by its def-type and
//! path, `fn #0 package dev.peios.testpkg`, and of [`FnKind::Memo`](crate::ir::FnKind::Memo) if the
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
//! dev.peios.testpkg`, and of [`FnKind::Memo`](crate::ir::FnKind::Memo) if its marker is `META->>`,
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
//! The program records the function made for every reference
//! (`Ir::set_references`), so that a host can tell a reference's closure from
//! any other that captures a definition (`Ir::is_reference`).
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
//! awaited deeply. A deep await carries a [`LazyShape`](crate::ir::LazyShape) of that type, listing
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

use std::collections::{HashMap, HashSet};

use crate::ast::{
    Ast, BinaryOp, ExprId, ExprKind, FnId, ForId, ItemId, ItemKind, ItemRange, PathSegment,
    SegmentKind, UnaryOp,
};
use crate::check::{AwaitMark, Checked, JumpTarget, Resolution, Slot, Types, VarRef};
use crate::mentions::FieldNode;

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
    CatchId, Cmp, FnCode, Ir, IrBuilder, IrId, IrKind, LayoutId, LazyShapeId, MetaAt, Operands,
    ShapeId, UpdateStep,
};
use crate::names::Name;
use crate::source::Span;

/// The capture of the closure of a def-type reference that holds the
/// definition, after the def-type's (`Lowerer::lower_def_type_ref`).
pub(crate) const REFERENCE_DEFINITION: u32 = 1;

/// The text of the key of the context a def-type reference's call binds,
/// around its def-type's call, to the definition it calls, as a prepared
/// call binds its own around all of it, and a definition's body binds
/// itself around its own code, wherever it runs: so that every step and
/// effect made in that call, whenever it is made, knows the definition it
/// is of ([`crate::workspace::Workspace::step_definition`]). No context can be declared of it,
/// since it is no name, and every table of names holds it first,
/// [`crate::names::DEFINITION`].
pub const DEFINITION_CONTEXT: &str = "<definition>";
use crate::types::{TyId, TypeTable};
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
        forks: Vec::new(),
        catches: Vec::new(),
        next_catch: 0,
        reads_caller: Vec::new(),
        meta_captures: Vec::new(),
        references: Vec::new(),
        definitions: definitions_of(ast),
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
    ir.set_foreign(resolution.place().foreign);
    ir.set_standard(&*resolution.place().this == crate::workspace::STANDARD_DIR);
    ir.set_paths(dotted_paths(ast, resolution));
    ir.set_metas(metas);
    ir.set_references(lowerer.references);
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
    /// ([`FieldSchedule`](crate::check::FieldSchedule)) have had their cells
    /// stored: the next one to store is at that index of its order.
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
    /// The iterations among [`Lowerer::loops`], outermost first, each with
    /// the slot of its variable, below which every variable declared
    /// outside it lies, and whether it is forkable so far (see
    /// [`IrKind::For`]).
    forks: Vec<(ForId, Slot, bool)>,
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
    /// The functions made for def-type references, every one (see
    /// [`Ir::is_reference`]).
    references: Vec<FnId>,
    /// The functions that are definitions' values, `def build app() { ... }`'s
    /// say, whose bodies bind their own definition to the context of
    /// [`DEFINITION_CONTEXT`] wherever they are called.
    definitions: HashSet<FnId>,
}

/// The functions of `ast` that are definitions' values: each top-level
/// definition made with a def-type, `def build app() { ... }`.
fn definitions_of(ast: &Ast) -> HashSet<FnId> {
    ast.items(ast.root())
        .iter()
        .filter_map(|&item| match ast.item(item).kind {
            ItemKind::DefTypeDef { value, .. } => match ast.get(value).kind {
                ExprKind::Fn(function) => Some(function),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

impl Lowerer<'_> {
    /// The type of expression `id` where it is used: its type once awaited
    /// at the top, if it is awaited, and its type otherwise.
    fn used_type(&self, id: ExprId) -> TyId {
        // A secret an operation on secrets takes is used as its value.
        if let Some((value, _)) = self.resolution.secret_operand(id) {
            return value;
        }
        self.awaited_type(id)
    }

    /// The type of expression `id` once awaited, if it is awaited where it
    /// is used, whether or not it is a secret's that is unwrapped there.
    fn awaited_type(&self, id: ExprId) -> TyId {
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
                | ItemKind::Enum { .. }
                | ItemKind::Variant { .. }
                | ItemKind::Context { .. }
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

    /// Lowers `value`, an expression item that is no block's result: it is
    /// evaluated for effect only, so it must be `{}` or a unit-like list,
    /// unless it is a bare `await` or a call the checker discards, whose
    /// value is discarded whatever it is. A unit-like `for` discards its
    /// body's values rather than build a list.
    fn lower_statement(&mut self, value: ExprId) -> Result<IrId, InternalError> {
        Ok(
            if self.is_await(value) || self.resolution.is_discarded(value) {
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
            },
        )
    }

    /// The span of the first item at or under `node`, a node of the tree of
    /// the input's top-level fields, whose root block's items are `root`:
    /// where its cell's `Store` and its loads are shown.
    fn node_span(&self, node: &FieldNode, root: &[ItemId]) -> Span {
        root.get(node.first)
            .map_or(Span::new(0, 0), |&id| self.ast.item(id).span)
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
        let node = match self.resolution.await_of(id) {
            AwaitMark::None => {
                let kind = self.kind_of_type(id, self.awaited_type(id))?;
                if kind == ValueKind::Never
                    && let Some(unreached) = self.lower_unreached(id)?
                {
                    return Ok(self.builder.alloc(unreached, self.ast.get(id).span));
                }
                self.lower_computed(id, kind)?
            }
            mark => self.lower_awaited(id, mark, self.ast.get(id).span)?,
        };
        // An operand of an operation on secrets: a secret unwrapped to its
        // value, and any marked as an operand, so that its failure is not
        // the operation's.
        let deep = match self.resolution.secret_operand(id) {
            Some((_, deep)) => deep,
            None if self.resolution.is_plain_operand(id) => false,
            None => return Ok(node),
        };
        let kind = self.kind(id)?;
        let value = IrKind::SecretValue {
            value: node,
            kind,
            deep,
        };
        Ok(self.builder.alloc(value, self.ast.get(id).span))
    }

    /// Lowers expression `id`, of kind `kind`, as [`Lowerer::lower_plain`]
    /// does, but an operation on secrets: the operation on their values, of
    /// its value's kind, made a secret (see the checker's "Secrets").
    fn lower_computed(&mut self, id: ExprId, kind: ValueKind) -> Result<IrId, InternalError> {
        let Some((value, revealed)) = self.resolution.secret_op(id) else {
            return self.lower_plain(id, kind);
        };
        let kind = self.kind_of_type(id, value)?;
        let computed = self.lower_plain(id, kind)?;
        // A field read of a secret names its field in the derivation.
        if let ExprKind::Field { name, .. } = self.ast.get(id).kind {
            self.builder.add_name_text(name, self.ast.names().get(name));
        }
        Ok(self.builder.alloc(
            IrKind::SecretWrap(computed, revealed),
            self.ast.get(id).span,
        ))
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
            (
                ExprKind::Duration(amount) | ExprKind::Size(amount),
                ValueKind::Quantity(quantity),
            ) => IrKind::Quantity(quantity, self.builder.alloc(IrKind::IntConst(amount), span)),
            (ExprKind::Bool(value), ValueKind::Bool) => IrKind::BoolConst(value),
            (ExprKind::Str(text), ValueKind::String) => {
                IrKind::StrConst(self.builder.push_string(self.ast.strings().get(text)))
            }
            (ExprKind::Interp(parts), ValueKind::String) => {
                self.lower_interpolation(id, parts, span)?
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
            (ExprKind::ContextRead(_), _) => self.lower_context_read(id, kind)?,
            (ExprKind::Assert { cond, message }, _) => IrKind::Assert {
                cond: self.lower_operand(cond, ValueKind::Bool)?,
                message: self.lower_operand(message, ValueKind::String)?,
            },
            (ExprKind::Recover(recover), _) => self.lower_recover(recover, kind)?,
            (ExprKind::With { bindings, body }, _) => {
                self.lower_with(id, (bindings, body), kind)?
            }
            (ExprKind::Variant { .. }, _) => self.lower_variant(id)?,
            (ExprKind::Error | ExprKind::NoField(_) | ExprKind::Unclosed { .. }, _) => {
                return Err(unparsed(span));
            }
            (
                ExprKind::Int(_)
                | ExprKind::Float(_)
                | ExprKind::Duration(_)
                | ExprKind::Size(_)
                | ExprKind::Bool(_)
                | ExprKind::Str(_)
                | ExprKind::Interp(_),
                _,
            ) => return Err(mistyped(expr.kind, kind, span)),
        };
        Ok(self.builder.alloc(lowered, span))
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

pub(crate) mod blocks;
pub(crate) mod calls;
pub(crate) mod context;
pub(crate) mod failure;
pub(crate) mod functions;
pub(crate) mod loops;
pub(crate) mod narrowing;
pub(crate) mod operators;
pub(crate) mod records;
pub(crate) mod shapes;
pub(crate) mod statements;
pub(crate) mod toplevel;
pub(crate) mod updates;

#[cfg(test)]
mod tests;

/// The `Int` node an integer operator makes, `+`, `-`, `*`, `//` or `%`, which a
/// quantity's arithmetic uses too; `None` for any other operator.
fn integer_operator(op: BinaryOp) -> Option<fn(Operands) -> IrKind> {
    Some(match op {
        BinaryOp::Add => IrKind::IntAdd,
        BinaryOp::Sub => IrKind::IntSub,
        BinaryOp::Mul => IrKind::IntMul,
        BinaryOp::FloorDiv => IrKind::IntFloorDiv,
        BinaryOp::Rem => IrKind::IntRem,
        _ => return None,
    })
}
