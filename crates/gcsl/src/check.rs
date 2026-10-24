//! The type checker and name resolver.
//!
//! One pass over the tree both types every expression and resolves every
//! variable. Its results are side tables indexed by the AST's ids:
//! [`Types`], the type of every expression, and [`Resolution`], where every
//! variable use reads from and every declaration writes to, what each
//! function's and `lazy`'s frame and captures are, which parameter each
//! argument fills, and which expressions are awaited where they are used.
//! Both are returned together as [`Checked`]. Types are [`TyId`]s interned
//! into a [`TypeTable`] the caller owns, so that a
//! [`crate::session::Session`] can keep one table across inputs.
//!
//! # Types
//!
//! `Int` and `Float` never mix implicitly. The one exception is that integer
//! literals adapt to their context: a literal starts as
//! [`TyId::UNTYPED_INT`], and an operator that needs a concrete type for it
//! decides one. The checker works in two directions:
//!
//! - **Up**: [`Checker::check_expr`] computes each node's type from its
//!   children's. Operators whose operands are all untyped produce
//!   `UntypedInt` themselves where the rules allow, so `1 + 2` stays untyped,
//!   and so does a block whose result is untyped.
//! - **Down**: when an untyped subtree meets a concrete type, as in
//!   `(1 + 2) * 3.0` or `let x: Float = 5`, [`Checker::coerce`] walks the
//!   untyped subtree and rewrites every entry in it to the decided type. It
//!   stops at nodes that are already concrete, so each node is rewritten at
//!   most once and the whole check stays linear.
//!
//! A variable must have a concrete type, so an untyped initialiser defaults
//! to `Int` where it is bound: after `let x = 5`, `x` is an `Int`, and
//! `x + 2.5` is an error. Finalisation defaults an untyped file result to
//! `Int` in the same way, and so do the other places that need a concrete
//! type but give no context: two untyped operands of a comparison, and an
//! interpolated expression. Every untyped node is either coerced by the
//! nearest concrete context or defaulted where it is bound or used, so
//! afterwards the table contains no `UntypedInt`.
//!
//! An `if` with `else` has the join of its branches' types (see
//! [`TypeTable::join`]), with untyped branches adapting as operands of `+`
//! do first; if both are untyped, the `if` stays untyped, like a block, and
//! `coerce` pushes a type into both branches. The join always exists:
//! branches that do not agree make a union, which is a warning, not an
//! error (see [Unions](#unions)). An `if` without `else` is `{}`, and so
//! must its block be, or `Never` (see [Never](#never)). There is no
//! truthiness: conditions and the operands
//! of `and`, `or` and `not` must be `Bool`.
//!
//! A node whose operator rejects its operands is reported once and given
//! [`TyId::ERROR`]. Every operator accepts `Error` operands silently and
//! produces `Error` itself, so the mistake does not cascade into a spurious
//! error at each enclosing operator. A variable whose initialiser has an
//! error is still declared, with type `Error`, and a use of an undefined
//! variable has type `Error`, so neither cascades either. Independent
//! mistakes elsewhere in the tree are still reported, so one run shows every
//! real error.
//!
//! A tree the parser recovered from syntax errors ([`Ast::syntax`]) is
//! checked as any is, an error node ([`ExprKind::Error`]) being of type
//! `Error` and a block with an error item ([`ItemKind::Error`]) erroneous,
//! its fields' and its result's types unknown; its check always fails. What
//! the parser kept of a half-typed field access or call
//! ([`ExprKind::NoField`], [`ExprKind::Unclosed`]) is of type `Error` too,
//! its parts checked as any expression is, for an index, with no
//! diagnostic: alone, a part may be what it may not be (a module or a
//! built-in read whole, a callback whose parameters only its call types),
//! and the error is the parser's (`Checker::check_unfinished`). In
//! it, and in a workspace's file whose view sees a file with syntax errors,
//! a name that names nothing, a variable, a type or a record's field, is no
//! error, since it may be declared where the source could not be parsed:
//! the check fails quietly instead (`Checker::unresolved`, and see
//! [`crate::workspace`], "Syntax errors"). In a workspace whose scan
//! recovered from its own errors, a name the scan poisoned in the file (what
//! an import that failed binds, a name in conflict) is read as `Error`,
//! silently, as a value or as a type, unless a binding of a function or an
//! inner block shadows it (`Checker::reads_poisoned`), and a check that
//! found an erroneous type with nothing to report, a read of what the scan
//! poisoned, fails quietly, its result never lowered (see
//! [`crate::workspace`], "Scan errors"). Nor is an empty list an error of
//! its own where an erroneous expected type, or an erroneous callee's
//! parameter, would have typed it (`Checker::check_list`).
//!
//! In a diagnostic's message and notes, a type written as a name, such as
//! `Int`, is shown plainly, and a structural type (a record, list,
//! function, future or union type) in backticks, since it contains spaces and
//! punctuation: "cannot add Int and `{}`". Labels show every type plainly:
//! "expected {x: Int}, found Int".
//!
//! A type may also be written as the name of a declared type, `type
//! PackageDef = {...}`, bare or qualified through an import (`t.PackageDef`),
//! and a record type may spread one, `{...PackageDef, name: String}` (see
//! [Type declarations](#type-declarations)). A declared type is a
//! STRUCTURAL ALIAS: it is its definition, not a new type, so two aliases of
//! one record type are one type, a record of the right fields is a
//! `PackageDef` with no ceremony, and a type is shown as what it stands for,
//! never by the name of an alias of it: an error about a `PackageDef` shows
//! the record type.
//!
//! # Records
//!
//! A block with at least one field or spread item is a *record block*. Its
//! value is the record of its fields, so it has no result: a `= EXPR` in it
//! is an error, and a final expression statement is a statement like any
//! other. A field is declared like a variable: it takes a slot, is visible
//! from the next item to the end of its block, shadows and is shadowed like
//! one, and is a binding marked as a field so that `:=` on it can be
//! rejected. A field item defines a *hard* field, which is defined once; a
//! spread defines *soft* ones, which later items may define again (see
//! [Spreads and soft assignment](#spreads-and-soft-assignment)). Each
//! block also records its fields in order in one flat vector of field
//! declarations, as it does its bindings, each where it was first defined,
//! and at the end of a record block the block's type is the record of those
//! fields.
//!
//! Where a type is *expected* (an annotation or an ascription), a subtype is
//! accepted (see [`TypeTable::is_subtype`]). A record block written where a
//! record type is expected, through any parentheses, is checked against it
//! directly by `Checker::check_expr_expecting`: each field's initialiser
//! is checked against the expected field's type, so literals adapt and
//! nested record blocks are checked the same way; a missing field is an
//! error, unless the record type is expected itself (not as a union's
//! member) and has a default for it, which the block is then given (see
//! "Field defaults" under [Functions](#functions); `{}` is checked so too
//! where such a type is expected), and a field item the
//! type lacks is a *warning*, since nothing with
//! the expected type can see it. Warnings are collected apart from errors
//! and returned with a successful check.
//!
//! An expected record type with a rest-field, `*: T` (see
//! [`crate::types`], "Rest-fields"), lacks no field: each field item it
//! does not name is checked against `T` as against a field of the type,
//! and so is each soft field a spread defines and each dotted path's value,
//! and an annotated field's annotation must be a subtype of `T`. The
//! block's record then has the rest-field, which no record built otherwise
//! has: so a literal can be a value of such a type, and a value whose type
//! hides fields cannot (`Checker::check_rest_fields`). A field that dotted
//! paths build is a record without one, which does not fit a `T` that has
//! one: "mismatched types" at its first definition, noted to define it
//! whole. A record type with a rest-field reaches `{}` and the record
//! literals a block or an `if` results in as a record type with defaults
//! does, to give them the rest-field. A spread of a value whose type has a
//! rest-field is an error, since a spread defines the fields its operand's
//! type names; so is reading by name a field only the rest-field covers.
//!
//! A field item may be annotated, `PATH: TYPE = EXPR`, in any block, the
//! top level included, exactly as `let PATH: TYPE = EXPR` is for a `let`
//! (`Checker::annotated_value`): the annotation is resolved with every bare
//! type present, the value is checked against it as an annotated `let`'s
//! is (a literal adapting, a record block field by field, a value awaited
//! to fit where it may be, subject to the purity rule, a pure function's
//! parameter measured), and the field's type IS the annotation's, not the
//! value's narrower type; on a dotted path the annotation applies to the
//! last segment, and the segments above it are built from it as from any
//! leaf; a quoted name is annotated as a plain one. A top-level field's
//! annotated type is its leaf's, which every reader sees, an item above it
//! and another file alike (see [Top-level fields](#top-level-fields)). A
//! top-level field that is annotated is never an alias of a built-in
//! function (see [Built-in functions](#built-in-functions)), as an
//! annotated `let` is never one. In a record block checked against an
//! expected record type, an annotated field has its annotation's type,
//! which must be a subtype of the expected field's, "mismatched types" at
//! the annotation otherwise, and one the expected type lacks is warned
//! about as any field it lacks is. A `def` is not annotated: it has its
//! signature.
//!
//! # Dotted fields
//!
//! A field item whose path has several segments, `a.b.c = e`, declares the
//! field `a` as a record containing `b`, a record containing `c`. Dotted
//! definitions with a common prefix merge, as in TOML:
//!
//! ```text
//! server.host = "localhost"
//! server.port = 8080
//! server.tls.enabled = true
//! # one field, server = {host = ...; port = ...; tls = {enabled = true}}
//! ```
//!
//! A field whose first definition is dotted is *open*: it is one field
//! declaration and one binding, in one slot, and each further dotted path
//! under it extends it. The binding's type grows with each extension, so
//! after `server.host = "..."` the name `server` has type `{host: String}`
//! from the next item on, and after the next line `{host: String, port:
//! Int}`; its slot holds the record built so far. The record's fields are in
//! the order they were first mentioned, at every level, and the field takes
//! its place in its block where it was first mentioned.
//!
//! The rules, checked in the block declaring the field, each reported at the
//! offending path with a label on the earlier definition it conflicts with:
//!
//! 1. A field defined whole (`a = e`) cannot be extended: a later dotted
//!    path under it is "field `a` is already defined". Nor can a segment
//!    whose value a dotted path defined whole as a record (`a.b = { x = 1 }`,
//!    then `a.b.y = 2`: "field `a.b` is already defined"). A whole
//!    definition after a dotted one is "field `a` is defined twice", as for
//!    any field defined twice.
//! 2. A segment along the path that exists as some other value cannot hold
//!    fields: "cannot define `a.b.c`: `a.b` is Int, not a record".
//! 3. The path's last segment must not exist yet: "field `a.b` is defined
//!    twice".
//!
//! A rejected path declares nothing, but its value is still checked, and
//! its block's record is erroneous, so that the mistake does not cascade.
//!
//! To check these, the checker keeps a small tree of the segments of each
//! open field (`Node`), in one arena for the whole checker that each block
//! truncates back when it ends, as it does its field declarations. After an
//! extension, the types of the segments along its path are interned again,
//! from the leaf outwards.
//!
//! All of this, the binding that grows and the slot that holds the record
//! built so far, is how a dotted field of an INNER block works. At the top
//! level of an input the rules above are kept, in either order of the two
//! items, but the paths are order-free: each node of the tree is a cell of
//! its own, a reference means the longest declared prefix of its path, and
//! a root is assembled once from its children's cells, not item by item
//! (see [Top-level fields](#top-level-fields)). So `a.b = 1; c = a; a.d =
//! 2` gives `c` the whole `a`, `{b = 1; d = 2}`. The types of the open
//! nodes above a top-level leaf are interned once, children first, when the
//! input's last item is checked (`Checker::settle_top_nodes`), so that a
//! root of N dotted items costs N steps, not N^2.
//!
//! In a record block checked against an expected record type, a dotted
//! field's value is checked against the type the expected type has at the
//! value's full path, as a field's value is against its field's type, if it
//! has one there. If it lacks a segment of the path, the value is checked
//! on its own and the missing segment is warned about as an extra field, at
//! its first definition only. When the block ends, each open field is
//! checked against the expected type at every level of nesting: a missing
//! field is an error, "missing field `a.x` in record", and a segment that
//! the expected type has as something other than a record is a mismatch.
//!
//! # Spreads and soft assignment
//!
//! A spread item, `...EXPR`, is a sequence of *soft assignments*, one for
//! each field of the record it spreads: `...{ x = 5; y = 2 }` is `x = 5; y =
//! 2`, except that what it defines may be defined again.
//!
//! ```text
//! let defaults = { host = "localhost"; port = 80 }
//! let conf = {
//!   ...defaults
//!   port = 8080
//!   url = "{host}:{port}"
//! }
//! # {host = "localhost"; port = 8080; url = "localhost:8080"}
//! ```
//!
//! The operand is checked on its own, then measured at the top, as the
//! record of a field access is (`Checker::measure`): awaited if it may be a
//! future, subject to the purity rule, and a pure function's bare parameter
//! measured; the await is marked on the operand, an expression, in
//! [`Resolution::await_of`], as for a list's spread. It must then be a
//! record: "cannot spread a value of type T: it is not a record". The fields
//! it defines are exactly those of its type once measured, whatever else the
//! runtime value holds, since the static type is the truth: a field hidden
//! by width subtyping is not copied. So an ascription selects fields,
//! `...conf : {b: Int}` defining only `b`, the ascription belonging to the
//! operand, which is parsed as a field's value is; a bare type there is
//! present, so it awaits a field that may be a future, and `{b: Int |
//! Future<Int>}` keeps it lazy. `{}` spreads nothing. A type has no
//! definition order, so the fields are taken in the order the type table
//! holds them, by [`Name`], which is the order of the soft assignments and
//! of the places the fields take in the record.
//!
//! Each field is then softly assigned: it becomes a field of the block's
//! record and a name in scope for the block's later items, exactly as a
//! field item makes it, with the type the operand's type gives it, which
//! may be the either shape or a future, since nothing inside the operand is
//! awaited. A field so defined is *soft* (its `FieldDecl` records the
//! operand of the spread whose definition stands); one a field item defines
//! is *hard*, as a dotted field is.
//!
//! - A later field item of a soft field's name overrides it: the field is
//!   hard from then on, with the new value and type, which may differ.
//! - A later spread with a field of a soft field's name overrides it too,
//!   and the field stays soft.
//! - A spread with a field of a hard field's name is "field `x` is defined
//!   twice", at the operand, labelled "`x` is a field of this, of type T",
//!   with "first defined here" on the hard field's name and the note "a
//!   spread cannot replace a field defined before it; put the spread
//!   first": once for the spread, naming the first such field and counting
//!   the others in the note. The hard field stands; the spread's other
//!   fields are still defined, so that their uses do not cascade, and the
//!   block's record is erroneous.
//! - A field item after a field item is "defined twice", as ever.
//!
//! At the top level of an input it is different, since a top-level field or
//! `def` is declared in the whole input (see [Top-level
//! fields](#top-level-fields)): it overrides a spread's field of its name
//! whether the spread is above it or below it, so a spread below one is no
//! longer "defined twice". The spread defines nothing and writes no slot
//! for a name that is a top-level declaration of the input, so the
//! declaration's cell is never overwritten, whichever is computed first;
//! where the spread comes first, the field keeps the place the spread gave
//! it in the record. A dotted root of a spread's name is "field `a` is
//! already defined" in either order: below the spread at the path, as
//! before, and above it at the spread's operand.
//!
//! An overridden field keeps its place in the record, where it was first
//! defined, and its one slot, which every definition of it writes, as every
//! definition of an open field does; the name, as a variable, is bound
//! anew by each definition, as a `let` shadows, in that slot, so the items
//! between a spread and an override see the spread's value and those after
//! it the override's. Only a block that has had a spread looks for a soft
//! field when a field item defines a name (`BlockSpreads`), so a block
//! without one does no more work than before. A spread holds its operand's
//! value in a slot of its own while its fields are read, the next free slot
//! once they are declared, which it leaves free and no binding owns; where
//! each field is stored is recorded in [`Resolution::spread_slots`]. A
//! dotted path whose first segment is a soft field is "field `a` is already
//! defined", as for a field defined whole; a dotted `let` of the name is an
//! ordinary update of the variable. `:=` of a soft field's name is the
//! error for any field.
//!
//! In a record block checked against an expected record type, each soft
//! field that stands when the block ends, one no later item overrode, is
//! checked against the expected field's type, if the type has the field,
//! as a field's value is, except that it cannot be awaited to fit, there
//! being no expression to await (`Checker::check_soft_fields`): a
//! provisional type in it where the expected type is present is measured,
//! at the operand; one that still does not fit is "mismatched types" at the
//! operand, "expected E, found F for field `a`", with the note "a spread's
//! field cannot be awaited; define the field with its value, as in `a =
//! conf.a`" when awaiting would have made it fit (the example written with
//! the operand when it is a name or a dotted path, and left out otherwise);
//! and one that fits takes the expected type, as a field item's does. A
//! soft field the expected type lacks is not warned about, since spreading
//! a wider record is normal. Missing fields are reported as ever.
//!
//! # Type declarations
//!
//! `type NAME = TYPE`, an item of a file's top level, declares a type in
//! the file's namespace of types, which is not its variables': a type and
//! a field, `def`, `let` or `fn` of one name coexist, a type's name being
//! read only in a type position. Types are order-free, public as `def`s
//! are, and importable; which file's declaration a name or a qualified name
//! names is the workspace's to say (see [`crate::workspace`], "Types"), and
//! it is said before any file is checked: the workspace *expands* every
//! file's types first ([`Ast::expand_types`]), so that each name of a
//! declared type, wherever it is written (an annotation, an ascription, a
//! parameter, a function type, another `type`), is what the type's
//! definition is, as if the definition were written there, and each record
//! type with spreads is the record of its merged fields. The checker
//! resolves the expanded types with the rules above, unchanged, and knows
//! no type's name: so bareness applies where an alias is used, as if its
//! definition were written there (`type N = Int` used as an impure
//! function's bare parameter is `Int | Future<Int>` there, and used as a
//! pure one's is inferred), a union with an alias of `Future<Int>` among its
//! members is a union with a `Future` member, an alias of a function type
//! is a callback parameter, and an impure function's result written as an
//! alias of a record type is inferred field by field.
//!
//! A spread in a record type, `{...PackageDef, name: String}`, merges as
//! value spread does (see [Spreads and soft
//! assignment](#spreads-and-soft-assignment)): the spread's fields are soft,
//! a later field of one's name overrides it (its type may differ), a later
//! spread overrides a soft one, a field written twice is "field `x` appears
//! twice in this type", and a spread's field of a field written before it
//! is "field `x` is defined twice", at the type spread, labelled "`x` is a
//! field of this, of type T" and "first defined here", with the note "a
//! spread cannot replace a field defined before it; put the spread first"
//! (`Checker::spread_problems`). Only a record type may be spread, written
//! so or named: anything else is "cannot spread type T: it is not a record
//! type", at it, `T` shown as what it stands for. A rest-field, `*: T`,
//! merges as a field would: a spread's is soft, a second written one is
//! "the rest-field appears twice in this type", and a spread's after a
//! written one is "the rest-field is defined twice", labelled "this has a
//! rest-field, of type T".
//!
//! What is wrong in a declaration's definition is reported once, where it
//! is written: the workspace checks every file's `type` items before any
//! file is checked ([`check_types`], `Checker::type_items`), since a file
//! may use a type of a file checked after it; there a name of a built-in
//! type is "`Int` is a built-in type", and a second `type` of one name in
//! one file "type `Foo` is declared twice", labelled "first declared here".
//! A copy of a definition in another file, or in its own (a use of the
//! type), is resolved quietly (`Checker::resolve_quietly`), once per mode
//! however many uses share it: if anything in it is wrong it is `Error`,
//! reported nowhere here, and the check fails *quietly*, with no error of
//! its own (a file's `type` items are resolved quietly too, in its own
//! check). So a type whose definition has an error is erroneous wherever it
//! is used, and reports once. A name that names nothing it can (an unknown
//! qualified name, a module's name, a declared type applied to a type
//! argument, a type nested too deeply once expanded) is reported where it
//! is written, from the problems the expansion kept ([`Ast::type_problems`]).
//! Checked alone, as [`check`] checks a file, with no workspace to expand
//! its types, a file knows no declared type: each of its names is "unknown
//! type", and a spread an internal error. A `type` item has no existence at
//! runtime: it is never run, nor warned about as never run, and lowers to
//! nothing; `--dump-types` shows each with its expansion
//! ([`Types::declared`]).
//!
//! # Dotted `let` and `:=`
//!
//! `let a.b.c = e` is a functional update. If no `a` is in scope, it
//! declares `a` as `{b = {c = e}}`. If `a` is in scope with a record type,
//! it declares a new binding `a`, shadowing the old one as any `let` does,
//! whose value is the old value with the path set: each segment between the
//! first and the last that the old value's type has as a record is updated
//! inside, each it lacks is created as a record, and the last is replaced
//! if the type has it (its type may change) or added. An annotation applies
//! to the last segment: `let a.b: Int = 5`. A segment along the way that is
//! not a record, or `a` itself not being one, is an error: "cannot update
//! `a.b.c`: `a.b` is Int, not a record". The new type is computed by walking
//! the old one along the path and interning the updated record types from
//! the leaf outwards.
//!
//! `a.b.c := e` is an update too, but `:=` is assignment-compatible: `a`
//! must be something `:=` may rebind, every segment of the path must
//! already exist (`:=` never adds a field: "cannot rebind `a.b.c`: `a.b`
//! has no field `c`"), and `e` must fit the leaf's type, a subtype of it or
//! an adapting literal, which the new binding keeps. Changing a variable's
//! type, or adding a field, takes a `let`.
//!
//! The old value is never changed. An update builds a new record, sharing
//! the old one's unchanged fields, so another binding holding the old value
//! still sees the old record:
//!
//! ```text
//! let a = { x = 1 }
//! let old = a
//! let a.x = 2      # a is {x = 2}; old is still {x = 1}
//! ```
//!
//! **Order.** Every dotted `let` and dotted `:=`, of a mutable variable or
//! not, is evaluated in one order, Python's: first its value, once; then
//! the variable is read, and the path walked from its first segment, each
//! record (or list, see [Lists](#lists)) along it read once and awaited
//! where it is marked to be, and each index segment's position evaluated
//! once, where the walk reaches it, left to right; and then the new value
//! is built from the leaf outwards and stored. So the update applies to the
//! variable as it is when the assignment happens: `r.a := { r := { a = 100;
//! b = 7 }; r.b + 1 }` makes `r` `{a = 8; b = 7}`, and a record along the
//! path that is a future is awaited after the value's effects. The order
//! changes nothing here: the checker walks the path before it checks the
//! value, since the value is checked against the leaf's type, and its
//! types, awaits, purity errors and their spans are the same whatever order
//! the program runs in (see [`crate::lower`] and [`crate::eval`] for how
//! the order is kept).
//!
//! The static type is the truth. An update that creates an intermediate
//! record where the static type has no such field replaces whatever the
//! runtime value holds there: a field hidden by subtyping, as in
//! `let p: {x: Int} = { x = 1; y = 2 }` followed by `let p.y.z = 3`, whose
//! `y` becomes `{z = 3}`.
//!
//! Updating a record reads it. Every record along the path that exists
//! (the variable's, and each following segment's, short of the last, that
//! the record before it has) is measured at the top, as the record of a
//! field access is (see [Futures](#futures)): a pure function's bare
//! parameter is measured, which makes it present, and a record that may be
//! a future is awaited, subject to the purity rule, whose error is reported
//! at the path up to that segment. The rules above then apply to its type
//! once awaited, stripped of its future layers, so that a record that is
//! not one once awaited is "not a record", shown as the type it had before,
//! and then alone: the update cannot work whether or not it is awaited, so
//! the purity error is reported only for a value that is a record once
//! awaited. The new binding is built from the stripped types: the updated
//! variable is present, as is every record along the path, and the fields
//! off the path keep their types. After `let r = lazy { a = 1 }`, `let r.a
//! = 2` makes `r` a `{a: Int}`. The last segment is never measured: it is
//! replaced, not read, so for `:=` it keeps the field's type, which may be
//! the either shape. Dotted fields are not affected: they only extend
//! records their own block is building.
//!
//! The path of a `:=`, and only of a `:=`, may index lists too, `xs[i] :=
//! v` or `conf.hosts[0].up := true`, which replaces an element of a list:
//! see "Updating an element" under [Lists](#lists).
//!
//! How a dotted item resolves (the slot of the record or list it updates,
//! how many of its path's segments name records or lists that already
//! exist, and which of those are awaited) is recorded for lowering as a
//! [`PathResolution`].
//! A path segment is not an expression, which [`Resolution::await_of`]
//! could mark, so the awaited records are listed in a table of their own,
//! [`Resolution::path_awaits`], in which only an item that awaits one has
//! entries.
//!
//! # Mutable variables
//!
//! `let mut x = e` declares a *mutable variable*, checked exactly as `let x
//! = e` is (and `let mut x: T = e` as `let x: T = e`), whose type is fixed
//! there for its whole life. Its binding is of kind `BindingKind::Mutable`,
//! and shadows and is shadowed as any variable's is: a later `let x`, `let
//! mut x` or dotted `let x.a = ...` is a new variable in a new slot, which
//! is mutable only if it is a `let mut`. A `let mut`'s path is a single
//! name (the parser rejects a dotted one).
//!
//! ```text
//! let mut total = 0
//! for x in xs { total := total + x }
//! let mut conf = { port = 80; host = "a" }
//! if tls { conf.port := 443 }
//! ```
//!
//! Values never change: a mutable variable is one that `:=` may give a new
//! value from *any* block of the function that declares it (the top level,
//! for one of the top level), an `if`'s branch, a `for`'s body or guard, a
//! nested block or a record block, and not only from its declaring block.
//! `x := e`, where the innermost binding of `x` (`lookup`) is a mutable
//! variable, is checked by `Checker::check_mutable_rebind`:
//!
//! 1. If the variable is of the function being checked, the rebind is
//!    allowed from any block, and `e` is checked against the variable's
//!    type exactly as a `:=` in the declaring block of a variable that is
//!    not mutable is: "the variable was declared here", literals adapting,
//!    a value awaited to fit where an await is allowed. Nothing is
//!    declared: the item writes the variable's own slot (see
//!    [`Resolution::item_slot`]), and the variable keeps its type, its
//!    declaration and its place in `bindings`. So in the declaring block
//!    too, a mutable variable has one slot for its whole life, and a frame
//!    does not grow with its rebinds.
//! 2. If it is of an enclosing function or of the top level, and the `:=`
//!    is in a function expression or a `lazy` nested in that, it is "cannot
//!    rebind `x` here: it is declared outside this function" (or "this
//!    `lazy`", for the innermost being one), labelled at the declaration,
//!    with a note that the function holds its own copy of the variable. The
//!    value is checked on its own and nothing is declared, so nothing
//!    cascades.
//!
//! A rebind of a mutable variable that is erroneous already, whose error
//! has been reported, checks its value on its own and declares nothing too.
//!
//! A dotted rebind, `x.a.b := e` or `xs[i].a := e`, of a mutable variable
//! of the function being checked, from any block, walks its path exactly
//! as any dotted `:=` does (see "Dotted `let` and `:=`" above): each
//! existing record or list measured, awaited where it may be a future,
//! subject to the purity rule; every segment existing; each position
//! checked; `e` fitting the leaf's type. The updated record or list is
//! written to the variable's own slot, and the variable keeps its declared
//! type, which an assignment in an `if` must not change for the code after
//! it, where a variable that is not mutable takes the updated type, the
//! records and lists along the path present. So the updated type must be a
//! subtype of the declared one (`Checker::kept_type`), which it is whenever
//! the variable and the records along the path are present or the either
//! shape; otherwise, as for a variable declared exactly `Future<{a: Int}>`,
//! it is "cannot rebind `r.a`: `r` is declared `Future<{a: Int}>`, and the
//! update makes it `{a: Int}`", at the path, the value still being checked.
//! It is evaluated as every dotted update is: its value first, then the
//! variable read, so `r.a := { r := { a = 100; b = 7 }; r.b + 1 }` updates
//! the record the value stored, and `r` is `{a = 8; b = 7}`.
//!
//! A variable that is not mutable is rebound as before: a `:=` in its
//! declaring block declares a new binding, and one from an inner block is
//! "declared in an enclosing block", whose note says that only a `let mut`
//! variable can be rebound there (a parameter and a `for`'s variable, which
//! cannot be declared `mut`, included), unless the `:=` is in a function or
//! a `lazy` nested in the variable's own, where `let mut` would not help
//! either. Fields and the callee are rejected as ever.
//!
//! Nothing else is new. Rebinding a mutable variable is not an effect
//! anything outside its function can see, so pure, memoised and impure
//! functions alike may. A function expression or a `lazy` that reads a
//! mutable variable of an enclosing function captures it, by value, when
//! it is created, as any variable (see [Functions](#functions)), so later
//! rebinds do not reach it, and no value is ever shared and changed. A `for`
//! and an `if` are not functions, so their guards and bodies may rebind the
//! enclosing function's mutable variables. There is no flow typing: a
//! mutable variable has its declared type at every use, so one that may be
//! a future is measured, and awaited, at each.
//!
//! Since the variable may come to hold another value, nothing is inferred
//! from its initialiser beyond its type: a mutable variable initialised
//! directly with a function expression does not remember the function
//! (`Binding::function` is `None`, and so is its entry of a session's
//! [`Scope`]), so a call through it is typed by its type alone (see "The
//! call-site error" under [Inferred parameters](#inferred-parameters)).
//! Inside `let mut f = fn ...`, the name `f` is still the function being
//! called, whatever `f` holds later. Its type is its initialiser's, which
//! for a function is the type inferred for it (see [Inferred
//! results](#inferred-results)); an annotation gives it a wider one.
//!
//! # Equality
//!
//! `==` and `!=` are *typed* equality, and `===` and `!==` *total*
//! equality. Both kinds need operands of one identical type. On scalars
//! they are the same. On records, typed equality compares only the fields
//! of the operands' static type, and of the static types of their nested
//! records, so after `let q: {x: Int} = { x = 1; y = 2 }`, `q == { x = 1 }`
//! is `true`; total equality compares whole runtime values, deeply and by
//! name, so `q === { x = 1 }` is `false`. The checker records nothing for
//! either: lowering reads the operands' type from the type table.
//!
//! Functions have no equality: once the operands are found to have one
//! identical type, a type that is a function type, or a record or list type
//! containing one at any depth (see [`TypeTable::contains_fn`]), is an
//! error. Nor do they have an order. Lists compare as records do, element by
//! element (see [Lists](#lists)). A union has no static type to compare
//! typed by: `==` and `!=` of two operands of one union type are "cannot
//! compare `Int | String` with `==`", with the note "narrow it, or compare
//! with `===`", while `===` and `!==` compare their runtime values whole;
//! nor does a union have an order (see [Unions](#unions)).
//!
//! # Functions
//!
//! A function expression's type is known before its body is checked, since
//! every parameter and the return type are declared: its parameter and
//! return types are resolved and its function type interned first, and then
//! the body is checked as a block whose scope starts with the parameters,
//! against the return type as an annotated initialiser is checked against
//! its annotation. The parameters are bindings like any other, so a `let`
//! may shadow one and a `:=` rebind one, in the body's own block, since a
//! parameter is not a mutable variable. (The three things the body decides
//! are whether a pure function's bare parameters may be futures, whether
//! an impure function's bare result is, and whether an impure function's
//! callback parameter may be given futures: see [Inferred
//! parameters](#inferred-parameters), [Inferred
//! results](#inferred-results) and [Narrowed
//! callbacks](#narrowed-callbacks).)
//!
//! A named function item, `fn f(...) ...`, is checked as the `let f =
//! fn(...) ...` it is parsed as, and a public function item, `def f(...)
//! ...`, as the field `f = fn(...) ...`: in every rule about fields (whether
//! a block has fields and so cannot have a result, the record's fields and
//! their order, a field defined twice, a soft field overridden) a `def` is
//! that field. The tree marks both as written, which only the printer
//! reads here.
//!
//! **Frames.** Each function runs in a frame of its own. The checker keeps a
//! stack of the functions it is inside (`Function`); entering one saves the
//! enclosing frame's slot allocation and starts a fresh one at slot 0, in
//! which the parameters take the first slots, and leaving it records the
//! function's frame size and restores the enclosing allocation. The
//! top-level body is the outermost frame. What the checker learns about each
//! function is recorded in [`Resolution`] as a [`FnResolution`], indexed by
//! its [`FnId`].
//!
//! **Captures.** A use of a name resolves to a [`VarRef`]: a slot of the
//! current frame, a captured value, or the function being called. `lookup`
//! finds the binding as for any variable; if it lies below the innermost
//! function's first binding, it belongs to an enclosing function (or the
//! top level), and the use is a capture. Every function between the
//! binding's home and the use captures it: the outermost of them copies it
//! from its home frame (its slot, or the home function's callee), and each
//! function nested in that one copies it from the captures of the function
//! enclosing it, so the use reads its own function's capture. This is the
//! upvalue chain of Lua and the free-variable cells of Python, except that
//! there are no cells: each closure holds copies, made when its function
//! expression is evaluated. For a variable that is not mutable, `:=`
//! declares a new binding rather than assigning to the old one: after `let
//! x = 1; let f = fn(): Int { x }; x := 2`, the `x` that `f` uses is the
//! first binding, whose value is 1 whenever it is read, so `f()` is 1. A
//! mutable variable is one binding, whose slot each `:=` writes (see
//! [Mutable variables](#mutable-variables)), and capturing by value is what
//! keeps it from being shared: after `let mut x = 1; let f = fn(): Int { x
//! }; x := 2`, `f()` is 1 too, since `f` copied `x` when it was made, and
//! no function can rebind the variable it copied, which `:=` from inside
//! one is rejected for. So no value is ever shared and changed. A capture
//! list is searched linearly, since it is short.
//!
//! **Self-reference.** A function expression that is the direct initialiser
//! of a single-name `let`, field, named function item or `def` (not through
//! parentheses) sees that name throughout its body as the function being
//! called: a binding declared before the parameters, so a parameter of the
//! same name shadows it, of kind `BindingKind::Callee`, with no slot. It
//! resolves to [`VarRef::Callee`], which the evaluator serves from the
//! closure being run, so a recursive closure never holds a reference to
//! itself and reference counts stay free of cycles. It cannot be rebound
//! with `:=`, and never reaches a session's [`Scope`]. A `def` named by a
//! dotted path has no such name: it calls itself by its path, as any other
//! top-level function (see [Top-level functions](#top-level-functions)). A function is
//! visible from the item after its own, as any binding is, except a
//! top-level function, whose name is in scope in its whole input, so that
//! top-level functions may be used above their declarations and call each
//! other in any order (see [Top-level functions](#top-level-functions)).
//! One top-level function's use of another is not self-reference and not a
//! capture: it reads the other's slot of the top level's frame
//! ([`VarRef::Fn`]). Mutual recursion between functions that are not
//! top-level does not exist: an inner function sees only what is above it.
//!
//! **Calls.** The callee must have a function type. Positional arguments
//! fill its parameters left to right, the trailing block fills the last,
//! and named arguments fill the parameters of their names; each argument
//! is then checked against its parameter's type as an annotated
//! initialiser is. The parameter each argument fills is recorded for
//! lowering in [`Resolution::arg_params`]. Too many positional arguments, an
//! unknown or unusable name, a parameter filled twice and parameters left
//! unfilled are each reported, and make the call erroneous. A binding
//! initialised directly with a function expression remembers which, so
//! that a call through its name knows the function's [`FnResolution`],
//! unless it is a mutable variable, which may come to hold another. A
//! call of a built-in function is typed apart (see [Built-in
//! functions](#built-in-functions)).
//!
//! **Defaults.** A parameter of a function expression may have a default,
//! `greeting: String = "hello"`, in any position, before, between or after
//! parameters without one. A default is any expression. One that is a
//! *constant*, a literal (a string without interpolation, raw or not, an
//! integer, a float, `true` or `false`), `-` applied directly to an integer
//! or float literal, or a list literal without spreads or a record block of
//! fields `NAME = CONSTANT` (of one name each, not annotated; `{}` included)
//! whose elements or fields are constants, nested freely
//! ([`crate::ast::is_constant`]), is checked against its parameter's type,
//! as resolved for the function before any inference (the either shape of
//! an impure function's bare parameter, which a constant fits as the
//! present value it is), as an annotated initialiser is against its
//! annotation, in the function's own frame before its parameters are
//! declared (`Checker::check_defaults`): so `[]` is any list, an integer is
//! a `Float` where one is expected, and a mismatch is the usual error, at
//! the default, "expected because of this annotation" at the parameter's
//! type. What it is once checked is kept as a [`Constant`], an owned value
//! that names no node of any tree, in [`FnResolution::defaults`], as
//! [`ParamDefault::Constant`].
//!
//! Any other default is its parameter's *default function*, which the
//! parser makes (see the parser's [Functions and
//! calls](crate::parser#functions-and-calls)): a function of the parameters
//! before it, as they are written, whose result type is the parameter's,
//! whose effect is the function's (a memoised function's is impure), whose
//! marker is the function's, and whose body is the default. It is checked
//! (`Checker::check_default_fn`) where the function expression is, before
//! the function's own frame is entered, as a function written there would
//! be: so it sees the function's lexical scope, what is declared where the
//! function is written, and captures from it, and the parameters before
//! its own, with the types the function gives them. It cannot see its own
//! parameter or a later one: a read of one is "undefined variable `b`",
//! labelled "`b` is a parameter from here on" at the first of them and
//! noted "a parameter's default sees only the parameters before it". Its
//! body is checked against the parameter's type as a constant default is,
//! and with its effect, so a pure function's default is pure code: calling
//! an impure function in it, or awaiting, is the usual purity error,
//! labelled at the function's `:`. A pure function's default's bare
//! parameters are inferred as any pure function's are (see [Inferred
//! parameters](#inferred-parameters)), and what it measures its function
//! measures, as if its body did, before the body: the default runs on the
//! arguments before the body does, so `fn f(a: Int, b: Int = a * 2): {v:
//! Int} { v = b }` takes a present `a`, and the call-site error for a
//! future there is labelled at the default's measuring use. Its closure is
//! made where the function's is, and kept in the function's closure after
//! the function's own captures: [`FnResolution::defaults`] has, for its
//! parameter, [`ParamDefault::Function`] with the index of that capture. A
//! top-level function's default's copies are the function's own, which
//! decide where its closure is made (see [Top-level
//! functions](#top-level-functions)), and what a default mentions the
//! function mentions (see [`crate::mentions`]). The function's type says
//! nothing of defaults: `fn(name: String, greeting: String = "hello")` is
//! `fn(name: String, greeting: String)`, wherever it is shown.
//!
//! Only a call that knows the function gives defaults: one of a *known
//! callee* (`KnownCallee`, `Checker::known_callee`), whose callee is the
//! name of a binding initialised directly with the function expression (a
//! `let`, a named `fn` item, a field or a `def`, but not a mutable
//! variable), a `def` named by a dotted path called by its path, a function
//! of an earlier session input called by its name, or a `def` of another
//! file reached through an import or by its path, or by its name from
//! another file of its directory chain. Positional arguments, the trailing
//! block and named arguments fill parameters as above; then each parameter
//! left unfilled that has a default is given it (`Checker::fill_defaults`),
//! and only the parameters left unfilled after that are reported, as
//! before, so `pad(1)`, with `def pad(n: Int = 0, s: String)`, fills `n`
//! and is "expected 2 arguments, found 1". The parameters a call gives
//! their defaults, with their defaults, are recorded by the call's id for
//! lowering ([`Resolution::call_defaults`]), which passes each as an
//! argument, so the callee sees an ordinary full list of arguments: a
//! constant as its value, and a default function's result, which the call
//! computes on its own side once the arguments before it are known, before
//! the callee's body starts, each time it leaves the parameter out (see
//! [`crate::eval`], "Calls"). A call
//! through any other function value (a parameter, `let g = greet`, an
//! element of a list, a mutable variable, a def-type reference, `DT PATH`,
//! whose function is not the def-type's) fills every parameter, as ever.
//! Nor does a recursive call, through the function's own name in its body
//! or through a member of the group being checked (see [Top-level
//! functions](#top-level-functions)), which has no known callee. A function
//! of an earlier input, or a `def` of another file, has its defaults
//! carried, beside which parameters its body measures, as a
//! [`KnownParams`], in the session's [`Scope`] or the workspace's cell of
//! the `def`.
//!
//! **Field defaults.** A field of a record type may have a default too,
//! `type Install = {from: String, exclude: List<String> = []}`, wherever a
//! record type is written: a `type`, an annotation, a parameter's or a
//! return type, a list, function or union type, another record type. It
//! is a constant by the rule above (`Checker::is_constant`); anything else
//! is "a field's default must be a constant", at it, with the same note.
//! It is checked against its field's type as a parameter's default is
//! against its parameter's, in the mode the type is resolved in (the
//! either shape of an impure function's bare parameter taking it as the
//! present value it is), the usual mismatch at it if it does not fit
//! (`Checker::field_default`), each time the type it is written in is
//! resolved, as the rest of the type is (a top-level function's signature,
//! resolved first in silence, reports it once). What it
//! is once checked, a [`Constant`], is part of the record type
//! ([`crate::types::RecordTy::defaults`]); a record type with an erroneous
//! default is erroneous. A declared type's definition is copied into each
//! tree using it apart from its own (see [Type
//! declarations](#type-declarations)), its defaults as the constants they
//! are written as, with no type known (`written_constant`): there each is
//! fitted to its field's type as `check_expr_expecting` would check it
//! (`Checker::fit_constant`: an integer a `Float` where one is expected, a
//! record given the defaults of its own type), with nothing reported, the
//! declaration's own file reporting what is wrong; one that is no
//! constant or does not fit fails the check quietly.
//!
//! A record literal (a record block, or `{}`, the empty record's, through
//! any parentheses) checked
//! against an expected type that is, once stripped of the either shape of
//! an impure function's bare parameter or result, a record type with
//! defaults, which it lacks fields of, by a field item or a spread whose
//! type has the field, is *given* each such field: the field is not
//! missing, the literal's type has it, of the expected type's type of it,
//! so it is read with no `is`, and the field with its default is recorded
//! for lowering by the literal's id ([`Resolution::record_fills`]), which
//! builds the record with it, so at run time the value simply has the
//! field (`Checker::report_missing_fields`). So `{}` where such a type is
//! expected is the record of its defaults if every field of it has one,
//! and otherwise each other field is missing, as for any record literal
//! (`Checker::empty_record`); where no record type with defaults is
//! expected itself, `{}` is checked as ever. That is wherever a literal is
//! checked against an expected type: an annotated `let` or field, an
//! ascription, an argument, a list's element, a field of an enclosing
//! literal, `:=`, a function's body and a `return`'s value against its
//! return type, a meta's body, and a parameter's or a field's default that
//! is a record literal, whose constant then has the fields given it. A
//! union of record types expected gives nothing, its one record member's
//! missing fields being missing as ever, nor does a dotted field's segment
//! (an open field).
//!
//! Where such a type is expected itself, it also reaches, for its defaults
//! alone, a literal that a block results in, at any depth of blocks,
//! labelled or not, through parentheses, or that a branch of an `if` with
//! `else` does, every `else if` included (`Checker::fill_result`): so a
//! function's body or a meta's whose result is a literal, as `defmeta DT
//! PATH = { ... }`'s is, a block or `if` written where the type is
//! expected, a `for`'s body where a list of the type is expected of the
//! loop (see [For](#for)), a `break`'s value for such a loop, and the body
//! of a `for` spread in a list literal whose elements are expected to be of
//! the type, `[...for x in xs { { ... } }]` (see [Lists](#lists)). Where the
//! elements of such a list literal are expected to be of the type, a spread
//! of a block with a result or of an `if` with `else` reaches, through the
//! same blocks and branches (`Checker::fill_elements`), a list literal they
//! result in, whose elements are then reached as the enclosing literal's
//! are, each an element that is no spread, as above, and each spread of a
//! `for`, a block or an `if` in turn, and a `for` they result in, whose
//! body is reached as a spread `for`'s is: `[...if c { xs } else { [{ ...
//! }] }]`, `[...{ let y = 1; [{ ... }] }]`. A
//! literal reached so that lacks a defaulted field, by its field items, and
//! has no spread is checked against the type as any literal is where it is
//! expected, field by field, and given the defaults it lacks; one that
//! gives every defaulted field, or has a spread, is checked as it is with
//! nothing expected, as ever. Nothing else of the block, the `if`, the
//! loop or the list literal changes: a result or a branch that is no
//! literal, a name or a call, is checked as with nothing expected, `[]`
//! included, the branches are joined, a list literal's elements are
//! joined, a labelled block's `return`s are checked and joined with its
//! result, and the whole must then fit the type where it stands, as ever, a
//! mismatch being reported there. A record type without defaults reaches
//! none of these, nor does one that is a union's member; nor is the result
//! of an `if` without `else`, or a spread of anything else, a list literal
//! itself included, reached. Nothing else is
//! given anything: a value that is not such a literal, a variable, a
//! call's result, a field read, a literal checked with no expected type,
//! that lacks a defaulted field is the usual mismatch where it meets the
//! type, noted "a default fills only a record literal written where `T` is
//! expected", `T` the record type, shown as types are
//! (`Checker::default_note`); nothing is converted at run time.
//!
//! Defaults do not change which values a type admits: subtyping and the
//! join ignore them (see [`crate::types`], "Defaults"), and where two
//! types must be one, as the operands of `==` must, a record type with
//! defaults is one with the record type without them
//! ([`TypeTable::same`]). Every type is shown with its defaults, after
//! their fields' types, `{exclude: List<String> = [], from: String}`, in
//! errors, hovers and the dumps; a function's type shows those of its
//! parameters' record types, which are part of them, unlike its own
//! parameters' defaults.
//!
//! **Purity.** A function declared with `:` is pure, one declared with `->`
//! impure, and one declared with `->>` impure and memoised: its [`Effect`]
//! is one of `Pure < Memo < Impure`, and only `Pure` is pure. A memoised
//! function's body is an impure context exactly as an impure function's
//! is; memoisation is the evaluator's business (see [`crate::eval`]), and
//! what the checker does differently for it is only what its bare types
//! mean (see [Futures](#futures)). The top level is impure, and a block has
//! the purity of the function it is in. Calling a function that is not
//! pure, an impure or a memoised one, from a pure one is an error, and so
//! is awaiting (see [Futures](#futures)). Those are the only gated
//! operations: a pure function may create, pass, store and return impure
//! and memoised functions and futures.
//!
//! # Top-level functions
//!
//! A *top-level function* of an input is a named function item (`fn f`)
//! or a `def` (`def f`, or `def dev.peios.f`, named by a dotted path: see
//! below) that is directly an item of the input's outermost block;
//! nothing inside a nested block, a loop body, a labelled block, a function
//! or a `lazy` is one. The input's top-level fields are order-free too (see
//! [Top-level fields](#top-level-fields)); together with its top-level
//! functions they are its *top-level declarations*. Everything else is
//! ordered as described above: `let`, `let mut`, spreads, `:=`, statements,
//! every field of an inner block, and every `fn` or `def` that is not at
//! the top level of an input.
//!
//! **Names.** A top-level function's name is in scope in its whole input,
//! above its declaration as well as below, and inside every function and
//! `lazy` of the input, unless a nearer declaration shadows it there. As
//! the input starts (`Checker::declare_top_fns`), every top-level function
//! is declared, in source order, with the type its signature declares, so
//! its binding comes first among the input's bindings and its slot is one
//! of the input's first. The input's other bindings are declared after
//! them as the walk reaches them, so a scope's bindings still occupy slots
//! in order, and a session's [`Scope`] keeps that order. A top-level
//! function hides a variable or field of its name from an earlier input,
//! and a soft field of its name that a spread of this input defines (the
//! spread still defines the record's field; a `def` of that name overrides
//! it, as a field does). Two top-level `fn`s of one name are "function `f`
//! is declared twice", at the second (two `def`s, or a `def` and a field,
//! are the field defined twice); a top-level `let`, `let mut`, field or
//! `:=` whose name, or its path's first segment, is a top-level function's
//! is "`f` is a function of this file", but a field all of whose leaves
//! are `#[shadowable]`, which a `fn` of its name shadows in the whole input
//! (see [Shadowed declarations](#shadowed-declarations)). In inner blocks a
//! name shadows as it always has.
//!
//! **What a function sees.** A function sees the variables and fields
//! declared above its own declaration and copies the ones it uses when it
//! is made, as any function does. A top-level variable declared below it
//! is "undefined variable" inside it, with a label on that declaration,
//! "`k` is declared here, below the function". What is new is only how one
//! top-level function refers to another of its input, or to a top-level
//! field of it: through the top level's frame, by [`VarRef::Fn`], never by
//! a capture. That is what lets two functions refer to each other, and a
//! closure never wait for a field.
//!
//! **Mentions.** What follows is decided from one relation, found from the
//! tree before the input is checked (see [`crate::mentions`]): an item or
//! a body *mentions* a top-level function when a name in it is the
//! function's and no declaration the tree shows shadows it there. A name
//! that a spread inside a nested block defines is not known before
//! checking, so it does not shadow: a read of it counts as a mention
//! although it reads the spread's field. The relation only ever
//! over-approximates, which can only make a body be checked earlier or an
//! item be held to the rule of [When a function may be
//! used](#when-a-function-may-be-used).
//!
//! **Groups, and the order bodies are checked in.** A caller needs more
//! than a callee's written signature (what the body decides: inferred
//! parameters, an inferred result, narrowed callbacks), and that must not
//! depend on which is written first. The top-level functions are grouped
//! by mutual recursion, the strongly connected components of "mentions"
//! between them, and every body is checked exactly once
//! (`Checker::check_group`): before the walk checks a top-level item that
//! is not itself a top-level function, the bodies of the functions it
//! mentions, and of those they mention, and so on, that are not checked
//! yet (`Checker::check_mentioned`); after the last item, the rest
//! (`Checker::check_remaining_bodies`). In both, the groups a group
//! mentions come first, then the group's members in source order. A use of
//! a top-level function from outside its group sees the function fully
//! resolved, exactly as a use below a function's declaration does. A use
//! from inside its group, its own body included, sees the declared type
//! and is treated as a recursive call: the declared type at the bare
//! positions, arguments measured conservatively, no `KnownCallee`,
//! nothing iterated. A function whose signature or body is erroneous is,
//! for its users, an erroneous function declared above them.
//!
//! **The view a body is checked in.** A body checked before the walk
//! reaches item `P` sees, of the input's top level, the bindings declared
//! above whichever is higher, `P` or its own declaration, and every
//! top-level function, and everything of earlier inputs; a body checked
//! when the walk reaches it, or after the last item, sees the bindings
//! above its own declaration, which is what a function always saw. The
//! bindings are in declaration order, so the view is a prefix, and
//! `Checker::check_top_body` makes it by setting the suffix aside while the
//! body is checked and putting it back after: slots and `measured` are not
//! touched. A body checked at an item above its own declaration reads
//! nothing declared between the two: a read of a name that a top-level
//! item between them declares (by `let`, a field, a dotted path's first
//! segment, or a `:=` of a variable that is not mutable) is not resolved
//! to an older binding but makes the function not usable at the item (see
//! below), and so is any read of a variable or field of the top level when
//! a top-level spread lies between them, since its names are not known
//! yet. That spread rule is conservative: it holds whether or not the
//! spread defines the name.
//!
//! **Dotted names.** A `def` named by a dotted path, `def
//! dev.peios.count(n: Int): Int { ... }`, is exactly a `def` whose name is
//! its path, in every rule above and below: what differs is only where its
//! function is kept and how it is read. It is the leaf of its path in the
//! tree of top-level fields (see [Top-level fields](#top-level-fields)), and
//! has no binding, since no variable could name it: the leaf's cell, a slot
//! of the top level's frame after the roots', holds its closure, stored
//! where the closure is made, and holds its type
//! (`Checker::type_dotted_defs`): the type its signature declares, and,
//! once its group is checked (`Checker::finish_group`), the type its body
//! gives it, as a plain function's binding has. A use of it is a read of
//! its path, a reference whose longest declared prefix is its leaf, which
//! reads the cell (`Checker::field_reference`): from top-level code a slot
//! of the frame, from inside any function or `lazy` [`VarRef::Fn`] of it,
//! the node a plain function's mention is, so mutual recursion with any
//! top-level function works as between plain ones, and a call from inside
//! its group is treated as a recursive call (`Checker::is_recursive`, by the
//! cell's slot). Once its group is checked, a call by its path is of a
//! known callee, as a call by a plain name is (`Checker::known_callee`).
//! There is no short self-name: inside `def dev.peios.count`, `count` is
//! whatever else binds it, if anything. Its diagnostics name it by its
//! path ("`dev.peios.count` cannot be used here yet"). When the walk
//! reaches it, its path joins the record of the input as any dotted field's
//! does, its leaf typed as the function is by then and retyped when its
//! group is checked, and its root's name clashes as a dotted field's
//! (`Checker::reach_dotted_def`); one whose leaf today's rules reject (a
//! leaf twice, a leaf under a leaf or over another) is no top-level
//! function but the field it is written as, reported as such. A read of a
//! prefix of its path whole (`dev.peios`, `dev`) is a record of the leaves
//! under it, its closure among them: it mentions the function (see
//! [`crate::mentions`], "Dotted `def`s"), so the read waits for the
//! function as a use of it does, and the function's group is checked before
//! the reader. The prefix's type is the record of its children's when it is
//! computed, which may be before that group is checked, so when the check
//! changes the function's type, every node computed above its leaf is
//! retyped (`Checker::retype_above`), before anything reads it; a node of a
//! workspace file's view above it too. A function that reads a prefix of
//! its own path whole is no cycle: the record holds its closure, which
//! reads the record when it runs.
//!
//! # When a function may be used
//!
//! Top-level code (the initialiser of a `let` or a field, a spread, a `:=`,
//! a statement, the result) can only use a function that exists by then,
//! and a function exists once it has copied what it reads. From the
//! captures its checked body resolved (not from the tree), each top-level
//! function is made ([`MadeAt`]):
//!
//! - at the start of the input, if it copies nothing from this input's top
//!   level (what it copies from an earlier input is bound before the input
//!   starts, and does not count);
//! - just after the item that binds the last of what it copies, if it
//!   copies only variables that are not mutable, and fields: the latest
//!   definition of each name above the function;
//! - at its own declaration, if it copies a mutable variable, whose value
//!   it copies there, as any function does.
//!
//! A function nested in a top-level function, or a `lazy` in it, that
//! reads a top-level variable makes the top-level function copy it. A
//! top-level function *needs* every function it mentions, and what those
//! need, and so on. A top-level item that mentions a top-level function
//! anywhere in it, even inside a function expression or a `lazy` it only
//! builds, is "`f` cannot be used here yet" unless every function `f`
//! needs exists before the item starts (`Checker::check_uses`). The label
//! names the reason first in source order and the shortest chain to it:
//! "`f` reads `k`, which is bound here", "`a` uses `b`, and `b` reads `k`,
//! which is bound here", up to three names, and "`a` uses `b`, ..., and
//! `z` reads" beyond; for a mutable variable it is on the function that
//! copies it, "`b` reads the mutable variable `n`, so it exists from here";
//! for a read blocked by a spread, "`g` reads `port`, which this spread may
//! define". There is one error per item and function. Inside function
//! bodies there is no such check: the rule, applied where top-level code
//! mentions the function whose body it is, covers what it calls. The rule
//! asks what is mentioned, not what is called, which is conservative, and
//! deliberately so.
//!
//! The "this is never run" warning neither starts at nor counts a
//! top-level function, since its closure may be made before the item that
//! never completes.
//!
//! # Top-level fields
//!
//! A field declared directly at the top level of an input, plain (`a = e`),
//! quoted (`"a b" = e`), dotted (`a.b.c = e`) or a `def` (`def f`, `def
//! a.b.f`), is a *top-level field*, and, like a top-level function, it is
//! visible in the whole input.
//!
//! **The tree.** The top-level field items form a tree by their paths
//! ([`crate::mentions::FieldTree`], built before the walk): each item is a
//! *leaf*, and every proper prefix of a leaf's path an *interior* node, the
//! record of its children in the order of their first items. Every node has
//! a *cell*, a slot of the top level's frame written once: a root's is its
//! binding's slot, declared as the input starts, after the top-level
//! functions (`Checker::declare_field_cells`; a `def`'s of one name is its
//! function's), and every other node's is a slot after those that no name
//! finds, a dotted `def`'s holding its closure. The
//! rules of [Dotted fields](#dotted-fields) that keep a leaf from also
//! being interior hold in either order of the two items.
//!
//! **References.** A *reference* to a top-level field is `NAME`, `NAME.f`,
//! `NAME.f.g`, ..., a variable and a chain of field accesses directly on
//! it, where `NAME` resolves to a top-level field root of this input. It
//! means the longest declared prefix of its path, a leaf or an interior
//! node, whose type it has (`Checker::field_reference`); the segments after
//! that prefix are ordinary field accesses. So `com.x.lib` reads only what
//! is under `com.x.lib`, not all of `com`. Anything else (`(com).x`,
//! `f(com).x`) means the whole root.
//!
//! **Names.** A field root's name follows the rules for a top-level
//! function's: it hides an earlier input's binding of the name, and
//! overrides a soft field of the name from a spread of this input, above it
//! or below (see [Spreads and soft
//! assignment](#spreads-and-soft-assignment)); a top-level `let`, `let
//! mut`, `:=` or dotted `let` of the name is "`a` is a field of this file",
//! with the field's first item labelled "declared here"; a top-level
//! function of the name is "`a` is a function of this file" at the field.
//! A root all of whose leaves are `#[shadowable]` is shadowed instead,
//! silently, by a `let` (a dotted one too) from its item on and by a `fn`
//! in the whole input (see [Shadowed
//! declarations](#shadowed-declarations)).
//!
//! **What an initialiser sees.** As a function body does: the variables of
//! the input's top level declared above the field's own item, everything of
//! earlier inputs, and every top-level declaration. A read inside a function
//! expression or a `lazy` the initialiser builds counts as the
//! initialiser's. A `let` below the field is "undefined variable", labelled
//! "`k` is declared here, below the field", with the note "a field's
//! initialiser sees the variables declared above it".
//!
//! **Points.** Each node has a *point*, the item of the outermost block
//! before which its cell is computed ([`crate::mentions::Schedule`]). An
//! ordered item (anything that is not a top-level declaration) *needs* every
//! top-level declaration it mentions and what those need: a leaf what its
//! initialiser mentions, an interior node its children, a function what its
//! body mentions. A leaf's point is the earliest of its own item and the
//! points of what needs it, directly or through functions; an interior
//! node's is the earliest of the item after its last child's and the points
//! of what needs it. A function's closure keeps its own rule ([When a
//! function may be used](#when-a-function-may-be-used)) and never pulls a
//! field earlier: a closure reads fields when it runs. Before each item,
//! the nodes whose point it is are computed, dependencies first, otherwise
//! in source order; after the last item, the rest. So a field nothing
//! mentions earlier is computed at its own line, and one mentioned earlier
//! just before the first item that needs it; the effects of its initialiser
//! happen there, which may be above its line. A field computed before its
//! own item is not code that runs at its line: the "this is never run"
//! warning neither starts at nor counts it; one computed at its own item is
//! warned about and counted as any item. Between the files of a workspace,
//! the order is by file: a file's effects all happen before the first
//! effect of a file that needs it ([Directory scope](#directory-scope)).
//!
//! **When a field may be used.** A leaf may be computed at a point only if
//! what it reads is bound by then, as a function may be used only once what
//! it copies is bound. Whatever needs, at its point, a leaf that cannot be
//! computed by then (one that reads a top-level variable bound at or after
//! that point, or a mutable variable, which makes the leaf computed at its
//! own item with the value the variable has there, or, through the chain, a
//! function or leaf that is not ready) is "`a.c` cannot be used here yet",
//! at the mention, with the chain of [When a function may be
//! used](#when-a-function-may-be-used) and "has" for the steps from an
//! interior node to what it holds: "`com` has `com.x.app.name`, and
//! `com.x.app.name` reads `version`, which is bound here". The rules for a
//! name declared, or a spread, between an early use and the declaration
//! apply to leaves as to functions. `Checker::check_uses` skips the search
//! for a reason when nothing a mention needs is ever blocked
//! (`Checker::closure_ready`), so a chain of N fields costs N steps.
//!
//! **Cycles.** Mutual recursion between functions is fine, but a cycle of
//! "needs" through a leaf or an interior node is "`a` depends on itself",
//! at the first item of the cycle's leaf earliest in the source, labelled
//! along the cycle ("`a` uses `b` here", ...), once per cycle, with the
//! note "a field cannot be computed from itself, even through a function"
//! (`Checker::report_cycles`, from [`crate::mentions::Mentions::cycles`]).
//! The nodes on a cycle are erroneous for their users. The rule asks what
//! is mentioned, not what is called, and a mention inside a `lazy` or a
//! function expression the initialiser only builds counts, so `steps = lazy
//! { run(steps.name) }` is an error: a function that reads the field is the
//! way to write it. It is conservative in another way too: the first
//! segment of a dotted field of an inner block counts as a mention of the
//! top-level root of that name, so `a.c = { a.b = 1 }` is a cycle.
//!
//! **Types: initialisers are checked when they are needed.** A field has no
//! declared type, which is why a cycle through one is an error rather than
//! a conservative view. (An annotated field has one, its annotation, which
//! is its type; a cycle through one is an error all the same.) Each leaf's
//! initialiser is checked exactly once, against its annotation if it has
//! one (`Checker::compute_fields`, walking the schedule's order with no
//! recursion): before the first item that needs it, after the
//! declarations it needs, or after the last item, with the top-level scope
//! cut to the bindings above whichever is higher, that item or the field's
//! own, as a body checked early is (see [Top-level
//! functions](#top-level-functions)). An interior node's type is the record
//! of its children's types. A reference has its node's type.
//!
//! # Shadowed declarations
//!
//! A top-level field or `def` marked `#[shadowable]` may be shadowed by a
//! later one it overlaps, in this input or, in a workspace, in a later
//! file of its directory (see [`crate::workspace`], "Shadowable
//! declarations", and [`crate::mentions`], "Shadowed declarations"). The
//! one that stands is the leaf for the whole input, a read above it
//! included, so it cannot read the one it shadows (`#[shadowable] a = 1`
//! then `a = a + 1` is "`a` depends on itself"). A shadowed declaration is
//! in no tree and no top-level function: the walk passes it by
//! (`Checker::set_aside_shadowed`) and, once the walk is done, checks its
//! value as it is written (`Checker::check_shadowed_items`), declaring
//! nothing, in the view of the top level above its own item, as a body
//! checked late is, a `def`'s function seeing itself by its name (one named
//! by a dotted path has none: its path reads the declaration that stands). So its
//! own errors are reported. It needs nothing, so what it alone would read
//! may not be computed, or checked, before it: a cell of a file this one
//! does not otherwise need has no type yet, and a record of an import only
//! it reads whole is not assembled; each is erroneous for it, which
//! suppresses the errors that would depend on it. It is never lowered (its
//! functions have code that does nothing, since none is made), so its
//! erroneous types break nothing ([`Checker::check`]'s last check exempts
//! its expressions). A top-level `let` or `fn` of the name of a field all
//! of whose leaves are shadowable shadows it in this input, silently: the
//! `let` from its item on, as it does any variable, the `fn` in the whole
//! input, its binding declared after the field roots'
//! (`Checker::declare_shadowing_fns`). The field is still a field of the
//! input's record and, in a workspace, of its directory, which commits
//! only fields ([`Scope`]'s `commit`): a `let` or `fn` is the file's own.
//!
//! # Def-types
//!
//! A `def` marked `#[def_type]` (see the parser's
//! [Attributes](crate::parser#attributes)) is a *def-type*, as in
//! `#[def_type] def package(name: String, func: fn(): PackageDef) -> R`. Its
//! first parameter must be a `String` and its second a function, its type
//! F, present or the either shape of one: the function type of the
//! *definitions* made with it; its further parameters, if any, are its
//! *invocation parameters*, and its effect and result are free. Otherwise it
//! is "a def-type takes the name as a `String` and then the definition as a
//! function, as in `def package(name: String, func: fn(): T)`", the
//! def-type's own name in place of `package`, at its parameters
//! (`Checker::check_def_type_signature`). It is still an ordinary `def`,
//! called as any is.
//!
//! **Definitions.** `def DT PATH(PARAMS) BODY` ([`ItemKind::DefTypeDef`])
//! binds nothing a variable or a field names. Its function is a leaf of
//! DT's *namespace*, a root of the tree of top-level fields that no source
//! can write, the namespace's key (see [`crate::names`], "Namespace keys"),
//! which the workspace finds once it knows what DT names (see
//! [`crate::workspace`], "Def-types"). So it is a top-level function in
//! every sense a `def` named by a dotted path is (see [Top-level
//! functions](#top-level-functions)): usable above it and from every file
//! that sees it, in its group, its closure made where the rules say, its
//! leaf's cell holding it. Its type is F: as the input starts, its `DT` is
//! read quietly and its leaf's cell given F (`Checker::definition_type`),
//! after the other dotted `def`s', so that a def-type may be one. Its body
//! is checked (`Checker::check_definition`), its `DT` read again and what
//! is wrong with it reported, as a function of F's parameters, which
//! PARAMS name in order, against F's result, with F's effect: so with
//! `fn(): PackageDef` it is pure and makes a `PackageDef`. Nothing is
//! inferred of it, no parameter, no result, no callback narrowed: its type
//! is F exactly. Parameters not as many as F's are "`package` defines
//! functions of no parameters" ("of 1 parameter, `x`", "of 2 parameters,
//! `a` and `b`", naming them if F names them all), at its parameter list.
//!
//! When the walk reaches a definition (`Checker::place_definition`), it is
//! placed in its namespace as the dotted field of its path under the key is
//! placed in a record, but among the namespaces' own fields, apart from the
//! input's record: so a file of definitions alone has no fields, and two of
//! one path, one under another and the like are what the rules of dotted
//! fields say they are, the path shown as the definition writes it ("field
//! `package x` is defined twice"). One whose DT is no def-type is "`DT` is
//! not a def-type", noted "a def-type is a `def` marked `#[def_type]`", and
//! its body is not checked, having no type to be checked as. A shadowed
//! definition is checked as written (see [Shadowed
//! declarations](#shadowed-declarations)), but one whose def-type it cannot
//! read, its file not otherwise needing the def-type's, is not: its types
//! are the def-type's to give. Where an error names a definition as what
//! cannot be used yet, or along the chain of uses that says why, it is shown
//! by its def-type and its path, as written, "`package dev.peios.x` cannot be
//! used here yet" (`Checker::need_text`).
//!
//! **References.** `DT PATH` ([`ExprKind::DefTypeRef`]) is a function: of
//! the def-type's invocation parameters, with its effect and its result,
//! and the meta of F, if F has one, so that it carries its definition's
//! (see [Metas](#metas), "Through a reference")
//! (`Checker::check_def_type_ref`). Its DT is read as any value is, and must
//! name a def-type: "`DT` is not a def-type" otherwise, noted as above and
//! "two names in a row are read as a def-type reference, as in `package
//! dev.x()`; is an operator or a line break missing?", with which the
//! "undefined variable" error of an undefined DT is noted too. A DT that
//! names a parameter or a local nearer than the input's top-level
//! declarations is no def-type, whatever the workspace found its name to
//! name at the top
//! level (`Checker::shadows_def_type`). Its path must name a definition of
//! the namespace that the file sees (`Checker::definition_at`): a leaf of
//! the view under the key, or, in a session, of an earlier input's
//! namespace, a binding of the key, by its record type. A path that names
//! nothing is "`package` has no definition `dev.peios.nope`", with a note
//! listing those the file sees ("the only definition of `package` here is
//! `x`", "the definitions of `package` here are `a` and `b.c`", "`package`
//! has no definitions here"); one that names a node above definitions,
//! "`dev.peios` is not a `package` definition", at the path. For a def-type
//! whose F has a meta, a path that goes on past a definition names it, the
//! rest of the path read from its meta (see [Metas](#metas)); for any other
//! the whole path must name one, as here. The
//! definition is then read as its leaf's cell is, by its path
//! (`Checker::field_reference`). A call of a reference is a call of a
//! function value, of no known callee: calling one of an impure def-type
//! from a pure function is the purity error. A record read whole through
//! an import holds no namespace (`Checker::imported_record`).
//!
//! # Attributes
//!
//! An attribute says how to run a function, for the host that runs it or
//! for GCSL: its arguments are a record of constants, and so an attribute
//! is declared as the record type of its arguments, a `type` marked
//! `#[attribute]` (see the parser's [Attributes](crate::parser#attributes)),
//! `#[attribute] type hermetic = {on: Bool = true}`. Being a type, it is
//! named, imported and reported in conflicts as any type is (see
//! [`crate::workspace`], "Types"), and is usable as one. Its type must be a
//! record type, with no rest-field, whose fields hold only constants:
//! `Int`, `Float`, `Bool`, `String`, `Duration`, `Size`, or lists and
//! records of them (`Checker::holds_constants`), "an attribute's arguments
//! are a record of constants" otherwise, at its definition, noted with what
//! a field may be; and it may not be named like a built-in attribute
//! ([`BUILTIN_ATTRIBUTES`]), "`retry` is a built-in attribute".
//!
//! `retry`, `{times: Int}`, and `timeout`, `{limit: Duration}`, are the
//! checker's own, known in every file, whatever its types.
//!
//! Each use on a function item (`Checker::check_attribute_uses`), checked
//! before the file's body, names one of the checker's or a type of the
//! file's view: "unknown attribute `x`" for none, with a suggestion among
//! the attributes in view and the checker's, and "`PackageDef` is not an
//! attribute", noted "an attribute is a `type` marked `#[attribute]`", for
//! a type that is not marked. Its arguments fill its record's fields: one
//! positional argument for an attribute of one field, and named ones for
//! any, "`#[device]` takes its arguments by name, as in `#[device(name =
//! ...)]`" for a positional one of another; "`#[retry]` takes one
//! argument", "`#[retry]` has no argument `x`", "`times` is given twice"
//! and "`#[retry]` takes no block" otherwise. Each must be a constant, by
//! the rule for constant defaults, "an attribute's argument must be a
//! constant", and is checked against its field's type as a constant
//! default is, "expected by this attribute" labelling the attribute's
//! name. A field given nothing takes its default, and one with none is
//! "`#[device]` needs `name`". `#[hermetic()]` is `#[hermetic]`. What the
//! arguments take of the frame's slots is given back: an attribute is never
//! run.
//!
//! A function's attributes ([`Attribute`]), each what a host knows it by
//! ([`AttributeId`]) and its arguments, every field by name with the
//! constant given or its default, are recorded by its function, in source
//! order ([`Resolution::attributes`]), for lowering to give its code
//! ([`crate::ir::FnCode::attributes`]). A definition made with a def-type
//! takes the def-type's attributes as defaults (`Checker::inherit_attributes`):
//! the def-type's, each replaced whole by the definition's own of the same
//! attribute, then the definition's others; a def-type of another file or
//! of an earlier session input is known by what its cell or binding
//! carries ([`KnownParams`]). Plain GCSL gives a host's attributes no
//! meaning; `retry` and `timeout` are checked and carried, and run as yet as
//! any function is.
//!
//! # Metas
//!
//! A function type may have a *meta*, written after its return type with a
//! marker of its own (see the parser's "Types"), as in `fn(v: Version):
//! PackageDef META-> PackageMeta`: a record attached to the function value,
//! whose fields are read as the function's. Its type is resolved as a
//! return type of its marker's effect is (`Checker::resolve_meta`): written
//! bare, it is the either shape under `META->` and `META->>`, at its
//! position and every bare position inside it, and present under `META:`,
//! so a `META: Future<X>` meta is a future. It must be a record type once
//! every `Future` is taken off it: "a meta must be a record type: its
//! fields are read from the function" otherwise, at the type. A function
//! type with a meta is a subtype of the same without one, and metas compare
//! as results do (see [`crate::types`], "Metas"); a function expression, a
//! `def` and a built-in function have no meta, so none fits a type with one.
//!
//! **Reading.** A field access on a value of a function type with a meta,
//! once a value that may be a future is awaited as any record a field access
//! reads is, reads the meta's field (`Checker::meta_read`): `func.versions`
//! is the meta's field `versions`, of the field's type, and `func.a.b` goes
//! on from it. A field the meta lacks is "the meta of `func` has no field
//! `x`", at the `.NAME`, noted with the meta's fields ("the only field is
//! `versions`"); a field access on a function with no meta is the error it
//! always was, "cannot access field `x` of `fn(): Int`". A read is a call of
//! the meta's closure, of the meta's effect: from a pure function, reading a
//! `META->` or `META->>` meta is "cannot read an impure meta from a pure
//! function", labelled "reading the meta of `func` is impure (`META->`)" at
//! the function and at the pure function's marker, noted as the call error
//! is; a memoised function may read one wherever it may call an impure
//! function. A meta that may be a future, the either shape of an impure
//! meta's or a `Future`, is then awaited at the top, as a record a field
//! access reads is, subject to the purity rule, unless the read is already
//! that error. What a read reads, and how its meta is awaited, is recorded
//! for lowering ([`Resolution::meta_read`]).
//!
//! **Declaring.** A def-type declares the meta of its definitions by the
//! meta of its definition parameter's function type, F. `defmeta DT PATH {
//! ... }` or `defmeta DT PATH = EXPR` ([`ItemKind::DefMeta`]) declares the
//! meta of the definition `def DT PATH(...)` of its directory: a function of
//! no parameters, its type the function of F's meta's effect returning F's
//! meta's type, its body checked as such a function's
//! (`Checker::check_defmeta`), so under `META->` it may `exec` and under
//! `META:` it is pure. Its `DT` is read as a definition's is ("`DT` is not
//! a def-type" otherwise); a def-type that declares no meta is "`package`
//! declares no meta for its definitions", noted "a def-type declares one by
//! its definition parameter's type, as in `func: fn(): PackageDef META:
//! PackageMeta`", at the meta's `defmeta` through its path. A meta binds
//! nothing a source can read: it is a top-level function, as a definition
//! is, and the leaf of its path, as one name, under the meta key of its
//! def-type and its file's directory (see [`crate::names`], "Namespace
//! keys"), placed among the namespaces' fields as a definition is
//! (`Checker::place_definition`) and typed as the input starts
//! (`Checker::defmeta_type`).
//!
//! **Pairing.** A definition whose def-type declares a meta reads the cell
//! of its meta, the leaf of its path among its directory's metas, as a
//! reference to it would, from inside its own function, as its body reads a
//! top-level function (`Checker::definition_meta`), and the slot that read
//! reads is what its code names ([`Resolution::metas`]), which a read of its
//! closure's meta calls; so the definition needs its meta, as mentions say
//! ([`crate::mentions`], "Def-types"). One whose directory declares no meta
//! of its path, which the workspace finds ([`crate::workspace`],
//! "Def-types"), is "`package dev.peios.x` has no `defmeta` in its
//! directory", at its def-type and path, noted "`package` declares a meta,
//! `META-> PackageMeta`, for each of its definitions": the meta as the
//! def-type's parameter writes it if the workspace has the def-type's tree,
//! and otherwise as its type shows, present under `META->` and `META->>`,
//! as a bare type would be written there. A meta declared twice in a
//! directory, or with no definition of its path there, is the workspace's
//! error. A definition reads nothing, its read typed `{}`, when its def-type
//! declares no meta, when its directory declares none, and when it is
//! shadowed, since it is never made.
//!
//! **Through a reference.** A reference to a definition, `package
//! dev.peios.testpkg`, carries the definition's meta: its type is the
//! function of the def-type's invocation parameters with F's meta, its marker
//! and type, `fn(version: String) -> R META-> PackageMeta` (see
//! [Def-types](#def-types)), so `(package dev.peios.testpkg).versions`, or a
//! reference stored, `let r = package x` then `r.versions`, reads it as any
//! function's meta is read, the errors naming the reference as written ("the
//! meta of `package dev.peios.testpkg` has no field `x`"). Without the
//! parentheses, a reference means the longest prefix of its path that names
//! a definition, since no definition lies under another
//! (`Checker::definition_at`): the rest of the path is read from that
//! definition's meta, `package dev.peios.testpkg.versions` its field
//! `versions`, each further segment a field of the one before. Only a
//! def-type whose F has a meta reads a path so: for any other, `package x.y`
//! names the definition `x.y`, as it always did, and a path no prefix of
//! which is a definition is the error it was. The parser reads the whole
//! path, and the reference's read is a field access for each of its
//! segments, so the accesses past the definition are typed as the field
//! accesses they are, the first reading the meta of the definition's cell
//! (`Checker::read_past`), with the meta read's errors, effect and awaiting,
//! a field the meta lacks noted "`package dev.peios.testpkg.x` is read as
//! the field `x` of the meta of `package dev.peios.testpkg`, its longest
//! prefix that is a definition". The reference has the type of the last of
//! them, and what it reads past, with the type of the reference to the
//! definition, is recorded for lowering and the dump
//! ([`Types::reference_past`]). A read through a reference and a read
//! through the definition, as the def-type's `func.versions`, read the one
//! meta, one closure in one cell: so a `META->>` meta's body runs once for
//! both, and a `META->` meta's at each. A read has the meta's effect wherever
//! it is, allowed at the top level of a file, an error in a pure function.
//! It needs what reading the reference needs, the definition, and so, as the
//! definition reads its meta, the meta.
//!
//! # Futures
//!
//! `lazy e` is checked as the body of an anonymous, parameterless function
//! (a `Function` of its own on the stack, with a frame of its own, capturing
//! as a closure does), which is always **impure**, wherever it is written;
//! its type is `Future<T>`, `T` being the body's type, an untyped integer
//! settled to `Int`, with every future layer at its top stripped, since
//! awaiting resolves fully (so `lazy lazy 1` is `Future<Int>`, and so is
//! `lazy f()` where `f()` is `Int | Future<Int>`; see
//! [`TypeTable::future`]). The name a `let` or a field binds is not in scope in
//! its own `lazy` initialiser, as for any initialiser.
//!
//! **Bare types.** In a type expression, a type written as a name, a
//! record type, a list type (`List<T>` or `T[]`) or a union with no member
//! written `Future<...>` means what the position it is written in says
//! (`Bareness`):
//! the either shape, `T | Future<T>`, in the parameters and the return type
//! of an impure function expression and of a written impure function type,
//! and in the return type of a memoised one, at that position and at every
//! bare position inside it, through record and list types (so `{a: Int}`
//! there is `{a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>`, and
//! `List<Int>` is `List<Int | Future<Int>> | Future<List<Int | Future<Int>>>`)
//! but not into a
//! nested function type, which has its own rule; and a present `T`
//! everywhere else: annotations, ascriptions, the return types of pure
//! functions, the parameters and return types of written pure function
//! types, and the parameters of memoised function expressions and written
//! memoised function types (see `bareness_of`). A memoised function is
//! strict: its arguments are the key its results are memoised by, so each
//! is present, at its position and at every position inside it, and none is
//! inferred; `Future<T>`, `T | Future<T>` and `Present<T>` may still be
//! written there and mean what they say. A caller passing a value that may
//! be a future to such a parameter awaits it at the call, deeply where the
//! parameter's type needs it, as for any present parameter. (A pure
//! function expression's bare parameters are inferred: see [Inferred
//! parameters](#inferred-parameters); and so are an impure or memoised
//! function expression's bare result positions, from what its body
//! returns, once its body is checked against the either shapes written:
//! see [Inferred
//! results](#inferred-results).) A function type is the one bare type
//! that is never the
//! either shape at its own position: its interior follows its own purity
//! marker, and a function value that may be a future must be written so,
//! as `Future<fn(): Int>` or `(fn(): Int) | Future<fn(): Int>` (in
//! parentheses, since a function type's return type would take the union).
//! A bare union is what the rule makes of each member, as it is written
//! (each record member's fields and list member's elements as the rule
//! says, a function type member following its own marker), the union of
//! them at its own position, and the either shape of that union where a
//! bare name would be: `Int | String` in an impure function's parameter is
//! `Int | String | Future<Int | String>`, so widening a parameter from
//! `Int` to `Int | String` does not stop it taking a future
//! (`Checker::resolve_union`). An applied type other than `List` and a
//! union with a member written `Future<...>` are explicit, and so never
//! wrapped. `Future<T>` and `Present<T>` override that at their position,
//! their own interiors being present, and a union with a `Future` member
//! may be written if it is the either shape, `T | Future<T>`: exactly one
//! member a `Future`, of the union of the others, as in `Int | String |
//! Future<Int | String>`; any other union with a `Future` member ("a union
//! with a `Future` member must be `T | Future<T>`", at its last `|`), any
//! other name applied to a type, and `Future` or `Present` without a type
//! argument are errors. So an impure function takes, and says it returns,
//! values that may be futures, since its body awaits them on demand, while
//! a pure function takes present values where its body needs them, since
//! its body cannot
//! await, and a memoised function takes present values, the key it looks
//! its results up by; and a pure `fn(Int): Int` is not a subtype of the
//! impure `fn(Int) -> Int`, which promises to pass futures (parameters are
//! contravariant), though it is of `fn(Present<Int>) -> Int`, and neither
//! is a memoised `fn(Int) ->> Int`, though it too is a subtype of
//! `fn(Present<Int>) -> Int` (see [`crate::types`] for the effect
//! lattice). An impure or memoised function's parameter written `fn(Int)
//! -> Int` is narrowed to `fn(Present<Int>) -> Int` if its body only ever
//! passes it present values: see [Narrowed
//! callbacks](#narrowed-callbacks).
//!
//! **Awaits.** Every place a value is used as its bits decides whether it
//! is awaited there, and records it in [`Resolution::await_of`] (or, for a
//! record an update reads, [`Resolution::path_awaits`]) for lowering, which
//! inserts the await; nothing is decided at runtime. A measuring operator
//! (arithmetic, ordering, `and`, `or`, `not`, negation), a call of a value,
//! a field access into one and a spread of one await it at the top if its
//! type is a `Future` or an either (`Checker::measure`), and type the
//! operation on the type it has once awaited, stripped of every future
//! layer at its top (see [`TypeTable::strip`]); so does a dotted `let` or `:=` each record,
//! or list, along its path that exists (see "Dotted `let` and `:=`" above). An
//! interpolated expression and an operand of an equality are awaited deeply
//! (`Checker::measure_deep`): at the top and in every lazy field of the
//! record it is, or element of the list it is, at any depth (see
//! [`TypeTable::present`]). Where a type is
//! expected, as for an argument, an annotated initialiser, a function's body or an `if` condition,
//! a value
//! that is not a subtype of it is awaited if it is one once awaited, at the
//! top or deeply (see [`TypeTable::needs_await`]); otherwise the mismatch is
//! reported, with a note when a future is expected and a value that is not
//! one is found. `await e` awaits `e` fully and has its type stripped; on a
//! present value it does nothing, silently, since whether a value is
//! present may be inferred from code elsewhere. A bare `await e` item
//! is never an unused value. A statement, an expression item that is not
//! its block's result, is a position expecting a *unit-like* value: `{}`,
//! a list whose element type is unit-like, or a union all of whose members
//! are, as the `List<{}> | {}` of a statement loop a bare `return` may
//! leave is (see [`TypeTable::is_unit_like`]), which holds nothing a
//! program could use.
//! A statement that is unit-like as it is, as `{}` is, or the `List<{}>` of
//! a `for` whose body is `{}`, is fine as it is; one whose value is
//! unit-like once awaited, at the top (a `Future<{}>`, or the `{}` an
//! impure function returns, which may be one) or deeply (a
//! `List<Future<{}>>`, as `for s in steps { lazy run(s) }` is), is awaited
//! there, at the top or deeply as its type needs (`Checker::measure`,
//! `Checker::measure_deep`), subject to the purity rule, so that its
//! effects are not lost. A call of a function or built-in that is not pure,
//! outside a pure function, is run for its effects as a bare `await` is:
//! awaited deeply, and its value, whatever it is, discarded
//! ([`Resolution::is_discarded`]). Any other statement is an unused value. A
//! statement that is directly the empty list literal, `[]`, is a
//! `List<{}>`, whose element type needs no inferring.
//!
//! Futures have no equality: an operand of `==`, `!=`, `===` or `!==` whose
//! type, once awaited at the top, has a `Future` in a field or as a list's
//! element type at any depth is an error (a field or an element of the
//! either shape is not: the deep await resolves it). Two `if` branches of which either may be a
//! future have their join
//! (see [`TypeTable::join`]).
//!
//! **Purity.** Awaiting runs a deferred expression, which may have effects,
//! so it is impure: an await in the body of a pure function, explicit or
//! implicit, is an error, reported where the await is decided, entirely
//! statically. A pure function may still build, hold, store, pass on and
//! return futures, and build them with `lazy`, whose body is impure. A
//! pure function measuring its own bare parameter is never an error: it
//! makes the parameter present (see below), so the await happens in the
//! caller, where an argument that may be a future is awaited at the call.
//!
//! The implicit-await error's note suggests declaring the function with
//! `->`. Where the value is a variable, through any parentheses and as the
//! result of any block (as in the body `{ n }`), or the variable a dotted
//! `let` or `:=` updates, the record at its path's first segment, that the
//! pure function captures, and that code outside it could await before
//! creating it, the note instead says to do that, as `let n = await n`
//! (`Checker::awaitable_capture`, `Checker::awaitable_outside`): a variable
//! of the top level, which is impure, or one whose home function (the
//! function whose body declares it) or a function between that one and the
//! pure function is impure, as a `lazy` and a memoised function are. A
//! capture whose home and every function between are pure could be awaited
//! there no more than in the pure function, and keeps the usual note, as
//! does a record further along a dotted `let`'s or `:=`'s path, which is
//! not a variable.
//!
//! # Inferred parameters
//!
//! A pure function's bare parameter, one whose type is written as a name, a
//! record type, a list type or a union with no `Future` member (not
//! `Present<...>`, `Future<...>`, a union with a `Future` member or a
//! function type, which say what they are), is present if the function's
//! body
//! *measures* it, and the either shape if it never does, so that a pure
//! function can pass a future through without declaring it: `fn double(x:
//! Int): Int { x * 2 }` is `fn(x: Int): Int`, and `fn keep(x: Int): {v: Int
//! | Future<Int>} { v = x }` is `fn(x: Int | Future<Int>): {v: Int |
//! Future<Int>}`. The result is not inferred: a pure function's bare result
//! is present, so `fn id(x: Int): Int { x }` measures `x` through it. (An
//! impure function's is inferred, and its parameters are not: see [Inferred
//! results](#inferred-results).)
//!
//! While the body is checked, such a parameter has a provisional type,
//! `provisional(function, index, T)` (see [`TypeTable::provisional`]), in
//! the function's type as first interned, in the callee binding and in the
//! parameter's binding, and the function's entry on the stack
//! (`Function::demanded`) records, for each parameter, the first place in
//! source order where it is measured. A value keeps the provisional type
//! through *blind assignment*, with no code of its own: a `let` without an
//! annotation binds it, a record field holds it and gives it back when read,
//! and an `if` both of whose branches have it has it. The parameter is
//! measured where a value of its provisional type is used as its bits,
//! which is exactly where a value that may be a future would be awaited:
//!
//! - `Checker::measure` (the operands of the measuring operators, the
//!   callee of a call, the record of a field access, the list of an index,
//!   a spread or a `for`, a statement that is `{}` once awaited) measures
//!   a provisional type at the top, and gives its present type, with no
//!   await and no purity error;
//! - `Checker::measure_deep` (an interpolated expression, an operand of an
//!   equality, a statement that is unit-like once awaited deeply) measures
//!   each provisional type in the value's type, at the top or in a field,
//!   before deciding the await for the rest;
//! - `Checker::expect`, where the value is not a subtype of what is
//!   expected as it is, measures each parameter whose provisional type sits
//!   where the expected type is present (a present parameter, annotation,
//!   ascription, `:=` target, field type or return type, or the `Bool` of a
//!   condition or a logic operand) and tries again with them present; a
//!   provisional type is a subtype of the either shape as it is, so passing
//!   it to an either parameter, storing it in an either field and returning
//!   it against an either return type measure nothing;
//! - an explicit `await` of it (an error in any case, the function being
//!   pure) and an update of the record it is, whether it is the updated
//!   variable or a record further along the path (`let p.x = ...`, or
//!   `let q.p.x = ...` where `q` holds it in its field `p`), measure it;
//! - a recursive call through the function's own name measures every
//!   argument whose type is a provisional type of the function,
//!   conservatively;
//! - an `if` whose branches have different types measures every parameter
//!   whose provisional type is in either branch, conservatively, and the
//!   branches then agree as they would with those parameters present: a
//!   parameter and a present value join to the present type, and a
//!   parameter and a future to the either shape. (Joining a parameter with
//!   a present value as the either shape instead would leave the `if` an
//!   either, which the pure function could then not measure, and would
//!   reject `if n < 2 { n } else { ... }` against an `Int` result.)
//!
//! Capturing a parameter in a closure or a `lazy`, and binding it with a
//! `let` without an annotation, measure nothing.
//!
//! When the body's check ends, each parameter is decided: present if it
//! was measured. [`TypeTable::settle`] then replaces the function's
//! provisional types in the type of every expression of the function, the
//! ids of its subtree (which the parser allocates contiguously: see
//! [`Ast`]), and in what every function and `lazy` nested in it captures,
//! and the function's own type is its settled type; which parameters were
//! measured, and where, is recorded in [`FnResolution::param_demands`]. A
//! provisional type reaches nothing outside the function but through the
//! function's type, which is settled with the rest, so this one pass is
//! enough, and a successful check leaves none (the final invariant check
//! says so). A nested function is settled when it ends, for its own
//! parameters, and again when the function enclosing it ends, for that
//! one's, which is why settling names the function.
//!
//! **Nested functions.** A provisional type of an *enclosing* function,
//! seen in a nested body through a capture, is measured for the function it
//! belongs to when every function between, the nested one included, is
//! pure (`Checker::claimable`): a pure closure can only ever see what the
//! function around it was given, so measuring the parameter there makes it
//! present, and the caller awaits it before the enclosing function runs, as
//! for a measurement in the enclosing body itself; `fn make(n: Int): fn():
//! Int { fn(): Int { n } }` therefore takes a present `n`. Through an impure
//! or memoised function or a `lazy`, which may await, the provisional type
//! is instead treated as the either shape it may become: measuring it there
//! is an await, which measures nothing for the function it belongs to, and
//! inside a pure function nested in that impure one it is the purity
//! error. This is conservative and correct: the enclosing function's
//! decision is not known while the nested body is checked, and if nothing
//! measures the parameter a future may indeed arrive, which the `lazy` or
//! impure function awaits when it runs. (An await of a parameter that turns
//! out present finds a present value, and does nothing.)
//!
//! **Where a provisional type is shown.** A diagnostic shows a provisional
//! type as what it stands for at that point (`Checker::shown`): one of a
//! function the body being checked may measure (`Checker::claimable`) as
//! present if its parameter has been measured so far and the either shape
//! otherwise, and any other enclosing function's as the either shape.
//! [`TypeTable::display`] shows one as `Int?`, which nothing the checker
//! returns holds.
//!
//! **The call-site error.** An argument that is awaited at the top, in a
//! pure caller, because the parameter it fills is present by inference, is
//! the error "argument `x` would be awaited inside `f`, which is pure",
//! labelled with where `f`'s body first measures the parameter when `f` is
//! known (`Checker::argument_await_error`). A callee is known when it is a
//! plain name whose binding, not a mutable variable's, was initialised
//! directly with a function expression, of this input
//! ([`FnResolution::param_demands`] says which
//! parameters were measured, and where) or of an earlier session input,
//! whose [`Scope`] keeps which parameters its functions' bodies measure
//! ([`KnownParams`]), but
//! not where, since that input's source is gone: the error then has no label
//! on the measuring use. A parameter written present, or of a callee that is
//! not known, gives the generic implicit-await error.
//!
//! # Inferred results
//!
//! An impure or memoised function's bare result position, one written as a
//! name, a record type, a list type or a union with no `Future` member, is
//! the either shape while the body is checked,
//! as the bare type rule says, and then takes the laziness of what the body
//! returns there: `T` if it is present, `Future<T>` if it is a future, and
//! the either shape, as written, if it may be either or has a provisional
//! type (see below), `T` being the type written there, with its own bare
//! positions inferred in the same way if it is a record type, each field
//! against the field of the same name of what the body returns there once
//! stripped (the body's other fields play no part), and if it is a list
//! type, its element type against the element type of what the body
//! returns there once stripped. So `fn get() -> {a: Int,
//! b: String} { a = 1; b = "x" }` is `fn() -> {a: Int, b: String}`, `fn
//! later() -> Int { lazy 5 }` is `fn() -> Future<Int>`, and `fn pass(x: Int)
//! -> Int { x }` is `fn(x: Int | Future<Int>) -> Int | Future<Int>`, its
//! parameter being the either shape, while `fn add(x: Int) -> Int { x + 1 }`
//! is `fn(x: Int | Future<Int>) -> Int`, and `fn l() -> List<Int> { [1,
//! lazy 2] }` is `fn() -> List<Int | Future<Int>>` (see
//! [Lists](#lists)). A bare union is inferred at its top only, as a name
//! is: `fn f(c: Bool) -> Int | String { if c { 1 } else { "a" } }` is
//! `fn(c: Bool | Future<Bool>) -> Int | String`, the inside of each member
//! staying as declared. A position that is not bare keeps
//! what is written, with everything inside it: `Present<...>`,
//! `Future<...>`, a union with a `Future` member and a function type say
//! what they are. A
//! parenthesised type is what is inside it. The parameters are never
//! inferred, nor is anything of a pure function's, though a callback
//! parameter may be narrowed, in the same step (see [Narrowed
//! callbacks](#narrowed-callbacks)). A memoised function's
//! result is inferred in the same way, and its parameters are present:
//! `fn fetch(x: Int) ->> Int { x }` is `fn(x: Int) ->> Int`, and `fn
//! later(x: Int) ->> Int { lazy x }` is `fn(x: Int) ->> Future<Int>`.
//!
//! What the body returns is the value it has once every await the checker
//! marked for fitting it to the declared result has run, since the typed
//! walkers of the evaluator trust the inferred type (`Checker::value_of`):
//! a body, or a field's value, awaited at the top has its type stripped, and
//! one awaited deeply its type made present (see [`TypeTable::present`]),
//! which is exactly what the deep await's lazy shape resolves; a record block
//! checked field by field against the declared record type, whose type is
//! the record of the declared fields' types, is instead the record of what
//! its fields' values are, each found in the same way; an untyped literal
//! has been coerced. An open field, one built by dotted paths, is a present
//! record, built by the block, of what its segments are: a segment that
//! further paths extend is such a record in turn, and a leaf, `a.b.c = e`,
//! is what `e` is, found as a field's value is, since it was checked
//! against the declared type at its path as a field's value is against its
//! field's type, and lowering stores exactly `e`, with any await marked on
//! it, into the record (see [Dotted fields](#dotted-fields)). So `fn n() ->
//! {a: {b: Int}} { a.b = 1 }` is `fn() -> {a: {b: Int}}`, as with `a = { b
//! = 1 }`. A soft field, one a spread defines (see [Spreads and soft
//! assignment](#spreads-and-soft-assignment)), is what the type its
//! spread's operand gives it says, as it is: the spread stores exactly the
//! value it reads from the operand, once awaited if marked, at that field,
//! never awaiting it, and the static type is the truth, so the record
//! really holds a value of that type there. Only the definition of a field
//! that stands is inferred, a soft one overridden later playing no part.
//! The block's items are passed over once, rebuilding the segments
//! of its open fields as a tree (`Built`), which is then inferred against
//! the declared type level by level; a segment the declared type lacks
//! plays no part, and a declared field the block does not define, which a
//! successful check rules out, keeps its declared type. A position whose
//! value's type is not a subtype of what would be inferred keeps its
//! declared type. The inferred result
//! is a subtype of the declared one, so the function's type is one too, by
//! ordinary result covariance; a written function type is not inferred, so
//! `fn() -> Int` written means the either shape as before, and accepts
//! `fn() -> Int { 5 }`, as `fn() -> Present<Int>` now does too.
//!
//! What the body returns is decided from all its *result sites* (see
//! [`return`](#return)): its own result and the value of every `return`
//! for the function, as if they were the branches of one `if`
//! (`Checker::inferred_result`). Each site is inferred as the body's
//! result alone would be, and the result is the join of what they give,
//! or the declared type should that join not be a subtype of it; a site
//! of type `Never`, as a body ending in a `return` is, plays no part, and
//! a function every site of which is `Never` is inferred from `Never`, a
//! present type. Inferring each site and joining is never less precise
//! than joining the sites' types first, which would see a record block
//! checked field by field as the record of the declared fields, and is the
//! same where the sites are names, futures or eithers: `fn f(c: Bool) ->
//! Int { if c { return 1 }; 2 }` is `fn(...) -> Int`, with `lazy 2` there
//! `-> Int | Future<Int>`, and with both lazy `-> Future<Int>`.
//!
//! The result is inferred once, when the body's check ends
//! (`Checker::inferred`), and only if no error was reported while the body
//! was checked, and the declared parameter and result types have none: an
//! erroneous function keeps its declared type, so that its calls do not
//! cascade. While the body is checked, the function's own name has the
//! declared type, so a recursive call's result is the either shape at the
//! bare positions, and nothing is iterated: `fn count(n: Int) -> Int { if n
//! == 0 { 0 } else { count(n - 1) } }` is `fn(n: Int | Future<Int>) -> Int |
//! Future<Int>`, but with `sum(n - 1) + n` in the `else` branch, which
//! measures the recursive call, the result is present. Everything that sees
//! the function's value sees the inferred type (the function expression's
//! type, the binding it initialises, a session's [`Scope`], a closure's
//! display, and captures); a use of its own name in its body keeps the
//! declared type, a supertype. A provisional type of an enclosing pure
//! function in what the body returns counts as the either shape, so the
//! inferred type holds no provisional type, and the enclosing function's
//! settling leaves it as it is.
//!
//! # Narrowed callbacks
//!
//! In an impure or memoised function expression, a *callback parameter* is
//! one whose type is written directly as an impure function type, `fn(...)
//! -> T` (not inside `Future<...>`, a union, a record type or another
//! function type), and each parameter position of that type written bare,
//! as a name, a record type, a list type or a union with no `Future`
//! member, is *narrowable*. As written,
//! such a position
//! is the either shape: the function says it may pass the callback a
//! future there, so a pure callback that measures its parameter does not
//! fit. A body that only ever passes present values there is narrowed to
//! say so, and then it does:
//!
//! ```text
//! fn double(x: Int): Int { x * 2 }
//! fn run(f: fn(Int) -> Int) -> Int { f(1) }
//! # run is fn(f: fn(Int) -> Int | Future<Int>) -> Int | Future<Int>
//! run(double)      # 2
//! ```
//!
//! Once the body is checked, each narrowable position becomes its *present
//! type*, the type written there with every bare type in it present (`Int`
//! for `Int`, `{a: Int}` for `{a: Int}`, and `{a: Future<Int>}` for `{a:
//! Future<Int>}`), if the body never uses the callback as a value, and
//! every call of it gives the position an argument that fits the present
//! type; otherwise it stays as written. A use as a value is any appearance
//! of the name other than as exactly the callee of a call: `let g = f`,
//! `other(f)`, `(f)(1)`, returning it, interpolating it. Each position is
//! decided on its own, all or nothing, and a callback that is never called
//! nor used has every narrowable position narrowed. An argument fits if
//! what it is where it is passed, found as an inferred result's value is
//! and inferred against the position as a bare result position is (see
//! [Inferred results](#inferred-results): once the awaits marked on it have
//! run, a record block by what its fields are, an untyped literal
//! coerced), is a subtype of the present type. So `f({ a = 1 })` fits `{a:
//! Int}`, while `f(j)`, `j` being a parameter of the function, which may be
//! a future, does not fit `Int`, and nor does a parameter of an enclosing
//! pure function, whose type is provisional. Positions not written bare,
//! `Present<T>`, `Future<T>`, `T | Future<T>` and function types, are never
//! narrowed, nor is the callback's result; a parameter written as a
//! memoised or pure function type is not a callback parameter, its bare
//! parameters being present already; and nothing of a pure function's is
//! narrowed, since it cannot call an impure callback.
//!
//! Calls count wherever they are in the body, in a nested function or
//! `lazy` too, as long as the name resolves to the parameter, not to
//! something shadowing it. While the body is checked, the parameter's
//! binding holds the index of its entry in the stack of the callback
//! parameters of the functions being checked (`Callback`, with one
//! `CallbackPosition` for each of its positions), which records the first
//! use as a value (`Checker::check_name`) and, for each position, the first
//! argument that does not fit (`Checker::check_callee`,
//! `Checker::record_argument`), each in constant time but for finding what
//! the argument is and the subtype test. When the body ends, the positions
//! are decided (`Checker::narrow_callbacks`), and the function's type is
//! interned again with the narrowed parameter types, in the same step as
//! its inferred result and on the same conditions: no error was reported
//! while the body was checked, and the declared parameter and result types
//! have none. Everything that sees the function's value sees that type.
//! Nothing inside the body changes: the callback has its written type
//! there, every expression keeps its type, and no await is added or
//! removed; the function's own name in its body has the declared type.
//!
//! Narrowing a position makes the callback's type a supertype of the
//! written one, since parameters are contravariant, so the function accepts
//! more callbacks. It is sound because the only code that can call the
//! callback is the body, which was checked against the written type and is
//! known to pass nothing at a narrowed position that does not fit its
//! present type: the value can leave the body only through a use of its
//! name as a value, which keeps every position as written.
//!
//! **The call-site label.** When an argument does not fit a callback
//! parameter of a known callee of this input (see "The call-site error"
//! under [Inferred parameters](#inferred-parameters)) that was not narrowed
//! at some position, the mismatch has a label at what kept a position the
//! argument needs as written, the first in source order: "`f` is given a
//! value that may be a future here" at the argument of a call, and "`f` is
//! used as a value here, so it may be given a future" at a use as a value.
//! A position is needed if the argument's function type does not take the
//! type the position is written as. The label is added only if the
//! argument would have fitted, as it is or once awaited, had every
//! narrowable position been narrowed, so that it never misleads about a
//! mismatch with another cause ([`FnResolution::unnarrowed`] records what
//! it needs). A callee of an earlier session input has no label, its
//! source being gone.
//!
//! # Lists
//!
//! `List<T>`, also written `T[]`, is a container, like a record type (see
//! [`crate::types`]): every rule above that names a record type names a
//! list type too, with its element type a position inside it. So in an
//! impure function's signature `List<Int>` is `List<Int | Future<Int>> |
//! Future<List<Int | Future<Int>>>`; a pure function's parameter written as
//! a list type is inferred, a bare list result position is inferred from
//! the body, its element type from the element type of what the body
//! returns, and a callback's parameter position written as a list type is
//! narrowable; and a memoised function's list parameter is present
//! throughout, so an argument whose elements may be futures is awaited
//! deeply at the call. What looks through a record's fields for futures
//! and provisional types looks through a list's element type, so an
//! interpolated list and an operand of an equality are awaited deeply,
//! element by element.
//!
//! **Literals.** A list literal with no expected type is the list of the
//! join of its elements' types (see [`TypeTable::join`]), in order, a
//! spread contributing its list's element type; untyped integers adapt as
//! an `if`'s branches do, so `[1, 2.5]` is `List<Float>` and `[1, 2]`
//! `List<Int>`, and two element types that differ measure the provisional
//! parameters in them, as an `if`'s branches do (see [Inferred
//! parameters](#inferred-parameters)). Elements that do not agree make a
//! union, which is the warning "the elements of this list have different
//! types", at the first element that widens the join of those before it
//! (see [Unions](#unions)); `[]` has nothing to join, and is "cannot infer
//! the element type of `[]`". Nothing is awaited: `[1, lazy 2]` is
//! `List<Int | Future<Int>>`.
//!
//! A literal where a list type `List<X>` (or its either shape) is expected
//! has each element checked against `X` as an argument is against its
//! parameter: an untyped integer adapts, an element that fits `X` once
//! awaited is awaited, subject to the purity rule, and anything else is a
//! mismatch. The literal's type is then the list of the join of what its
//! elements are once those awaits have run (`Checker::fit`: what the
//! evaluator's typed walkers will find in it), a subtype of `List<X>`, so
//! that an inferred result sees what the list really holds; `[]` there is
//! the list of `X` made present, where that fits `X`, since no element of it
//! is a future. The expected type reaches a literal where it reaches a
//! record block (an annotation, an argument, a function's body, a field of
//! an expected record type, a `:=`, an ascription) through parentheses, and,
//! for a list type or a union only, also through a block's result and the
//! branches of an `if` with `else`, so that a function's body, a block,
//! reaches its literal; it reaches a `for` in the same way (see
//! [For](#for)); such a block or `if` has the type of what its result, or
//! the join of what its branches, are once awaited, or the expected type
//! if that join is not a subtype of it. Under an expected type, a join is
//! never a warning, since each part fits what is expected. Where a union
//! is expected, a list literal or a `for` is checked against its one list
//! member, and `[]` against any (`Checker::expected_member`). It never
//! flows from one branch of an `if` to the other. A record type expected
//! reaches neither a block's result nor an `if`'s branches, but for a
//! record type with defaults, which reaches a record literal there for its
//! defaults alone (see "Field defaults" under [Functions](#functions)).
//!
//! A spread's operand is used as its bits at the top (`Checker::measure`),
//! and must be a list: "cannot spread a value of type T: it is not a list".
//! It is checked with nothing expected, but for a `for` where the elements
//! are expected to be of a record type with defaults, whose body has a
//! record literal it results in given them, as a `for`'s body has where a
//! list of the type is expected of it (`Checker::check_spread`; see "Field
//! defaults"), the rest of the loop checked as ever; and there for a block
//! with a result or an `if` with `else`, whose list literal results, and
//! `for`s, have the record literals among their elements, and their
//! spreads, given them in the same way, the rest of the block, the `if` and
//! the list checked as ever (`Checker::fill_elements`).
//! Its elements are not measured: where an element type is expected, they
//! must fit it as they are, since there is no expression to await for each;
//! a list whose elements would fit only once awaited is a mismatch, with a
//! note saying why.
//!
//! **Indexes.** `e[i]` measures `e` at the top, which must be a list
//! ("cannot index a value of type T: it is not a list"), and `i`, which
//! must be an `Int` ("a list index must be an Int, found T"). Its type is
//! the element type as it is, so the element, not the index, is awaited
//! where it is used as its bits. A position outside the list is a runtime
//! error (see [`crate::eval`]).
//!
//! **Updating an element.** The path of a `:=` may have index segments,
//! `[EXPR]`, among its names, in any number and order (see
//! [`crate::ast::SegmentKind::Index`]):
//!
//! ```text
//! let mut xs = [1, 2, 3]
//! xs[0] := 10                         # [10, 2, 3]
//! let mut conf = { hosts = [{ name = "a"; up = false }] }
//! conf.hosts[0].up := true
//! grid[y][x] := 1
//! ```
//!
//! Which variable a `:=` may rebind is as for any `:=` (see [Mutable
//! variables](#mutable-variables)). The walk along the path (see "Dotted
//! `let` and `:=`" above, and `Checker::walk_path`) has one case more: at
//! an index segment `P[EXPR]`, the value at `P` is measured at the top, as
//! a record along a path is (awaited if it may be a future, subject to the
//! purity rule, reported at `P`; a pure function's bare parameter
//! measured), and must then be a list: otherwise "cannot update `xs[0]`:
//! `xs` is Int, not a list", showing the type it had before stripping, the
//! purity error being reported only for a value that is a list once
//! awaited. `EXPR` is then checked and measured, as an index's position is
//! (`Checker::check_position`), an expression of the enclosing function
//! like any other, its purity, captures and awaits as usual: an `Int`, an
//! untyped integer being one, or "a list index must be an Int, found T".
//! The segment's type is the list's element type as it is; a segment after
//! it walks into the element, which is measured in turn, as a record for a
//! name or a list for another index. A name segment on a list, or an index
//! segment on a record, is the matching "not a record" or "not a list"
//! error. Each position is checked exactly once: where the walk reaches it,
//! or on its own if the walk stops before, or the `:=` is rejected, so that
//! its mistakes are still reported.
//!
//! If the last segment is an index, the value must fit the list's element
//! type, as a field's leaf must fit the field's: a subtype, an adapting
//! literal, or an await to fit where one is allowed, labelled "`:=` keeps
//! the element's type" at the path. A `let` path cannot index a list, so a
//! field missing after an index segment, "cannot rebind `xs[0].b`: `xs[0]`
//! has no field `b`", has no note suggesting one.
//!
//! A list's type never changes. The updated type is built from the leaf
//! outwards (`Checker::record_update`), each record along the path present
//! with its segment set, and each list along it present, keeping its
//! element type, which the updated element must be a subtype of
//! (`Checker::updated_list`). It is whenever the element type is present or
//! the either shape; otherwise, as for elements declared exactly
//! `Future<{a: Int}>` of a path continuing through one, it is "cannot
//! rebind `xs[0].a`: the elements of `xs` are `Future<{a: Int}>`, and the
//! update makes one `{a: Int}`", at the path, with a note that a list keeps
//! its element type. A mutable variable keeps its declared type, which the
//! updated type must then fit (`Checker::kept_type`); a variable that is
//! not mutable is rebound with the updated type. `:=` never adds: an index
//! segment does not create an element, and its position is not checked
//! statically. A rejected `:=` with an index segment declares its name as
//! `Error`, rather than walking its path to find an updated type, which
//! would check its positions again.
//!
//! **Equality.** Lists compare as records do (see [Equality](#equality)),
//! when their types are identical, contain no function and no `Future`:
//! element by element, typed equality comparing records inside by their
//! static type. They have no order, and no arithmetic: `+` does not
//! concatenate them, a spread does.
//!
//! # Quantities
//!
//! `Duration` and `Size` are *quantities*: a duration of whole
//! nanoseconds and a size of whole bytes, each an `i64` of its smallest
//! unit ([`TyId::DURATION`], [`TyId::SIZE`]). A duration literal, `30s` or
//! `1h30m`, is a `Duration`, and a size literal, `2GiB`, a `Size` (see the
//! lexer's literal grammar, `Lexer::quantity`); each is a constant, negated
//! or not, so it may be a parameter's or a field's default. Each is a type
//! of its own: it is a subtype only of itself, mixes with no other type
//! (`Int`, `Float` and the other quantity included), and nothing converts
//! to it, so `0` is no duration and an integer literal never adapts to
//! one. Their names are built-in type names, so a `type` may not be named
//! `Duration` or `Size`.
//!
//! Its operators (`Checker::arithmetic`, `Checker::integer_only`), `Q`
//! being either quantity:
//!
//! | expression                | type   |
//! |---------------------------|--------|
//! | `Q + Q`, `Q - Q`, `-Q`    | `Q`    |
//! | `Q * Int`, `Int * Q`      | `Q`    |
//! | `Q // Int`                | `Q`    |
//! | `Q // Q`                  | `Int`  |
//! | `Q % Q`                   | `Q`    |
//! | `Q < Q` and the others    | `Bool` |
//! | `Q == Q`, `Q != Q`        | `Bool` |
//!
//! An untyped integer beside a quantity is an `Int`. Any other mix is the
//! operator's usual error, naming both operands, as in "cannot add
//! Duration and Int"; `/` is "cannot divide Duration and Int", noted "use
//! `//`; a Duration is a whole number of nanoseconds", and a `//` or `%`
//! that does not fit is noted with what a quantity divides by. Comparing
//! two of one type is as for `Int`, and comparing a quantity with anything
//! else is the mismatch. A quantity is shown as its canonical literal
//! wherever a value is (see [`crate::value::write_quantity`]): in an
//! interpolation, by `show` and in a type's default. Lowering computes on
//! the amounts with the `Int` nodes, giving a quantity's result its unit
//! (see [`crate::ir`], "Quantities").
//!
//! # Ranges
//!
//! `a..b` is the list of the integers from `a` up to but not including
//! `b`, and `a..=b` up to and including `b`. Both bounds are measured, as
//! the operands of any measuring operator are (`Checker::measure`), and
//! must be `Int`, an untyped integer being one, which is coerced: "a
//! range's bounds must be Int, found T", at the bound. The range is a
//! present `List<Int>`, and pure. That a range used as a value may not be
//! too long is a runtime matter (see [`crate::eval`]).
//!
//! # For
//!
//! `for x in xs if GUARD { BODY }` is the list of the body's values for the
//! elements of `xs`, in order, that pass the guard (`Checker::check_for`).
//! The iterable is checked and measured at the top, as the list of an
//! index is: awaited if it may be a future, subject to the purity rule, and
//! a pure function's bare parameter measured. It must then be a list:
//! "cannot iterate over a value of type T: it is not a list", at the
//! iterable. Its elements are not measured.
//!
//! The variable is declared in a scope of its own, which holds the guard and
//! the body and nothing else, as a block's scope holds its items: it takes
//! the next free slot of the enclosing function's frame (see
//! [`Resolution::for_slot`]), which the `for` leaves when it ends, and has
//! the element type as it is, which may be the either shape or a future. It
//! shadows as a `let` does, and since the body is a block of its own, `:=`
//! of it there is "declared in an enclosing block". A `for` is not a
//! function: the guard and the body are checked in the enclosing function,
//! whose purity governs their awaits and calls and whose captures they use,
//! so a closure or a `lazy` made in the body captures the variable as it
//! would any variable of the enclosing function, by value, which is its
//! value in that iteration. The guard is checked as an `if`'s condition is:
//! measured, and `Bool`. Being blocks of the enclosing function, the guard
//! and the body may rebind its mutable variables (see [Mutable
//! variables](#mutable-variables)), and the guard for each element reads
//! what the body wrote for the elements before it.
//!
//! Every loop, of any form (see [Loops](#loops)), is the list of what its
//! iterations contribute, `List<E>`, `E` being its *element type*
//! (`Checker::element_type`): the join of what can contribute, the body's
//! type and the type of the value of every `break E` and `continue E` for
//! it (see [`break` and `continue`](#break-and-continue)), treated exactly
//! as the elements of a list literal are (see [Lists](#lists) and
//! [Unions](#unions)), in the order they are checked, the body last. Where
//! no list type is expected, they are joined, untyped integers adapting to
//! each other and settling to `Int` as a list literal's elements do, and a
//! widened join is a warning, "the values this loop collects have
//! different types", at the loop's keyword, labelled at each contributing
//! site with its type (the body at its value, through blocks, as an `if`'s
//! branch is shown), with a note naming the loop's type as a list's does.
//! Where a list type `List<X>`, or its either shape, is expected, the
//! expectation reaches the loop as it reaches a list literal, and the body
//! and each value are checked against `X` as a literal's element is
//! (`Checker::fit`): an untyped integer adapts, a value that fits once
//! awaited is awaited, and a record block is checked field by field; where
//! `X` is a record type with defaults, a record literal that a body or a
//! value that is a block results in, or an `if`'s branches do, is given
//! them (see "Field defaults" under [Functions](#functions)), as is one a
//! spread `for`'s body results in where the list's elements are expected
//! to be of such a type, or the body of a `for` that a spread block or `if`
//! results in there (see [Lists](#lists)). `E` is
//! then the join of what they are once those awaits have run, the type of
//! every value the evaluator collects, or `X` if that is not a subtype of
//! `X`, as for a literal, and is no warning. A bare `break` or `continue`
//! contributes nothing, and a contribution of type `Never` joins as
//! nothing (see [Never](#never)), so `for x in xs { break x }` is
//! `List<Int>`. If nothing can contribute, as in `for { break }`, `E` is
//! the expected element type, or `{}` without one, so that such a loop is
//! unit-like as a statement. A body that can fall off its end contributes
//! its own value: `for x in xs { if x > 5 { break x } }` is `List<Int |
//! {}>`, with the warning, which a final `continue` removes. A body that
//! is a record block makes a list of records. That list is the loop's
//! *ordinary* type, what it is when it ends: a loop that cannot end is
//! `Never` instead (see [Never-ending loops](#never-ending-loops)), and a
//! loop a `return` leaves is the join of its ordinary type and the values
//! returned (see [Loops that return](#loops-that-return)).
//!
//! # Loops
//!
//! Besides the iteration, a loop is a condition loop, `for COND { ... }`,
//! with a header, `for BEFORE; COND; AFTER { ... }`, or without a condition,
//! `for { ... }`, the loop whose condition is `true`; or a `do` loop, `do {
//! ... } for COND`, whose body runs before its condition. Each is an
//! expression of the type above, with a scope of its own, as an
//! iteration's: the header's items are declared in it, so they are visible
//! in the later parts of the header and in the body, an inner block, and not
//! after the loop. A condition is checked as an `if`'s is, measured and
//! `Bool` ("`for` conditions are Bool"), once, and narrows nothing; a
//! `do`'s is checked after the body, outside the body's block, whose
//! variables it cannot see. The enclosing function's purity governs the
//! awaits and calls of every part, and a loop is allowed in a pure
//! function.
//!
//! A header's items are checked as a block's (`Checker::header_item`):
//! before the condition, `let`s, of any form a block allows, and `:=`s;
//! after it, `:=`s only. A plain `let` there declares a variable checked as
//! a `let mut`'s is, one slot for its life and its type fixed by its
//! initialiser or annotation, a dotted or indexed `:=` of it having to keep
//! that type, but of kind `BindingKind::Header`, which only a `:=` of the
//! header itself, its own block, may rebind, in its one slot
//! (`Checker::header_rebind`); one from the body, an inner block, is
//! "cannot rebind `i` here: it is declared in the header of a `for`",
//! labelled at the declaration, with a note to write `let mut` in the
//! header, and one from a function nested in the loop is the error of any
//! variable declared in an enclosing block. A `let mut` in the header is a
//! mutable variable, which the body may rebind too (see [Mutable
//! variables](#mutable-variables)). A `:=` in the header of a variable from
//! outside the loop follows the rules of any `:=`. Each iteration sees the
//! variable's slot as the after-items left it, and a closure made in the
//! body copies it, so it captures that iteration's value.
//!
//! # `break` and `continue`
//!
//! `break`, `continue`, each with an optional value and label, are items of
//! a block, never expressions (see [`crate::parser`]). A block whose last
//! item is one is of type `Never`. Each is for a *target loop*
//! (`Checker::jump_target`, recorded for lowering in
//! [`Resolution::jump_target`]): without a label, the innermost loop of the
//! function being checked whose *body* holds it, a loop's iterable, guard,
//! condition and header items being outside its body; with one, the loop of
//! that label whose body holds it. To find it, the checker keeps the loops
//! whose bodies are being checked on a stack (`LoopFrame`), with the
//! labelled blocks whose items are (see [Labels](#labels)), each with the
//! number of functions being checked when its body began, so that a loop
//! outside the innermost function or `lazy` is not a target. A `break` for
//! a loop is recorded on its frame, for [Never-ending
//! loops](#never-ending-loops). Errors, each at the keyword and its label:
//!
//! - no loop holds it: "`break` outside a loop" (or "`continue`");
//! - only loops outside the innermost function or `lazy` hold it, or have
//!   its label: "`break` cannot leave a function" (or "a `lazy`"),
//!   labelled "inside a function here" at the function's keyword, and, for
//!   a label, "the loop it names is outside it" at the loop's label, with a
//!   note that the body of a function runs when it is called (a `lazy`'s
//!   when it is awaited), which may be after the loop has ended;
//! - no loop it could be for has its label: "no enclosing loop is labelled
//!   `'L`", at the label;
//! - it is an item of a record block: "`break` cannot be an item of a
//!   record block".
//!
//! Its value, if any, is what it contributes to its target's list (see
//! [For](#for)), checked against the target's expected element type if it
//! has one; where there is no target, it is checked on its own.
//!
//! # Labels
//!
//! A loop's label, `for'L` or `do'L`, names it for the `break'L`,
//! `continue'L` and `return'L` in its body, and a block's, `{'L ... }`,
//! names it for the `return'L` in it (see [Labelled
//! blocks](#labelled-blocks)). Loop labels and block labels are one
//! namespace, kept on one stack, [`Checker::loops`], of the loops whose
//! bodies and the labelled blocks whose items are being checked
//! (`LoopFrame`; an unlabelled block has none, and costs nothing). A loop
//! or labelled block labelled `L` inside the body of one labelled `L` of the
//! same function is "label `'L` is already in use", at the inner label,
//! labelled "the loop around this one is labelled so" (or "the block") at
//! the outer, with the note "give the inner loop another label" (or
//! "block"); a function or `lazy` nested in a loop or block starts with no
//! labels, and an unused label is fine. A labelled `break` or `continue`
//! leaves every loop between it and its target, and its value joins the
//! target's element type only. An unlabelled `break` or `continue` is for
//! the innermost loop, whatever labelled blocks lie between; a labelled one
//! whose label names a block is "`'L` labels a block, not a loop", at the
//! keyword and its label, labelled "the block is labelled here" at the
//! block's label, with the note "use `return'L` to leave a block".
//!
//! # `return`
//!
//! `return`, `return EXPR`, `return'L` and `return'L EXPR` are items, as
//! `break` is (`Checker::check_return`). Each is for a *target*, recorded
//! for lowering in [`Resolution::target`]:
//!
//! - unlabelled, the innermost function or `lazy` being checked, however
//!   many blocks, labelled blocks and loops of it lie between; at the top
//!   level, outside any, it is "`return` outside a function";
//! - labelled `L`, the innermost loop or labelled block labelled `L`
//!   holding it, of the same function or `lazy`, found as a `break`'s loop
//!   is ([`Checker::jump_target`]): none at all is "no enclosing block or
//!   loop is labelled `'L`", at the label, and one only outside the
//!   innermost function or `lazy` is "`return` cannot leave a function" (or
//!   "a `lazy`"), as for `break`, labelled "the block it names is outside
//!   it" (or "the loop") at its label. A loop's iterable, guard, condition
//!   and header items are not in its body, so a `return'L` there is not for
//!   it.
//!
//! As an item of a record block it is "`return` cannot be an item of a
//! record block". A block whose last item is a `return` is `Never`, and a
//! `return` never completes, for the unreachable warning, unless it is in
//! error. A `return` without a value returns `{}`.
//!
//! Each target has *result sites*: its own result, and the value of every
//! `return` for it (a `Site`, gathered on its frame as it is checked). How
//! they combine depends on the target:
//!
//! - **A function.** Every value returned is treated exactly as the body's
//!   result is, at every place the checker treats that result specially:
//!   it is checked against the declared result type, as the body's result
//!   is, with [`Checker::check_expr_expecting`] (a mismatch is "mismatched
//!   types" at the value, labelled "the return type is declared here" at
//!   the type, and a `return` without a value that `{}` does not fit is the
//!   same at the `return`, with a note that it returns `{}`), so that an
//!   untyped literal adapts, a value that fits once awaited is awaited, a
//!   record block is checked field by field and a list literal, a block
//!   with a result or an `if` against a list type or a union, as the body
//!   is, and a record literal such a block or `if` results in is given the
//!   defaults of a record result type ("Field defaults" under
//!   [Functions](#functions)); the pure function's parameters it holds are
//!   measured where the declared type is present ([Inferred
//!   parameters](#inferred-parameters), through `Checker::expect`); a
//!   callback parameter returned is used as a value, and a call of one in
//!   it records its argument ([Narrowed
//!   callbacks](#narrowed-callbacks)), as anywhere in the body; and an
//!   impure or memoised function's bare result is inferred from every
//!   site, each as the body's result alone would be, dotted fields
//!   included (see [Inferred results](#inferred-results)).
//! - **A `lazy`.** A `return` leaves the `lazy`, whatever lies between,
//!   and its value is the future's: it does not leave the function around
//!   the `lazy`, which may have finished, and is no result site of it. The
//!   `lazy` is `Future<T>`, `T` the join of its operand's type and every
//!   value returned, as an `if`'s branches are joined, untyped integers
//!   adapting to each other and then settling to `Int`; a join that widens
//!   is a warning, "the values this `lazy` returns have different types",
//!   at its keyword, labelled at each site, with the union note. Nothing
//!   is expected of a `lazy`'s operand, so nothing is of its `return`s: a
//!   type expected of the `lazy` is fitted by the `Future` as a whole, as
//!   before (`Checker::returned_lazy`).
//! - **A labelled block or a loop**: see [Labelled
//!   blocks](#labelled-blocks) and [Loops that return](#loops-that-return).
//!
//! # Labelled blocks
//!
//! A block labelled `L`, `{'L ... }`, any block that is not a record block
//! (which the parser rejects), evaluates to the value of a `return'L` in
//! it at once. It is checked as any block is, with a frame on
//! [`Checker::loops`] around its items (`Checker::labelled_block`, and
//! for a function's body, which is a block like any other,
//! `Checker::check_fn_body`); its type is the join of its own result and
//! every value returned to it (`Checker::leave_block`), as an `if`'s
//! branches are joined: untyped integers adapt to each other and to the
//! numeric types beside them, and stay untyped if all are, a later
//! [`Checker::coerce`] reaching the values returned as it reaches the
//! block's result. Where a list type or a union is expected of the block
//! (as it reaches a block's result: see [Lists](#lists)), each value
//! returned is checked against it, as the result is, and the join is no
//! warning; a record type with defaults, which reaches a record literal
//! its result is for its defaults alone (see "Field defaults" under
//! [Functions](#functions)), reaches no value returned, which is checked
//! as ever; otherwise a join that widens is a warning, "the values this
//! block returns have different types", at its label, labelled at each
//! site, with the union note. A block every way through which returns is
//! of the values' type; a block that returns on no path is `Never` if its
//! result is. A labelled loop body is a labelled block like any other: in
//! `for x in xs {'it if x < 0 { return'it 0 }; x }`, `0` is what the
//! iteration contributes, as `continue 0` would make it.
//!
//! # Loops that return
//!
//! A `return'L` whose `L` labels a loop makes the whole loop evaluate to
//! its value at once, not to a list. The loop's type is the join of its
//! ordinary type, its list or `Never` (see [Never-ending
//! loops](#never-ending-loops)), and every value returned to it
//! ([`Checker::returned_loop`]). Without an expected type, untyped values
//! settle against the list, and a join that widens, as `List<Int> | Int`
//! does, is a warning, "this loop is its list or a value returned to it, of
//! different types", at the loop's keyword, labelled "its list is
//! List<Int>" there and with its type at each `return`, with the union
//! note. Where a type `T` is expected of it, which reaches a loop directly,
//! through parentheses, wherever an expected type reaches an expression,
//! and through a block's result or an `if`'s branches where a list type or
//! a union is expected ([`Checker::check_loop_expecting`]): every value
//! returned is checked against `T`; its body is checked against the
//! element type of `T`'s list type, when `T` is one, or a union with
//! exactly one, as before, and against nothing otherwise; and a loop that
//! can end must fit `T` as a list too, or it is "this loop may end as a
//! list, `List<{}>`, which is not Int", at its keyword, with the note
//! "annotate it as `Int | List<{}>`, or return from every way the loop can
//! end". A loop no `return` leaves keeps the generic mismatch where its
//! list does not fit. Which loops a `return` leaves is recorded for
//! lowering in [`Resolution::loop_of`].
//!
//! # Never-ending loops
//!
//! A `for BLOCK` loop, written with no condition (not `for true`, nor a
//! `do` loop), that no `break` is for, its own or a labelled one from a
//! loop inside it, can never end normally: its ordinary type is `Never`,
//! not a list, whatever its body's type, and it builds no list
//! ([`Resolution::loop_of`]). So `let v: Int = for'spin { ... return'spin
//! r }` is an `Int` with no union and no warning, a bare `for { ... }`
//! statement with no `break` is `Never`, and what follows it in its block
//! gets the unreachable warning; a function whose only result site is
//! such a loop has a body of type `Never`, from which its result is
//! inferred (a present `T`, honestly: it never returns a future). Its body
//! and its `continue`s still make an element type, as any loop's, with
//! the same warning, though no list is ever built.
//!
//! # Never
//!
//! `Never` (see [`crate::types`]) is the type of what never completes: a
//! block whose last item is a `break`, `continue` or `return`, a loop that
//! cannot end (see [Never-ending loops](#never-ending-loops)), and so
//! anything whose value is such a block's. Everything else follows from the join, `Never`
//! being its identity: an `if` both of whose branches are `Never` is
//! `Never`, `if c { break } else { 5 }` is `Int`, and an `if` without
//! `else` whose branch is `Never` is `{}`.
//!
//! Code that cannot be reached is never an error because of `Never`. Where
//! a rule asks what an operand *is*, a record for a field access, a list to
//! index, iterate, spread or count, a function to call, a number for
//! arithmetic, a `Bool` condition, an operand of `is`, `as`, `await`, a
//! comparison or an interpolation, an operand of type `Never` satisfies it,
//! and the operation is `Never` if that operand always runs first: so `{
//! break } + 1`, `f()({ break })`, `{ break }.a`, `if { break } { 1 } else
//! { 2 }`, a loop whose iterable or first condition is `Never` and `"{ {
//! break } }"` are `Never`. An operand that may not run asks nothing that
//! makes its operation `Never`: the right operand of `and` and `or`, a
//! `for`'s guard and a `do`'s condition, which comes after the body, the
//! operation keeping its own type. Where a type is only compared with an
//! expected one, an argument, an annotated `let`, a list element, a field,
//! subtyping accepts `Never`, so `f({ continue })` is `f`'s result and `[{
//! break }]` a `List<Never>`. A variable, a list element or a field may be
//! of type `Never`, a value that is never there: a `:=` or a dotted `let`
//! of such a variable, or along a path through such a field, is checked on
//! its own and declares `Never`, quietly.
//!
//! **The unreachable warning.** In a block, the first item after one that
//! never completes, a `break`, `continue` or `return` (not one that is an
//! error, in a record block or with nothing to leave), or an expression, result,
//! `let` or `:=` item whose
//! value is `Never`, is warned about once: "this is never run", labelled
//! "this always leaves the block" at the earlier item. The block's type is
//! still its last item's.
//!
//! # Unions
//!
//! `Int | String` is the type of a value that is an `Int` or a `String`;
//! any present types may be members (see [`crate::types`]). The checker
//! does not block where two types merely disagree: it joins the types of
//! the expressions whose values one expression may have, the branches of
//! an `if` and the elements of a list literal (and its spreads' elements),
//! and the join always exists ([`TypeTable::join`]). Where two types have
//! no common supertype, at the top or inside a record field, a list's
//! element type or a function's return type, the join is their union, and
//! *widened*; a widened join is a warning, not an error, at the expression
//! that does not agree with what came before it:
//!
//! - an `if`'s branches, "the branches of this `if` have different types",
//!   at the `else` branch's value (a block's result, through blocks and
//!   parentheses), labelled with its type, with a label of the `then`
//!   branch's value's type on it, and the note "the `if` is `Int |
//!   String`; annotate it with that type if both are meant, or tell them
//!   apart with `is`" (`Checker::branches_widened`);
//! - a list literal's elements, "the elements of this list have different
//!   types", at the first element (or spread) that widens the join of those
//!   before it, labelled with its type, with a label of the elements' so far
//!   on the first, and the note "the list is `List<Int | String>`; ..."
//!   (`Checker::elements_widened`).
//!
//! There are no other joins of expressions' types that could widen: a
//! field access, an index or an iteration of a union whose members are all
//! records with the field, or all lists, has the join of what the members
//! have there, which is no warning, since the union says the value may be
//! any of them. Nor is there a warning where a type is expected that the
//! union fits: the expected type reaches the branches of an `if`, a
//! block's result and a list's elements (see [Lists](#lists)), each is
//! checked against it, and the join of what they are is no warning. So
//! `let w: Int | String = if c { 1 } else { "one" }` warns about nothing.
//! An integer literal adapts to a union's `Int` member, or its `Float`
//! member if it has no `Int` one; one that cannot adapt to the other side
//! of a join settles to `Int` first, so `if c { 1 } else { "a" }` is `Int |
//! String`.
//!
//! A union is used by narrowing it (see [`is`](#is), [Narrowing](#narrowing)
//! and [`as`](#as)). Used as it is where one particular type is needed, an
//! operand of an operator, a condition, an index position, a range bound,
//! a callee, the record of a field access, the list of an index, an
//! iteration, `len` or a list's spread, it gives that place's existing
//! error, the union shown in it ("cannot add `Int | String` and Int"),
//! with the note "tell them apart first, as in `if v is Int { ... }`",
//! naming the variable when the operand is one, through parentheses, and
//! `x` otherwise, and the union's first member that `is` can test, as the
//! union shows its members, or, for a union of function types only, which
//! `is` cannot tell apart, a note to annotate the value with one function
//! type (`Checker::union_note`). An expected type the
//! union does not fit gives the mismatch it always has, with the same note.
//! A field access is allowed when every member is a record type with the
//! field, and an index, a `for` and `len` when every member is a list type;
//! a spread needs a list, or a record, as before, and the path of a dotted
//! `let` or `:=` records or lists, as before, without the note.
//! Interpolating, passing, storing,
//! returning and capturing a union need nothing. Awaiting an either of a
//! union gives the union.
//!
//! A union whose members are of different kinds is of
//! [`ValueKind::Mixed`] at runtime: lowering gives its nodes to the
//! evaluator's general walker only, as it does a value that may be a
//! future (see [`crate::lower`]).
//!
//! # `is`
//!
//! `EXPR is TYPE` is a `Bool`: whether the value is a `TYPE`
//! (`Checker::check_test`). `TYPE` is resolved with every bare type
//! present, and may hold no future, nor an either shape ("`is` cannot test
//! a future"), and no function type ("`is` cannot test a function type":
//! a function's signature is not something a value can be asked for; a
//! function is told apart by the other members, in the `else` branch). The
//! operand is checked, an untyped integer settling to `Int`, and measured
//! at the top, as a measuring operator's operand is: awaited if it may be a
//! future, subject to the purity rule, a pure function's parameter being
//! measured. Its type then, `S`, has *members*: its members if it is a
//! union, and itself otherwise. Each member `M`, made present (a list of
//! futures being asked about as a list of what they yield), *always
//! passes* if it is a subtype of `TYPE`, *may pass* if not but it
//! *overlaps* `TYPE` ([`TypeTable::overlaps`]: a value could be both, as
//! far as their structure says), and otherwise *cannot pass*. If every
//! member always passes, the test is `true`, and a warning, "`x` is always
//! an Int", at the `is`; if none passes or may, it is an error, "`x` is
//! never a String", at `TYPE`, labelled with the operand's type at the
//! operand. `x` is the operand as written when it is a name or a dotted
//! path, and "this value" otherwise.
//!
//! The test at runtime is by the value's structure, as its *plan*
//! ([`TypeTable::test_plan`]) says, which is recorded for lowering in
//! [`Resolution::test_of`]: a scalar type by the value's kind; a record
//! type by the record's having each field, each passing its own test,
//! whatever else it has; a list type by each element's passing (the empty
//! list passes every list type); a union by any member's. The plan looks no
//! further than `S` leaves in doubt: if every member of `S` of the kind
//! tested always passes, it tests the value's kind alone (`x is List<Int>`
//! for `x: Int | List<Int>`), and otherwise it looks inside, applying the
//! same rule at each field and element, from what `S`'s members of that
//! kind say of it (or nothing, where one lacks the field). How deeply the
//! operand is awaited follows the plan: at the top for one that tests the
//! kind alone, so that `xs is Int` awaits no element of a list of futures,
//! and deeply, as an operand of an equality is (`Checker::measure_deep`),
//! for one that looks inside, `S` then being made present for everything
//! below. Values are immutable and types structural, so a value that
//! passes the test of a type is of that type, whatever it was built as,
//! which is what makes narrowing on the test sound.
//!
//! The *narrowed type* `N` is the union of each member that always passes,
//! as it is, and `TYPE`, if any member may pass; the *remainder* `R` is the
//! union of the members that do not always pass. "May pass" asks whether
//! `M` and `TYPE` overlap, not merely whether `TYPE` is a subtype of `M`, so
//! that `x is {value: Int}` is a real test for `x: {kind: String, value:
//! Int | String}`, and so that a member that cannot pass is one no value of
//! which passes the test: were it one whose record could have a field its
//! type hides, such a record could pass and be taken for another member's
//! type than its own.
//!
//! # Narrowing
//!
//! `if NAME is TYPE { A } else { B }`, the condition being exactly that
//! through any parentheses, and `NAME`, through any parentheses, a
//! variable, mutable or not, a parameter, a field, a `for`'s variable or a
//! variable of a `for` header,
//! narrows `NAME` in the branches (`Checker::if_condition`). The `is`
//! writes the value it tests, awaited as the test awaited it, to a slot of
//! its own, taken for the `if` and free again after it; in `A`, `NAME` is a
//! new binding of `N` in that slot, in a scope around `A`, as a `for`'s
//! variable is around its body, and in `B` a new binding of `R` there in
//! the same way, unless `R` is the whole of `S` (no member always passes)
//! and the test awaited nothing, when `B` keeps `NAME` as it was. Each is
//! a copy (`BindingKind::Narrowed`), declared, as its diagnostics say, by
//! the `is`. Being a copy, nothing that rebinds the variable it was copied
//! from can change it, which keeps the narrowing sound for a mutable
//! variable. For the same reason `NAME := v` in the branch is an error,
//! "cannot rebind `x` here: it is narrowed by `is`": it would change the
//! copy and leave the variable as it was, silently for a mutable one, so
//! the note says to test a copy under another name, which leaves the
//! variable free to rebind. A `let NAME` in the branch shadows it as any
//! `let` does. When the
//! test always passes (and is warned about), `A` is narrowed and `B` keeps
//! `NAME`. An `else if` is an `if` inside `B`, narrowing in turn. Both
//! paths that check an `if`, with and without an expected type, narrow
//! alike.
//!
//! Nothing else narrows, for now: not `x is Int and x > 5`, not `not (x is
//! Int)`, not a field path (`r.a is Int`), and nothing after the `if` (an
//! early exit from `A` does not make `NAME` the remainder after it). These
//! are flow narrowing, deferred to a later slice.
//!
//! # `as`
//!
//! `EXPR as TYPE` is the operand as a `TYPE`, or a runtime error: a checked
//! narrowing, never a blind promise. The operand and `TYPE` follow the
//! rules of `is`, the same members, the same plan, awaited the same way,
//! the same "never" error and the same rejection of futures and function
//! types; if every member always passes, it is a warning, "`as` changes
//! nothing here: `x` is already an Int", with the note "`: Int` says the
//! same without a test". Its type is `N`, `TYPE` or what is more precise,
//! and its value the operand, awaited as the test awaited it. The test runs
//! at runtime, and a value that fails it is "`x` is not an Int", at the
//! whole expression, labelled with what the value is (see [`crate::eval`]).
//! It does nothing impure, so a pure function may use it on an operand that
//! needs no await.
//!
//! # Built-in functions
//!
//! The built-in functions are the members of the `builtins` module, a
//! module implemented in Rust (see [`crate::workspace`], "The standard
//! library"): `len`, `int`, `float`, `exec`, `panic`, `split`, `join`,
//! `starts_with`, `ends_with`, `trim`, `strip_prefix`, `strip_suffix`,
//! `lines`, `fields`, `compare_versions`, the file system's `fs_exists`,
//! `fs_read`, `fs_mkdir`, `fs_copy`, `fs_temp_dir` and `fs_write`,
//! `sort_versions`, `show` and `exec_live`, each a [`Builtin`],
//! with its name, its parameters' names (at most [`Builtin::MAX_PARAMS`],
//! two), its [`Effect`] (`exec`, `exec_live` and the file system's are
//! impure, every other one pure) and its [`Typing`]. The checker knows no name of one: a
//! file has them as it has any other module's members, by an import, or by
//! the prelude, which binds each of them but the file system's to its own
//! name, or by the standard `fs` module, whose members `exists`, `read`,
//! `mkdir`, `copy`, `temp_dir` and `write` are the file system's, bound so
//! too; the prelude is what every file and session input imports
//! implicitly, `#[shadowable]` and first of all in its view's order, but a
//! file marked `#![no_prelude]` (see [`crate::workspace`], "Shadowable
//! declarations"). A file that declares its own `len` (a field, a `def`, a
//! `let`, a `fn`, or a path below it, `len.x`)
//! shadows the prelude's, and loses the built-in there. The cell of each is
//! a slot of the frame the driver writes before anything runs, its value a
//! [`crate::Value::Builtin`], its type each one's, as the workspace's
//! context gives every cell's.
//!
//! An ORDINARY built-in ([`Typing::Ordinary`]) has a function type the type
//! table can hold, of any parameters, result and effect, which
//! [`Builtin::ty`] interns with the table in hand (its parameters' and its
//! result's names into the run's names): `int` (`fn(x: Float): Int`,
//! truncation toward zero), `float` (`fn(x: Int): Float`), `exec`
//! (`fn(argv: List<String>) -> {status: Int, stderr: String, stdout:
//! String}`, which runs a program), `exec_live` (of `exec`'s type, which
//! runs a program as `exec` does, copying its output to standard error as
//! it runs), `panic` (`fn(message: String):
//! Never`, which ends the run with a runtime error of the program's own
//! wording), the nine on strings: `split` (`fn(s: String, sep:
//! String): List<String>`, the pieces between the separators; an empty
//! separator is a runtime error), `join` (`fn(parts: List<String>, sep:
//! String): String`), `starts_with` (`fn(s: String, prefix: String):
//! Bool`), `ends_with` (`fn(s: String, suffix: String): Bool`), `trim`
//! (`fn(s: String): String`), `strip_prefix` (`fn(s: String, prefix:
//! String): String`), `strip_suffix` (`fn(s: String, suffix: String):
//! String`), `lines` and `fields` (each `fn(s: String): List<String>`),
//! each as Rust's `str` does it, and `compare_versions` (`fn(a: String, b:
//! String): Int`, `-1`, `0` or `1` as `a` sorts before, equal to or after
//! `b` in version order) and `sort_versions` (`fn(vs: List<String>):
//! List<String>`, the list sorted in that order), and the six of the file
//! system, each impure and each taking only absolute paths: `fs_exists`
//! (`fn(p: String) -> Bool`), `fs_read` (`fn(p: String) -> String`),
//! `fs_mkdir` (`fn(p: String) -> {}`), `fs_copy` (`fn(from: String, to:
//! String) -> {}`), `fs_temp_dir` (`fn() -> String`) and `fs_write` (`fn(p:
//! String, contents: String) -> {}`), which a file reaches as `fs.exists`
//! and so on after `import fs` (see [`crate::eval`], "The built-in
//! functions" for all of them); a file's own `join` shadows
//! the prelude's as any declaration does, and so does `path`'s, in
//! `path.gcsl` and in a file importing it by member or `*`. Each is a
//! value of its type like any function: passed, stored, called through a
//! variable or a parameter, and a call of it is a call
//! like any other, typed by its type, binding its arguments by position or
//! by its parameters' names, awaiting a future argument as any call
//! does, lowered to a `Call` (see [`crate::eval`], which applies a built-in
//! it finds called). So a call of `exec`, of `exec_live` or of `fs.read`,
//! from a pure
//! function is "cannot call an impure function from a pure function", as
//! for any impure
//! function, and from a `lazy`, an impure or memoised function or the top
//! level it is fine. Like any function it has no equality: `int === int` is
//! "cannot compare functions".
//!
//! A call of `panic` is of type `Never`, its result type, as a `break` is:
//! `if c { panic("x") } else { 1 }` is an `Int`, an `if` whose branch
//! panics needs no `else`, and what follows a panicking statement is never
//! run (see [Never](#never)). `panic` fits a parameter of type `fn(String):
//! Int`, since `Never` is a subtype of every result. `Never` is shown in its
//! type although no annotation can write it.
//!
//! A SPECIAL built-in ([`Typing::Special`]), `len` or `show`, is typed call
//! by call by the checker, since it is generic. `len(list)`'s argument is
//! measured at the top, its elements not at all, and must be a list ("`len`
//! takes a list, found T"), and the call is an `Int`. `show(value)`'s
//! argument may be of any type with no function in it anywhere, a field,
//! an element, a member of a union, an alias's expansion, at any depth, a
//! built-in function's value being one too: else "cannot show a function:
//! `show` renders data as source", at the argument, noting "the argument is
//! of type `{run: fn(): Int}`" (`Checker::check_show`); it is used as its
//! bits all the way down, as an interpolated expression is, so awaited
//! deeply if it may be a future or hold one, which a pure function cannot
//! do; and the call is a `String`, the value's GCSL source (see
//! [`crate::eval`], "The built-in functions"). A call of one binds its
//! arguments by position or by the table's names, with the errors of any
//! call's arguments (`Checker::bind_builtin_arguments`); its callee then
//! resolves to [`VarRef::Builtin`] and has the function type of this call,
//! and lowering makes the call the built-in's own node, `ListLen` or
//! `Show`, never reading the callee. Calling a built-in that is not pure
//! from a pure function is an error, as for any function.
//!
//! ## The alias rule, temporary
//!
//! TEMPORARY (alias rule): a special built-in has no type the table can hold, so until the
//! inference engine gives built-in functions real polymorphic types (see the [inference
//! limits](https://learn.peios.org/gcsl/language/inference-limits/)), its value has an opaque type
//! of its
//! own, [`TyKind::Builtin`], shown `builtin len` or `builtin show`, which no type expression can
//! write, so that no annotation, parameter type, field type or result type accepts it. A reference
//! whose type is it may stand in two places only (`Checker::alias_site`): the callee of a call,
//! which is then typed by the built-in's rule as above, and the whole initialiser of a top-level
//! field or a top-level `let` of one name, neither annotated nor mutable (`len = builtins.len`,
//! `let l = len`, `m = l`), which makes that name an *alias* of the built-in, of the same opaque
//! type. A member or `*` import of one is an alias of the same kind, the cell it names being of
//! that type. The type is the alias's identity: it is what every boundary already carries, the type
//! of a cell another file reads ([`crate::workspace`]'s cell types), of a root committed to a
//! directory's scope, of a session's [`Scope`] binding, of an import's target, so a call through an
//! alias is typed and lowered as a call of the built-in in any file or session input. Any other
//! reference to one, or to a record holding one (`builtins` or the prelude read whole), is "`len`
//! is a built-in function and cannot be used as a value yet", with the note "a built-in function
//! with a type for any list cannot be written as a type yet; call it, or bind it to a name" (for
//! `show`, "for any value"; `Checker::builtin_use`, the one place that says so): an argument, a
//! list element, a field of an inner record, a function's result, an operand of `==` or `===`, an
//! `is`, an `as`, an ascription, a `let` in a block, a `let mut`, a rebind, a branch, a dotted
//! field, a callee in parentheses. What replaces it: a polymorphic type for `len` (`fn<T>(list:
//! List<T>): Int`) and for `show` (`fn<T>(value: T): String`, `T` bounded to types without
//! functions), with which each is a value like `int`, its calls ordinary calls, and the opaque
//! type, `alias_site`, `builtin_use` and the special path of `Checker::check_call` go.
//!
//! # Scopes
//!
//! Every block, including the file's own body, is a scope, and so is the
//! scope a loop declares its variable, or its header's items, in (see
//! [For](#for) and [Loops](#loops)). A `let` declares
//! a variable visible from the next item to the end of its block; a later
//! `let` of the same name shadows it, in the same block or an inner one. A
//! `:=` is a new declaration too, allowed only for a name already declared
//! in the same block, and not for a field; its value must fit the
//! variable's type (a subtype, or an adapting literal), and the new binding
//! keeps that type, so `:=` reads as assignment: a `let` is needed to
//! change a variable's type. A `:=` of a mutable variable, a `let mut`'s, is
//! allowed from any block of the variable's function, and declares
//! nothing: it is an assignment to the variable, which keeps its type (see
//! [Mutable variables](#mutable-variables)).
//!
//! The scopes in effect form a stack, kept as one flat vector of bindings
//! with the innermost block's bindings last: entering a block records where
//! its bindings start, and leaving it truncates the vector back. Lookup
//! searches the vector from the end, which visits the innermost block first
//! and, within a block, the latest declaration first, so shadowing needs no
//! extra work. Blocks are short, so a linear search beats a hash map, and
//! the one vector means entering a block allocates nothing.
//!
//! # Slots
//!
//! Every variable and field is stored in a [`Slot`] of its function's flat
//! frame (the top level's, outside functions), assigned stack-fashion, as
//! in Lua and clox: entering a block, or a `for`'s scope, records the next
//! free slot; each declaration (a parameter, a `let`, a field, a `for`'s
//! variable, and `:=` of a variable that is not mutable, which is a new
//! binding) takes the next free slot; leaving the block restores the next
//! free slot to what it was on entry, so the slots of a finished block are
//! reused by whatever comes after it. A `:=` of a mutable variable takes
//! none: it writes the variable's one slot, from whatever block it is in,
//! all of which end before the variable's own block does, so the slot is
//! never reused while the variable is in scope. The highest slot count
//! reached is the
//! frame size. A variable's slot is therefore known statically and never
//! changes, and evaluation needs no environment lookups and no allocation
//! per block. Some items share a slot: each definition of a field after its
//! first, a dotted path extending an open field or an item overriding a
//! soft one, writes the field's one slot. The top-level functions of an
//! input take its first slots, in source order, before the walk starts
//! (see [Top-level functions](#top-level-functions)); a `def` among them is
//! a field whose slot is the function's. The input's top-level field roots
//! take the slots after them, and every other node of the tree of its
//! top-level fields a slot after those, its cell, which no name finds (see
//! [Top-level fields](#top-level-fields)); so a scope's next free slot is
//! not always its bindings' count, and [`Scope`] records it. A spread's
//! operand is held in the next free slot,
//! which counts towards the frame size but stays free (see [Spreads and
//! soft assignment](#spreads-and-soft-assignment)). An `if` that narrows
//! takes the next free slot for the value its test writes, which both
//! branches' narrowed bindings share, and leaves it when it ends (see
//! [Narrowing](#narrowing)).
//!
//! # Sessions
//!
//! A [`Checker`] can start from an existing outermost [`Scope`], which it
//! returns extended by the new input's top-level declarations. This is what
//! lets a [`crate::session::Session`] check each input in the context of the
//! variables its earlier inputs declared, with the same code that checks a
//! file. The scope keeps each binding's kind, so a later input may rebind an
//! earlier input's mutable variable from any block of its top level, but
//! not from a function or a `lazy`, as if it continued the same block; and
//! it lists their slots ([`Scope::mutable_slots`]), the only ones of the
//! scope a later input can write.
//!
//! Within an input, top-level functions are in any order (see [Top-level
//! functions](#top-level-functions)); between inputs, order is as it was: a
//! later input sees the earlier ones, and an earlier one is never changed by
//! a later one. A later input's top-level functions take its own first
//! slots, after the scope's, and may hide an earlier input's names, and an
//! earlier input's functions keep reading their own input's slots, which no
//! later input writes. Nothing about top-level functions is kept in the
//! [`Scope`] beyond their bindings.
//!
//! The same holds of top-level fields. Every cell slot of an input, a root's
//! or another node's, is committed with the input: the scope's next free
//! slot is past the last of them, so no later input reuses one, and a
//! closure or a future of the input reads its cells by slot for as long as
//! it exists. A failed input's cells are forgotten with it, as its other
//! slots are. A later input sees an earlier input's field root as a field
//! of its type, whole, as before this order-free model; `a.c = 2` in a later
//! input declares that input's own `a`.
//!
//! # Directory scope
//!
//! The files of a [`crate::workspace`] are checked as the inputs of a
//! session are, in the workspace's order, each by `Checker::in_workspace`
//! from a scope joined from the scopes of its directory and of each
//! directory above (only their declarations of names the file spells, which
//! are all it can look up, so that checking stays linear in the files),
//! but with what the scan found: the file's mentions are
//! resolved against its view (the merged trees of its directory chain and
//! what it imports, see [`crate::workspace`], "The view of a file"), and a
//! context gives every cell of the workspace its slot (its node's index
//! among the workspace's nodes, slots below every private one, the
//! built-in functions' the first; a session's input's own cells take the
//! slots after the session's instead) and, once the file that computes it
//! is checked, its type. A cell of another file
//! is read as a cell of this file is: a load of its slot at the top level,
//! and inside a function or a `lazy` a read of the top level's frame, never
//! a capture, since it is written before this file runs. Only a root has a
//! binding, committed to its directory's scope once the last file
//! contributing to it is checked (its type is whole only then), and every
//! `def` has one, so calls of another file's `def` are checked as calls of
//! an earlier function. A root that several sources of the file's view
//! have (`com` in a directory and the directory above) is never read
//! through a binding: it is a node of the view (see [Imports](#imports)).
//! An interior node shared with other files of the directory has their
//! children in its type. The file's own leaves are as in [Top-level
//! fields](#top-level-fields).
//!
//! What a file declares privately (`let`, `let mut`, `fn`, and the soft
//! fields of its spreads) is not committed: later files are checked from a
//! scope without those bindings, though their slots stay taken, since the
//! file's closures read them. A top-level `let`, `let mut`, `:=`, dotted
//! `let` or `fn` of a name that is a root of its directory's tree or of a
//! directory above, declared in another file, is "`a` is a field of this
//! directory", labelled on the declaration, unless what the file's view has
//! of the name is shadowable, which it then shadows silently
//! ([`crate::workspace::FileScope::yields`]); within a file, "`a` is a field
//! of this file" stands as it was, but for a shadowable root (see [Shadowed
//! declarations](#shadowed-declarations)).
//!
//! A file is checked after every file it needs, so their cells' types are
//! known; there is nothing to be ready across files, since a needed file
//! runs as a whole first. Readiness ("cannot be used here yet") is within
//! a file only. Effects follow: every effect of a needed file happens
//! before the first of the file that needs it.
//!
//! # Imports
//!
//! An `import` names a node of another part of the workspace, or a module
//! of the standard library (see [`crate::workspace`], "Imports" and "The
//! standard library"), and the scan has resolved it, and reported its
//! clashes ("`lib` is imported in this file", "`version` is a field of this
//! directory"), before any file is checked. A lone file and a REPL input
//! are checked beside the standard library, with no workspace: a path of
//! anything else names nothing, and the checker, which knows its context is
//! a lone one, reports "`lib` is not a built-in or standard module, and
//! there is no `workspace.gcsl` above this file" at the import, with the
//! input's other errors. A file checked with no context at all ([`check`])
//! has nothing to import: "`import` needs a workspace: no `workspace.gcsl`
//! above this file". Otherwise the item itself checks nothing.
//!
//! An imported name, and a root of the file's view that several sources
//! have, has no binding. A name the scope does not bind nearer (a
//! parameter, a local `let`, anything but the top level, shadows it) and
//! that the file's [`crate::workspace::FileScope`] reads through its view
//! ([`crate::workspace::FileScope::reads_through`]) is a reference,
//! resolved like a reference to a field: to the longest prefix the view
//! declares, the accesses after it ordinary ones. A name the view has as
//! something shadowable (the implicit `#[shadowable] import * from
//! prelude`'s names among them, see [`crate::workspace`], "Shadowable
//! declarations") yields instead to a binding of its name that is not a
//! declaration of the view's own order
//! ([`crate::workspace::FileScope::yields`],
//! `Checker::binding_shadows_view`): a top-level `let` or `fn` of the file,
//! or an earlier session input's variable, shadows it, and no clash is
//! reported ("`len` is a field of this directory" is not); a field root of
//! the file or one its directory chain committed is what the view says it
//! is. An import a
//! session's input inherits from an earlier input
//! ([`crate::workspace::FileScope::inherited`]) is shadowed as an earlier
//! input's variable is, with no clash reported: by the input's top-level
//! `fn`, field, `def` or `:=` of its name in the whole input (the scan
//! drops it), and by a top-level `let` from its point on. What the prefix
//! reaches decides
//! what is read, always a cell, so never a capture: a load of its slot at
//! the top level, a read of the top level's frame inside a function or a
//! `lazy`.
//!
//! - A node of a directory's tree is read from its cell, its type the
//!   cell's. A `def`'s cell is its function's slot (the merged node of a
//!   `def` is never written), and its type the function's.
//! - A value file's value is read from the cell the file stores it in, its
//!   type the file's.
//! - A directory, a file, a file's part of a shared node, or a node of the
//!   view that several sources have and no leaf of the file is under, read
//!   whole, is a record the reading file assembles itself: before the walk,
//!   the checker gives each such target the file's mentions reach a private
//!   top-level slot of its own (nested parts first), with the record type
//!   of its roots or children, each a cell's slot and type as above;
//!   lowering builds them before the file's first item. Where a reference
//!   is checked the record is only looked up, never allocated, so that no
//!   such slot is taken from a function's frame.
//! - A node of the view that several sources have and a leaf of the file
//!   is under is one of the file's field nodes, numbered after its tree's
//!   ([`crate::mentions::Mentions::views`]): it has a private slot, and,
//!   read whole, it is computed at its point in the file's schedule as an
//!   interior node is (`Checker::compute_view`), its type the record of its
//!   children in the view's order (root directory first), the file's own
//!   as computed by then and every other one another file's cell or a
//!   record assembled as the file started. Its readiness and cycles are an
//!   interior node's: "`com` has `com.b.v`, and ...".
//!
//! A call of a `def` reached through an import (`f(x)` after `import f
//! from lib`, or `lib.f(x)`) is of a known callee, `KnownCallee::Imported`,
//! whose measured parameters and defaults the workspace carries
//! ([`KnownParams`]), so its call-site diagnostics are those of a `def` of
//! the directory, and it is given its defaults (see [Calls](#calls)); the
//! prelude's `half`
//! is such a `def`. So is a call of a `def` named by a dotted path, through
//! an import or by its path from another file of the directory chain
//! (`dev.peios.f(x)`), which reads the root the chain committed, a record
//! that knows no function: the reference is resolved in the file's view to
//! the merged node (`Checker::committed_callee`), whose measured parameters
//! and defaults the workspace carries as a `def`'s of one name (a dotted one
//! has no binding: they are its function's).
//!
//! The `fileroot` module (see [`crate::workspace`], "The standard library")
//! is no record: a reference through its whole import resolves, as any
//! does, to the longest prefix it declares, a member or the module itself
//! ([`crate::workspace::Target::Fileroot`]), and is typed by
//! `Checker::fileroot_reference`, resolving to [`VarRef::Fileroot`], which
//! lowering makes the member's own node. `fileroot.this`,
//! `fileroot.workspace` and `fileroot.caller` are each a `String`;
//! `fileroot.helper`, only as the callee of a call (not in parentheses), is
//! a pure function of nothing to `{}`, so `fileroot.helper()` may be called
//! from any function and at the top level, and its arguments are bound as
//! any function's ("expected 0 arguments, found 1"). The module's own name
//! is typed `{}`, which nothing reads. Every other use is an error, naming
//! the module by the name the file imports it as: the module read whole,
//! "`fileroot`'s members are read as `fileroot.caller`"; `helper` not
//! called, "`fileroot.helper` is not a value"; a name it lacks, "`fileroot`
//! has no `nope`" (`Checker::fileroot_lacks`); and a member used as what it
//! is not, the error any `String` gets, as `fileroot.this(1)`'s "cannot
//! call a value of type String". A nearer binding of the name shadows the
//! import, as it shadows any. The checker records the file's place, given
//! by the workspace's context ([`Place`]), in the resolution, for lowering.
//!
//! A session's input is checked beside the standard library as a lone file
//! is, from the session's scope ([`crate::workspace`]'s context marks it a
//! session's): the variables of earlier inputs are read as a session's
//! input reads them, captured by a function that reads one, rather than
//! through the top level's frame as a directory's committed declarations
//! are.
//!
//! # Recording
//!
//! A language server answers what a name refers to and what it is from an
//! index of the workspace ([`crate::index`]), which needs what the check
//! found of the names it resolved by scope, since scopes are gone once it
//! ends. A check asked to (`Checker::recording`, which only the index's
//! entry points do: never `gcsl run`'s, a session's or `pekit`'s) records
//! (`Record`) each binding as it is declared, where, under its name, of its
//! type then and of what kind, and each `Name` it reads from a binding
//! declared in this input, by where that binding was declared. A binding
//! committed by another file, or by an earlier input of a session, was
//! declared nowhere here, and a read of it is recorded as none, as is a
//! name read through the file's view: the index resolves those again in
//! the view, as the check does (see [Imports](#imports)), and so a read of
//! the input's own top-level field or `def`, whose declarations are nodes
//! of the view. A name read from the narrowed copy of a variable is read
//! from the `is` that narrowed it, and a function's name for itself from
//! the function expression. A `:=` is recorded with the binding it
//! rebinds: a variable's declares one in its place, which an index takes as
//! the same variable. `Checker::check_recorded` returns the record
//! with the type of every expression, of every `type`
//! item and of every cell of the input's tree as the check left them,
//! WHETHER IT SUCCEEDED OR FAILED, so that a file with errors is indexed
//! for everything that resolved: an expression that failed is `Error`, and
//! after a failure some may be untyped or provisional. Not recording costs
//! one test of an `Option` per binding declared and per name read.
//!
//! # Suggestions
//!
//! An error about a name that names nothing carries a suggestion of what
//! it was probably meant to be, if a name usable where it is written is
//! close enough by [`crate::suggest`]'s rule, as the exact span of the name
//! and its replacement ([`crate::diagnostic::Suggestion`]):
//!
//! - an undefined variable: the bindings in scope, the nearest first (the
//!   innermost block's, then the enclosing ones', then the top level's,
//!   then those the input was checked from: committed by its directory
//!   chain or an earlier session input), then the roots of the file's view
//!   (its own, its directories', what it imports, the implicit prelude's),
//!   leaving out the top-level field whose initialiser is being checked,
//!   a name the file reads as `Error`, one that a top-level function
//!   checked early could not read yet, and a name that is no identifier;
//! - a missing field, "no field `x` in ..." and "the meta of `f` has no
//!   field `x`": the record type's fields, a view node's children among
//!   them, quoted as a field's name is if need be;
//! - an unknown type: the types of the file's view, then `Int`, `Float`,
//!   `Bool` and `String`; written with a type argument, `List`, `Future`
//!   and `Present` instead, which alone take one;
//! - a named argument no parameter has: the callee's named parameters that
//!   no other argument fills.
//!
//! The ranks are coarse, as the checker keeps no stack of blocks: names of
//! one rank tie and are taken in alphabetical order. The search runs only
//! as such an error is reported, never on valid input.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use crate::ast::{
    Annotate, ArgKind, ArgRange, Ast, AttributeId, AttributeUse, BinaryOp, DefTypeId, DefTypeRole,
    Effect, Element, ElementRange, Expr, ExprId, ExprKind, FieldDefault, FieldTy, FieldTyKind,
    FieldTyRange, FnDecl, FnId, ForHead, ForId, Item, ItemId, ItemKind, ItemRange, Jump, Label,
    LabelId, MetaTy, Param, PartRange, PathRange, PathSegment, SegmentKind, StrPart, Syntax,
    TypeExprId, TypeExprKind, TypeProblem, UnaryOp, path_span, symbol_span,
};
use crate::diagnostic::{self, Diagnostic, InternalError};
use crate::mentions::{self, FieldLeaf, FieldTree, Mentions, Need, Resolve, Schedule};
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::FieldName;
use crate::suggest;
use crate::types::{AwaitKind, FnTy, Laziness, Meta, TyId, TyKind, TypeTable, ValueTest};
use crate::value::{Quantity, ValueKind, int_to_float};
use crate::workspace::{AttributeLookup, Context, Target};

/// The type of every expression in an [`Ast`], indexed by [`ExprId`].
///
/// A `Types` returned by [`check`] contains neither [`TyId::UNTYPED_INT`]
/// nor [`TyId::ERROR`], nor any provisional type. An expression that is awaited where
/// it is used keeps the type it has before the await; see
/// [`Resolution::await_of`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Types {
    types: Vec<TyId>,
    /// The types of what is not an expression's, usually empty.
    others: OtherTypes,
}

/// The types a [`Types`] holds that are not an expression's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OtherTypes {
    /// The type each `type` item of the tree stands for, by item, in
    /// source order (see [Type declarations](#type-declarations)).
    declared: Vec<(ItemId, TyId)>,
    /// Each def-type reference whose path goes on past the definition it
    /// names, by its id, with the read of the definition and the type of
    /// the reference to it (see [Metas](#metas)); usually empty.
    references_past: Vec<(ExprId, ExprId, TyId)>,
}

impl Types {
    /// Wraps a raw table. Only the checker (and tests) build tables.
    pub(crate) fn new(types: Vec<TyId>) -> Self {
        Self {
            types,
            others: OtherTypes::default(),
        }
    }

    /// If the def-type reference `id` has a path that goes on past the
    /// definition it names, `package dev.peios.testpkg.versions`: the read
    /// of that definition, the prefix of the reference's hidden read that
    /// names it, and the type of the reference to it, the function with the
    /// definition's meta, whose meta the rest of the path reads, the
    /// reference itself having the type of the last field read (see
    /// [Metas](#metas)). `None` for any other expression.
    #[must_use]
    pub fn reference_past(&self, id: ExprId) -> Option<(ExprId, TyId)> {
        self.others
            .references_past
            .iter()
            .find(|&&(at, ..)| at == id)
            .map(|&(_, read, ty)| (read, ty))
    }

    /// The type the `type` item `item` stands for, its expansion, if it is
    /// one the check resolved.
    #[must_use]
    pub fn declared(&self, item: ItemId) -> Option<TyId> {
        self.others
            .declared
            .iter()
            .find(|&&(at, _)| at == item)
            .map(|&(_, ty)| ty)
    }

    /// The type of expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn get(&self, id: ExprId) -> TyId {
        self.types[id.index()]
    }

    /// Number of entries, equal to the number of nodes in the [`Ast`].
    pub(crate) fn len(&self) -> usize {
        self.types.len()
    }

    /// An [`Annotate`] that writes ` : Type` after each node, for
    /// `--dump-types`, showing the types with `table`, the table they were
    /// interned into, and `names`, the tree's names.
    #[must_use]
    pub fn annotation<'a>(&'a self, table: &'a TypeTable, names: &'a Names) -> TypeAnnotation<'a> {
        TypeAnnotation {
            types: self,
            table,
            names,
        }
    }
}

/// Writes ` : Type` after each node. Returned by [`Types::annotation`].
#[derive(Clone, Copy, Debug)]
pub struct TypeAnnotation<'a> {
    types: &'a Types,
    table: &'a TypeTable,
    names: &'a Names,
}

impl Annotate for TypeAnnotation<'_> {
    fn annotate(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            " : {}",
            self.table.display(self.types.get(id), self.names)
        )
    }

    /// Writes ` = Type` after a `type` item's type as written: its
    /// expansion, the type it stands for.
    fn annotate_type(&self, item: ItemId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.types.declared(item) {
            Some(ty) => write!(f, " = {}", self.table.display(ty, self.names)),
            None => Ok(()),
        }
    }

    /// The read of the definition a def-type reference's path goes on past,
    /// as the check found it ([`Types::reference_past`]).
    fn reference_past(&self, id: ExprId) -> Option<ExprId> {
        self.types.reference_past(id).map(|(read, _)| read)
    }

    /// Writes ` : Type` after the reference to the definition a def-type
    /// reference's path goes on past: its type, with the definition's meta.
    fn annotate_reference(&self, id: ExprId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.types.reference_past(id) {
            Some((_, ty)) => write!(f, " : {}", self.table.display(ty, self.names)),
            None => Ok(()),
        }
    }
}

/// The index of a variable's storage in the evaluator's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slot(u32);

impl Slot {
    /// The absence of a slot, in [`Resolution`] entries for nodes that are
    /// not variables.
    pub const NONE: Slot = Slot(u32::MAX);

    /// The slot `index`: in a workspace, a merged node's cell is the slot
    /// of its index (see [Directory scope](#directory-scope)).
    pub(crate) const fn at(index: u32) -> Slot {
        Slot(index)
    }

    /// The slot numbered `number`. Slots are normally assigned by the
    /// checker; this is for building IR by hand in tests.
    #[cfg(test)]
    pub(crate) const fn new(number: u32) -> Self {
        Self(number)
    }

    /// The slot's number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The slot as an index into a frame.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Where a use of a variable reads its value from. See the module
/// documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VarRef {
    /// A slot of the current function's frame, or of the top level's.
    Local(Slot),
    /// One of the values the current closure captured, by its index in the
    /// function's capture order.
    Capture(u32),
    /// A top-level function of the input, by its slot in the top level's
    /// frame, read from inside a function or `lazy` of the same input:
    /// never captured, read where the top level's frame is when it runs
    /// (see [Top-level functions](#top-level-functions)).
    Fn(Slot),
    /// The closure being called: a function's reference to itself.
    Callee,
    /// TEMPORARY (alias rule): a built-in function whose typing is special, called: only the callee
    /// of a call of it, or of an alias of it, resolves to one, and lowering makes the call the
    /// built-in's own node. Replaced by an ordinary call when it gets a polymorphic type (see the
    /// [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    Builtin(Builtin),
    /// A member of the `fileroot` module, read as `fileroot.NAME` (see
    /// [Imports](#imports)): the field access that reads `this`,
    /// `workspace` or `caller`, and the callee of a call of `helper`, which
    /// lowering makes the member's own node, never reading the module.
    Fileroot(Fileroot),
    /// Nothing: the expression or item is not a variable use.
    None,
}

/// A member of the `fileroot` module, implemented in Rust: where files are
/// (see [Imports](#imports) and [`crate::eval`], "Callers and helpers").
/// The module has no value and no cells: each member is read only as
/// `fileroot.NAME`, through a whole import of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fileroot {
    /// `fileroot.this: String`: the directory of the file the expression is
    /// written in, a constant of the file.
    This,
    /// `fileroot.workspace: String`: the root directory of the workspace, a
    /// constant of the run.
    Workspace,
    /// `fileroot.caller: String`: the directory of the file whose code made
    /// the call the running function is running for, read at runtime.
    Caller,
    /// `fileroot.helper(): {}`: from the point it runs until its function
    /// returns, the calls the function makes pass its own caller on.
    Helper,
}

impl Fileroot {
    /// Every member, in the order of their numbers among a file's scope's
    /// targets (see [`crate::workspace::Target::Fileroot`]).
    pub const ALL: [Fileroot; 4] = [
        Fileroot::This,
        Fileroot::Workspace,
        Fileroot::Caller,
        Fileroot::Helper,
    ];

    /// The name of the module.
    pub const MODULE: &'static str = "fileroot";

    /// Its name, as `fileroot.NAME` reads it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Fileroot::This => "this",
            Fileroot::Workspace => "workspace",
            Fileroot::Caller => "caller",
            Fileroot::Helper => "helper",
        }
    }

    /// The member named `text`, if one is.
    #[must_use]
    pub fn named(text: &str) -> Option<Fileroot> {
        Fileroot::ALL
            .into_iter()
            .find(|member| member.name() == text)
    }

    /// Its index in [`Fileroot::ALL`].
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Fileroot::This => 0,
            Fileroot::Workspace => 1,
            Fileroot::Caller => 2,
            Fileroot::Helper => 3,
        }
    }
}

/// Where a file is, as the driver of its run resolved it, for the `fileroot`
/// module (see [`crate::workspace`], "The standard library"): its directory
/// by its index in the run's table of directories, which the code lowered
/// from it carries ([`crate::ir::Ir::dir`]), and the texts of
/// `fileroot.this` and `fileroot.workspace`, which lowering makes constants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// The index of the file's directory in the run's table of directories.
    pub dir: u32,
    /// The absolute path of the file's directory: `fileroot.this`.
    pub this: Rc<str>,
    /// The absolute path of the workspace's root directory:
    /// `fileroot.workspace`.
    pub workspace: Rc<str>,
}

/// A built-in function: a member of the `builtins` module, implemented in
/// Rust, whose value is a [`crate::Value::Builtin`]. See the module
/// documentation, [Built-in functions](#built-in-functions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// `len(list)`: the number of elements of a list, an `Int`. Its typing
    /// is special.
    Len,
    /// `int(x: Float): Int`: `x` truncated toward zero.
    Int,
    /// `float(x: Int): Float`: the `Float` nearest `x`.
    Float,
    /// `exec(argv: List<String>) -> {status: Int, stderr: String, stdout:
    /// String}`: runs the program `argv[0]` with the arguments `argv[1..]`
    /// and reports what it did (see [`crate::eval`]). Impure.
    Exec,
    /// `panic(message: String): Never`: ends the run with the runtime error
    /// `message`, at the call.
    Panic,
    /// `split(s: String, sep: String): List<String>`: the pieces of `s`
    /// between occurrences of `sep`, in order, as Rust's `str::split` gives
    /// them; an empty `sep` is a runtime error.
    Split,
    /// `join(parts: List<String>, sep: String): String`: the parts with
    /// `sep` between each pair.
    Join,
    /// `starts_with(s: String, prefix: String): Bool`: whether `s` begins
    /// with the bytes of `prefix`.
    StartsWith,
    /// `ends_with(s: String, suffix: String): Bool`: whether `s` ends with
    /// the bytes of `suffix`.
    EndsWith,
    /// `trim(s: String): String`: `s` without its leading and trailing
    /// Unicode whitespace.
    Trim,
    /// `strip_prefix(s: String, prefix: String): String`: `s` without the
    /// bytes of `prefix` if it begins with them, else `s` unchanged.
    StripPrefix,
    /// `strip_suffix(s: String, suffix: String): String`: `s` without the
    /// bytes of `suffix` if it ends with them, else `s` unchanged.
    StripSuffix,
    /// `lines(s: String): List<String>`: the lines of `s`, as Rust's
    /// `str::lines` gives them.
    Lines,
    /// `fields(s: String): List<String>`: the pieces of `s` between runs of
    /// Unicode whitespace, as Rust's `str::split_whitespace` gives them.
    Fields,
    /// `compare_versions(a: String, b: String): Int`: `-1`, `0` or `1` as
    /// `a` sorts before, equal to or after `b` in version order.
    CompareVersions,
    /// `fs_exists(p: String) -> Bool`, `fs.exists`: whether something is at
    /// the absolute path `p`, symbolic links followed. Impure.
    FsExists,
    /// `fs_read(p: String) -> String`, `fs.read`: the contents of the file
    /// at the absolute path `p`, which must be UTF-8 text. Impure.
    FsRead,
    /// `fs_mkdir(p: String) -> {}`, `fs.mkdir`: makes the directory at the
    /// absolute path `p` and any missing parents, as `mkdir -p`. Impure.
    FsMkdir,
    /// `fs_copy(from: String, to: String) -> {}`, `fs.copy`: copies the
    /// file at the absolute path `from` to the absolute path `to`. Impure.
    FsCopy,
    /// `fs_temp_dir() -> String`, `fs.temp_dir`: makes a new empty
    /// directory, removed with everything in it when the run that made it
    /// is dropped, and gives its absolute path. Impure.
    FsTempDir,
    /// `fs_write(p: String, contents: String) -> {}`, `fs.write`: writes
    /// `contents` to the file at the absolute path `p`, made or replaced
    /// whole, atomically, in a directory that must exist. Impure.
    FsWrite,
    /// `sort_versions(vs: List<String>): List<String>`: `vs` sorted in
    /// version order, as `compare_versions` orders two, a stable sort.
    SortVersions,
    /// `show(value)`: the GCSL source of a value, as text, which reads back
    /// as an equal value. Its typing is special.
    Show,
    /// `exec_live(argv: List<String>) -> {status: Int, stderr: String,
    /// stdout: String}`: `exec`, but the program's standard output and
    /// standard error are also copied, line by line as they arrive, to this
    /// process's standard error (see [`crate::eval`]). Impure.
    ExecLive,
}

/// How a built-in function is typed (see [Built-in
/// functions](#built-in-functions)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typing {
    /// By a function type the type table can hold, of any parameters, named
    /// as [`Builtin::params`] says, any result and any effect, which
    /// [`Builtin::ty`] interns, the table in hand: its value is a value of
    /// that type, and a call of it is a call like any other.
    Ordinary,
    /// TEMPORARY (alias rule): call by call, by the checker's rule for it, since no function type
    /// the table can hold is its type (`len`: a `List<T>` for any `T` to `Int`; `show`: any `T`
    /// without a function in it to `String`); its value has an opaque type of its own,
    /// [`TyKind::Builtin`]. To be replaced by a real polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    Special,
}

impl Builtin {
    /// Every built-in function, in the order of the `builtins` module, which
    /// is also the order of their cells, the first slots of a workspace's
    /// frame (see [`crate::workspace`]).
    pub const ALL: [Builtin; 24] = [
        Builtin::Len,
        Builtin::Int,
        Builtin::Float,
        Builtin::Exec,
        Builtin::Panic,
        Builtin::Split,
        Builtin::Join,
        Builtin::StartsWith,
        Builtin::EndsWith,
        Builtin::Trim,
        Builtin::StripPrefix,
        Builtin::StripSuffix,
        Builtin::Lines,
        Builtin::Fields,
        Builtin::CompareVersions,
        Builtin::FsExists,
        Builtin::FsRead,
        Builtin::FsMkdir,
        Builtin::FsCopy,
        Builtin::FsTempDir,
        Builtin::FsWrite,
        Builtin::SortVersions,
        Builtin::Show,
        Builtin::ExecLive,
    ];

    /// The most parameters any built-in function has: what a call of one
    /// collects its arguments into (see [`crate::eval`]).
    pub const MAX_PARAMS: usize = 2;

    /// The names of the fields of what `exec` and `exec_live` return, in the
    /// order of the record each builds: its exit status, its standard output
    /// and its standard error.
    pub const EXEC_FIELDS: [&'static str; 3] = ["status", "stdout", "stderr"];

    /// The name of its member of the `builtins` module.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Builtin::Len => "len",
            Builtin::Int => "int",
            Builtin::Float => "float",
            Builtin::Exec => "exec",
            Builtin::Panic => "panic",
            Builtin::Split => "split",
            Builtin::Join => "join",
            Builtin::StartsWith => "starts_with",
            Builtin::EndsWith => "ends_with",
            Builtin::Trim => "trim",
            Builtin::StripPrefix => "strip_prefix",
            Builtin::StripSuffix => "strip_suffix",
            Builtin::Lines => "lines",
            Builtin::Fields => "fields",
            Builtin::CompareVersions => "compare_versions",
            Builtin::FsExists => "fs_exists",
            Builtin::FsRead => "fs_read",
            Builtin::FsMkdir => "fs_mkdir",
            Builtin::FsCopy => "fs_copy",
            Builtin::FsTempDir => "fs_temp_dir",
            Builtin::FsWrite => "fs_write",
            Builtin::SortVersions => "sort_versions",
            Builtin::Show => "show",
            Builtin::ExecLive => "exec_live",
        }
    }

    /// The name of the member of the standard `fs` module that is it,
    /// `fs.read`, for one of the file system's, by which its runtime errors
    /// name it whatever name it is called by; `None` for every other
    /// built-in function.
    #[must_use]
    pub const fn fs_name(self) -> Option<&'static str> {
        match self {
            Builtin::FsExists => Some("fs.exists"),
            Builtin::FsRead => Some("fs.read"),
            Builtin::FsMkdir => Some("fs.mkdir"),
            Builtin::FsCopy => Some("fs.copy"),
            Builtin::FsTempDir => Some("fs.temp_dir"),
            Builtin::FsWrite => Some("fs.write"),
            Builtin::Len
            | Builtin::Int
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
            | Builtin::SortVersions
            | Builtin::Show
            | Builtin::ExecLive => None,
        }
    }

    /// The names of its parameters, in order, which named arguments match.
    /// None has more than [`Builtin::MAX_PARAMS`].
    #[must_use]
    pub const fn params(self) -> &'static [&'static str] {
        match self {
            Builtin::Len => &["list"],
            Builtin::Int | Builtin::Float => &["x"],
            Builtin::Exec | Builtin::ExecLive => &["argv"],
            Builtin::Panic => &["message"],
            Builtin::Split => &["s", "sep"],
            Builtin::Join => &["parts", "sep"],
            Builtin::StartsWith | Builtin::StripPrefix => &["s", "prefix"],
            Builtin::EndsWith | Builtin::StripSuffix => &["s", "suffix"],
            Builtin::Trim | Builtin::Lines | Builtin::Fields => &["s"],
            Builtin::CompareVersions => &["a", "b"],
            Builtin::FsExists | Builtin::FsRead | Builtin::FsMkdir => &["p"],
            Builtin::FsCopy => &["from", "to"],
            Builtin::FsTempDir => &[],
            Builtin::FsWrite => &["p", "contents"],
            Builtin::SortVersions => &["vs"],
            Builtin::Show => &["value"],
        }
    }

    /// What it may do: only a pure one may be called from a pure function.
    /// `exec` and `exec_live`, which run a program, are impure, and so is
    /// each of the file
    /// system's, `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`,
    /// `fs_temp_dir` and `fs_write`, which touch the world; every other
    /// built-in function is pure, `panic` included, which ends the run as a
    /// division by zero does, `split`, whose empty separator does the same,
    /// and `show`, which fails so on a `Float` with no literal.
    #[must_use]
    pub const fn effect(self) -> Effect {
        match self {
            Builtin::Exec
            | Builtin::ExecLive
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsTempDir
            | Builtin::FsWrite => Effect::Impure,
            Builtin::Len
            | Builtin::Int
            | Builtin::Float
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
            | Builtin::SortVersions
            | Builtin::Show => Effect::Pure,
        }
    }

    /// How it is typed.
    #[must_use]
    pub const fn typing(self) -> Typing {
        match self {
            Builtin::Len | Builtin::Show => Typing::Special,
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
            | Builtin::ExecLive => Typing::Ordinary,
        }
    }

    /// How its value is shown: as its type, `fn(x: Float): Int`, for an
    /// ordinary one, as any function is, and as the type table shows that
    /// type (a record's fields by name, `panic`'s result `Never`, which no
    /// annotation can write); as `builtin len`, `builtin show`, for a
    /// special one.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Builtin::Len => "builtin len",
            Builtin::Show => "builtin show",
            Builtin::Int => "fn(x: Float): Int",
            Builtin::Float => "fn(x: Int): Float",
            Builtin::Exec | Builtin::ExecLive => {
                "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}"
            }
            Builtin::Panic => "fn(message: String): Never",
            Builtin::Split => "fn(s: String, sep: String): List<String>",
            Builtin::Join => "fn(parts: List<String>, sep: String): String",
            Builtin::StartsWith => "fn(s: String, prefix: String): Bool",
            Builtin::EndsWith => "fn(s: String, suffix: String): Bool",
            Builtin::Trim => "fn(s: String): String",
            Builtin::StripPrefix => "fn(s: String, prefix: String): String",
            Builtin::StripSuffix => "fn(s: String, suffix: String): String",
            Builtin::Lines | Builtin::Fields => "fn(s: String): List<String>",
            Builtin::CompareVersions => "fn(a: String, b: String): Int",
            Builtin::FsExists => "fn(p: String) -> Bool",
            Builtin::FsRead => "fn(p: String) -> String",
            Builtin::FsMkdir => "fn(p: String) -> {}",
            Builtin::FsCopy => "fn(from: String, to: String) -> {}",
            Builtin::FsTempDir => "fn() -> String",
            Builtin::FsWrite => "fn(p: String, contents: String) -> {}",
            Builtin::SortVersions => "fn(vs: List<String>): List<String>",
        }
    }

    /// The type of its value, interned into `types`, the names of its
    /// parameters, and of the fields of its result, into `names`: for an
    /// ordinary one its function type, its parameters' types and its result
    /// built here, with its [`Builtin::effect`]; the opaque type of a
    /// special one.
    pub(crate) fn ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        // What fills the entries of `param_types` past the built-in's
        // parameters, which the `zip` with `params` below never reads.
        const UNUSED: TyId = TyId::ERROR;
        // Each parameter's type, in the order of `params`, and the result's.
        let (param_types, result): ([TyId; Builtin::MAX_PARAMS], TyId) = match self {
            Builtin::Int => ([TyId::FLOAT, UNUSED], TyId::INT),
            Builtin::Float => ([TyId::INT, UNUSED], TyId::FLOAT),
            Builtin::Exec | Builtin::ExecLive => {
                let argv = types.list(TyId::STRING);
                let [status, stdout, stderr] = Builtin::EXEC_FIELDS.map(|text| names.intern(text));
                let result = types.record([
                    (status, TyId::INT),
                    (stdout, TyId::STRING),
                    (stderr, TyId::STRING),
                ]);
                ([argv, UNUSED], result)
            }
            Builtin::Panic => ([TyId::STRING, UNUSED], types.never()),
            Builtin::Split => ([TyId::STRING, TyId::STRING], types.list(TyId::STRING)),
            Builtin::Join => ([types.list(TyId::STRING), TyId::STRING], TyId::STRING),
            Builtin::StartsWith | Builtin::EndsWith => ([TyId::STRING, TyId::STRING], TyId::BOOL),
            Builtin::Trim | Builtin::FsRead => ([TyId::STRING, UNUSED], TyId::STRING),
            Builtin::StripPrefix | Builtin::StripSuffix => {
                ([TyId::STRING, TyId::STRING], TyId::STRING)
            }
            Builtin::Lines | Builtin::Fields => ([TyId::STRING, UNUSED], types.list(TyId::STRING)),
            Builtin::CompareVersions => ([TyId::STRING, TyId::STRING], TyId::INT),
            Builtin::FsExists => ([TyId::STRING, UNUSED], TyId::BOOL),
            Builtin::FsMkdir => ([TyId::STRING, UNUSED], TyId::EMPTY),
            Builtin::FsCopy | Builtin::FsWrite => ([TyId::STRING, TyId::STRING], TyId::EMPTY),
            Builtin::FsTempDir => ([UNUSED, UNUSED], TyId::STRING),
            Builtin::SortVersions => {
                let list = types.list(TyId::STRING);
                ([list, UNUSED], list)
            }
            // TEMPORARY (alias rule): the opaque type, until a polymorphic one (see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
            Builtin::Len | Builtin::Show => return types.builtin(self),
        };
        let params = self
            .params()
            .iter()
            .zip(param_types)
            .map(|(&text, ty)| (Some(names.intern(text)), ty));
        types.fn_type(params, result, self.effect())
    }
}

/// Whether and how an expression's value is awaited where it is used, in
/// [`Resolution::await_of`]. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AwaitMark {
    /// Not awaited: the value is used as it is.
    None,
    /// Awaited fully at the top, through every level of future.
    Top,
    /// Awaited fully at the top and then in every lazy field of the record
    /// it is, at any depth.
    Deep,
}

impl From<AwaitKind> for AwaitMark {
    fn from(kind: AwaitKind) -> Self {
        match kind {
            AwaitKind::Top => AwaitMark::Top,
            AwaitKind::Deep => AwaitMark::Deep,
        }
    }
}

/// What the checker learns about one function expression or `lazy`, in
/// [`Resolution::function`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FnResolution {
    /// The number of slots the body's frame needs, its parameters included.
    pub frame_size: u32,
    /// The number of parameters, which take slots `0..param_count`.
    pub param_count: u32,
    /// What the function captures, in capture order: where each value is
    /// read from in the frame that creates the closure, with its type. Its
    /// closure keeps, after these, the closures of its default functions
    /// (see [`FnResolution::defaults`]).
    pub captures: Vec<(VarRef, TyId)>,
    /// The name the function sees itself by, if it is the direct
    /// initialiser of a single-name `let`, field or named function item.
    pub callee: Option<Name>,
    /// For each slot of the body's frame, the name of the last variable
    /// assigned to it. Only for showing slots to people.
    pub slot_names: Vec<Name>,
    /// For each parameter, in order, where the body, or a default, first
    /// measures it, in source order, if it is a pure function's bare
    /// parameter that one measures, which makes it present; `None` for every other
    /// parameter. Empty for a `lazy`. See the module documentation.
    pub param_demands: Vec<Option<Span>>,
    /// For each parameter, in order, if it is a callback parameter of an
    /// impure or memoised function whose body was checked without an
    /// error, and was not narrowed at some narrowable position, what kept
    /// it from being narrowed; `None` for every other parameter. Empty if
    /// no parameter has an entry, as for a `lazy`. See the module
    /// documentation.
    pub unnarrowed: Vec<Option<Unnarrowed>>,
    /// For each parameter, in order, its default once checked, a constant or
    /// its default function's place among the closure's captures, if it
    /// declares one (see [Calls](#calls)); `None` for every other
    /// parameter, and one whose default is in error. Empty if no parameter
    /// declares one, as for a `lazy`.
    pub defaults: Vec<Option<ParamDefault>>,
}

/// A callback parameter that was not narrowed at some of its narrowable
/// positions, in [`FnResolution::unnarrowed`]: what the call-site label on
/// a mismatched argument needs. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unnarrowed {
    /// The parameter's type had every narrowable position been narrowed.
    pub narrowed: TyId,
    /// For each parameter of the parameter's function type, in order, what
    /// first kept it from being narrowed, in source order; `None` for one
    /// that was narrowed or is not narrowable.
    pub blocks: Box<[Option<NarrowingBlock>]>,
}

/// What kept a narrowable position of a callback parameter from being
/// narrowed. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NarrowingBlock {
    /// A call of the callback gives the position a value that may be a
    /// future: the argument.
    Given(Span),
    /// The callback is used as a value, other than as the callee of a
    /// call: the use of its name.
    Used(Span),
}

impl NarrowingBlock {
    /// Where it is.
    #[must_use]
    pub const fn span(self) -> Span {
        match self {
            NarrowingBlock::Given(span) | NarrowingBlock::Used(span) => span,
        }
    }
}

/// A parameter's constant default, as its `= DEFAULT` is once checked
/// against the parameter's type (see [Calls](#calls)), which a call that
/// leaves the parameter unfilled is given, as [`ParamDefault::Constant`] in
/// [`FnResolution::defaults`] and [`Resolution::call_defaults`]. It holds no id of a tree, since a call of a
/// `def` of another file, or of a function of an earlier session input, is
/// given it where that tree is not. A field's default of a record type is
/// one too, held in the type ([`crate::types::RecordTy::defaults`]), which
/// a record literal that lacks the field is given, in
/// [`Resolution::record_fills`] (see [Field defaults](#field-defaults)).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constant {
    /// An `Int`.
    Int(i64),
    /// A `Duration` or a `Size`, of its smallest unit.
    Quantity(Quantity, i64),
    /// A `Float`, by its bits ([`f64::to_bits`]), so that a constant has
    /// equality: an integer literal typed `Float` included.
    Float(u64),
    /// A `Bool`.
    Bool(bool),
    /// A `String`.
    Str(Box<str>),
    /// A list, of its elements, in order.
    List(Box<[Constant]>),
    /// A record of at least one field, of its fields, in the order they are
    /// written.
    Record(Box<[(Name, Constant)]>),
    /// The empty record, `{}`.
    Empty,
}

/// What a call that leaves a parameter unfilled gives it, its parameter's
/// default once checked (see [Calls](#calls)), in [`FnResolution::defaults`]
/// and [`Resolution::call_defaults`]: a constant, copied in where the call
/// is, or the default function of a default that is no constant, which the
/// call calls on its side, with the parameters before it. Neither holds an
/// id of a tree, since a call of a `def` of another file, or of a function
/// of an earlier session input, is given it where that tree is not: the
/// default function is found in the callee's closure, by the index among its
/// captures that the closure keeps it at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ParamDefault {
    /// A constant default.
    Constant(Constant),
    /// A default that is no constant: its default function's closure is the
    /// callee closure's capture number `capture`.
    Function {
        /// The index of the default function's closure among the callee
        /// closure's captures, after the function's own.
        capture: u32,
    },
}

/// What the check gives lowering besides types and slots, as
/// [`Resolution`] carries it.
#[derive(Debug, Default)]
struct ForLowering {
    /// Each call that gives a parameter its default, by its id: see
    /// [`Resolution::call_defaults`].
    call_defaults: HashMap<ExprId, Box<[(u32, ParamDefault)]>>,
    /// Each function item's attributes: see [`Resolution::attributes`].
    fn_attributes: HashMap<FnId, Box<[Attribute]>>,
    /// Each record literal given fields' defaults, by its id: see
    /// [`Resolution::record_fills`].
    record_fills: RecordFills,
}

/// One attribute of a function item, resolved (see [Attributes](#attributes)):
/// what a host knows it by, and its arguments, every field of its record
/// type by its name, in the type's order, each the constant given or its
/// default. A constant record among them names its fields by the program's
/// names, as every [`Constant`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// What a host knows it by.
    pub id: AttributeId,
    /// Its arguments, by field.
    pub args: Box<[(Box<str>, Constant)]>,
}

/// The defaults of a function, for [`FnResolution::defaults`]: `constants`,
/// each parameter's constant default, if it has one that checked, and
/// `functions`, each parameter with a default function, in parameter order,
/// with whether it checked, whose closures follow the function's
/// `captures` own captures in its closure (see [Calls](#calls)). Empty if no
/// parameter has a default.
fn merged_defaults(
    constants: Vec<Option<Constant>>,
    functions: &[(usize, bool)],
    captures: usize,
) -> Vec<Option<ParamDefault>> {
    let count = functions
        .iter()
        .map(|&(param, _)| param + 1)
        .max()
        .unwrap_or(0)
        .max(constants.len());
    if count == 0 {
        return Vec::new();
    }
    let mut merged: Vec<Option<ParamDefault>> = constants
        .into_iter()
        .map(|constant| constant.map(ParamDefault::Constant))
        .collect();
    merged.resize(count, None);
    for (at, &(param, checked)) in functions.iter().enumerate() {
        if let (true, Some(entry)) = (checked, merged.get_mut(param)) {
            let capture = u32::try_from(captures + at).unwrap_or(u32::MAX);
            *entry = Some(ParamDefault::Function { capture });
        }
    }
    merged
}

/// Each record literal given fields' defaults, by its id, with each such
/// field and its default: see [`Resolution::record_fills`].
type RecordFills = HashMap<ExprId, Box<[(Name, Constant)]>>;

/// What a call of a function whose tree is gone needs to know of its
/// parameters: a function declared by an earlier session input, through
/// the binding its scope carries ([`Scope`]), or a `def` of another file of
/// a workspace, through its cell (see [Imports](#imports)). For each
/// parameter, in order, whether its body measures it, which makes it
/// present, for the call-site form of the implicit-await error, and its
/// default, if it has one, which a call that leaves it unfilled is given;
/// and its attributes, which a def-type gives its definitions (see
/// [Attributes](#attributes)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KnownParams {
    /// For each parameter, whether the body measures it.
    measured: Box<[bool]>,
    /// For each parameter, its default, if it has one; empty if none has.
    defaults: Box<[Option<ParamDefault>]>,
    /// The function's attributes, in source order.
    attributes: Box<[Attribute]>,
}

impl KnownParams {
    /// What a call of the function `resolution` describes, whose attributes
    /// are `attributes`, needs to know of its parameters.
    pub(crate) fn of(resolution: &FnResolution, attributes: &[Attribute]) -> Rc<Self> {
        Rc::new(Self {
            measured: resolution
                .param_demands
                .iter()
                .map(Option::is_some)
                .collect(),
            defaults: resolution.defaults.iter().cloned().collect(),
            attributes: attributes.into(),
        })
    }

    /// Whether the body measures parameter number `param`.
    fn measured(&self, param: usize) -> bool {
        self.measured.get(param).copied().unwrap_or(false)
    }

    /// The default of parameter number `param`, if it has one.
    fn default(&self, param: usize) -> Option<&ParamDefault> {
        self.defaults.get(param).and_then(Option::as_ref)
    }
}

/// How the path of a dotted `let`, field or `:=` resolves: the record, or
/// for a `:=` with an index segment the list, it updates, how far into its
/// path that value's static type reaches, and which of the records and
/// lists along the way are awaited. See the module documentation.
///
/// For `let a.b.c = e`, with `a` in scope as `{b: {c: Int}}`, the source is
/// where `a` is read from and all of `a` and `a.b` exist, so `existing` is
/// 2; with `a` as `{x: Int}`, only `a` exists, and `existing` is 1; with no
/// `a`, there is no source and `existing` is 0. A dotted field's first
/// definition has no source; each later one has the field's own slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathResolution {
    /// Where the record the item updates is read from, as a use of its
    /// variable would read it (a slot, or a captured value), or
    /// [`VarRef::None`] if it builds its record from nothing.
    pub source: VarRef,
    /// How many of the path's leading segments name records, or lists, that
    /// exist in the static type of the updated value: 0 without a source,
    /// and otherwise 1 for the source itself and one more for each
    /// following segment, short of the last, that the type has as a record
    /// or a list (every one, for a `:=`). Every segment after them, the last
    /// excepted, is created as a record.
    pub existing: u32,
    /// Where the item's `existing` entries start in the resolution's table
    /// of awaited records (see [`Resolution::path_awaits`]), or
    /// [`PathResolution::NOT_AWAITED`] if none of its records is awaited,
    /// which is the case of every item but a dotted `let` or `:=` of a
    /// record that may be a future.
    pub awaits: u32,
}

impl PathResolution {
    /// The `awaits` of an item none of whose records is awaited.
    pub const NOT_AWAITED: u32 = u32::MAX;

    /// The resolution of an item that is not a dotted declaration, and of
    /// one that builds its record from nothing.
    pub const NONE: PathResolution = PathResolution::unawaited(VarRef::None, 0);

    /// The resolution of an item that updates the record read from
    /// `source`, `existing` of whose path's segments exist, none of them
    /// awaited.
    const fn unawaited(source: VarRef, existing: u32) -> Self {
        Self {
            source,
            existing,
            awaits: PathResolution::NOT_AWAITED,
        }
    }
}

/// What the checker decided for one `is` or `as` expression, in
/// [`Resolution::test_of`]. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResolution {
    /// The plan of the test the expression runs at runtime, against the
    /// value of its operand once awaited as [`Resolution::await_of`] says.
    pub test: ValueTest,
    /// For an `is` that narrows the branches of the `if` it is the
    /// condition of, the slot the value it tests is stored in, which the
    /// branches' narrowed bindings read; [`Slot::NONE`] for every other.
    pub slot: Slot,
    /// For an `as`, its operand as its failure names it: the operand as
    /// written in backticks if it is a name or a path, as in "`x`", and
    /// "this value" otherwise.
    pub subject: Box<str>,
    /// For an `as`, the type it tests for as its failure names it, with its
    /// article, as in "an Int" or "a `List<Int>`".
    pub target: Box<str>,
}

/// For each node of a workspace file's tree, the children that other
/// files declare under it, each with its name, cell and type (see
/// [Directory scope](#directory-scope)).
type SharedChildren = Vec<Vec<(Name, Slot, TyId)>>;

/// The records a file of a workspace reads whole through its imports: each
/// one's slot and fields (see [`Resolution::imported_records`]).
type ImportedRecords = Vec<ImportedRecord>;

/// A record a file of a workspace reads whole through an import: its
/// private slot, and the name, cell and type of each of its fields.
pub type ImportedRecord = (Slot, Vec<(Name, Slot, TyId)>);

/// A record a file of a workspace assembles of a node of its view with a
/// leaf of the file under it, as a node of its schedule: its private slot
/// ([`Slot::NONE`] if the file never reads it whole), the position of the
/// first item of the file under it, and the name, cell and type of each of
/// its fields, in the view's order.
pub type ViewRecord = (Slot, usize, Vec<(Name, Slot, TyId)>);

/// Where every variable of an [`Ast`] is stored, what every function's
/// frame and captures are, and which parameter every argument fills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// Where each `Name` expression reads, indexed by [`ExprId`];
    /// [`VarRef::None`] for every other expression.
    expr_vars: Vec<VarRef>,
    /// Whether and how each expression is awaited where it is used,
    /// indexed by [`ExprId`].
    awaits: Vec<AwaitMark>,
    /// The slot each `Let`, `Field` and `Rebind` item writes, indexed by
    /// [`ItemId`], and for a `Spread` item the slot its operand's value is
    /// held in while its fields are read; [`Slot::NONE`] for every other
    /// item. Every definition of an open field writes the field's one slot,
    /// and so does every definition of a field that overrides a soft one.
    item_slots: Vec<Slot>,
    /// Where the slots of each `Spread` item's fields are in
    /// `spread_slots`, indexed by [`ItemId`]: their start and their number;
    /// `(0, 0)` for every other item.
    item_spreads: Vec<(u32, u32)>,
    /// The slot each field of each spread's operand is stored in, in the
    /// order of the fields of the operand's static type, each spread's
    /// together.
    spread_slots: Vec<Slot>,
    /// How the path of each dotted `Let`, `Field` and `Rebind` item
    /// resolves, indexed by [`ItemId`]; [`PathResolution::NONE`] for every
    /// other item, and for one that builds its record from nothing.
    item_paths: Vec<PathResolution>,
    /// For each dotted `let` and `:=` item of which some record along its
    /// path is awaited, whether each of its existing records is, from its
    /// source down, where its [`PathResolution::awaits`] says. Items with no
    /// awaited record have no entries.
    path_awaits: Vec<bool>,
    /// Each top-level function of the input, by its item, in source order,
    /// with where its closure is made.
    top_functions: Vec<(ItemId, MadeAt)>,
    /// The cell of each node of the input's tree of top-level fields
    /// ([`Mentions::tree`]), by its index there: the slot of the top level's
    /// frame that holds its value. A root's is its binding's slot, a `def`'s
    /// its function's; the others are slots no name finds.
    field_cells: Vec<Slot>,
    /// For a file of a workspace, the private slot of the file's own record
    /// of each node it shares with another file, by its index in the tree;
    /// [`Slot::NONE`] for any other node, and empty outside a workspace
    /// (see [Directory scope](#directory-scope)).
    own_records: Vec<Slot>,
    /// For a file of a workspace, the children of each node of its tree
    /// that other files declare, with their cells and types: what the
    /// node's assembly reads besides its own children. Empty outside a
    /// workspace.
    shared_children: SharedChildren,
    /// For a file of a workspace, the records it reads whole through its
    /// imports (a directory's, a file's own, a file's part of a shared
    /// node), each with the private slot it is assembled into as the file
    /// starts and the name, cell and type of each field, nested records
    /// before the records that hold them (see [Imports](#imports)).
    imported_records: ImportedRecords,
    /// For a file of a workspace, the record of each node of its view with
    /// a leaf of the file under it, by its index among them ([`Mentions::views`]):
    /// stored at its point in the schedule, after its own children, if the
    /// file reads it whole (see [Imports](#imports)).
    view_records: Vec<ViewRecord>,
    /// For a value file of a workspace that another file imports, the cell
    /// its result is stored in as its block ends; [`Slot::NONE`] otherwise.
    value_cell: Slot,
    /// Where the input is, as its run's driver resolved it: what its code
    /// carries and what `fileroot.this` and `fileroot.workspace` are in it;
    /// the default, directory 0 and empty texts, outside a workspace's
    /// context, where nothing can import `fileroot`.
    place: Place,
    /// The tree of the input's top-level fields and when each node is
    /// computed (see [Top-level fields](#top-level-fields)).
    field_schedule: FieldSchedule,
    /// The number of slots the top level's frame needs.
    frame_size: u32,
    /// For each slot of the top level's frame, the name of the last
    /// variable assigned to it.
    slot_names: Vec<Name>,
    /// What the checker found for each function expression, indexed by
    /// [`FnId`].
    functions: Vec<FnResolution>,
    /// The index of the parameter each argument fills, indexed like the
    /// [`Ast`]'s argument pool; `u32::MAX` for an argument of an erroneous
    /// call.
    arg_params: Vec<u32>,
    /// The slot each `for` expression's variable is stored in, indexed by
    /// [`ForId`].
    for_slots: Vec<Slot>,
    /// What each `break`, `continue` and `return` item is for, indexed by
    /// [`ItemId`]; `None` for every other item, and for one that is in
    /// error.
    jumps: Vec<Option<JumpTarget>>,
    /// What was found of each loop, indexed by [`ForId`].
    loops: Vec<LoopResolution>,
    /// The labelled blocks that some `return` leaves, by their ids.
    returned_blocks: HashSet<ExprId>,
    /// The statements whose values are discarded, by their ids: calls of a
    /// function that is not pure, outside a pure function.
    discarded: HashSet<ExprId>,
    /// What was decided for each `is` and `as` expression, by its id; only
    /// a program that has one has entries.
    tests: HashMap<ExprId, TestResolution>,
    /// Each field access that reads a function's meta, `func.versions`, by
    /// its id, with the meta's type and how the meta is awaited before its
    /// field is read (see [Metas](#metas)); only a program that reads one
    /// has entries.
    meta_reads: HashMap<ExprId, (TyId, AwaitMark)>,
    /// Each definition made with a def-type that declares a meta, by its
    /// function, with the cell of the top level's frame that holds its
    /// meta's closure (see [Metas](#metas)).
    metas: Vec<(FnId, Slot)>,
    /// Each call that gives a parameter its default, by its id, with each
    /// such parameter and its default, in parameter order (see
    /// [Calls](#calls)); only a program with one has entries.
    call_defaults: HashMap<ExprId, Box<[(u32, ParamDefault)]>>,
    /// The attributes of each function item that has any, by its function,
    /// in source order (see [Attributes](#attributes)).
    fn_attributes: HashMap<FnId, Box<[Attribute]>>,
    /// Each record literal, a record block or `{}`, that the checker gave
    /// fields their defaults, by its id, with each such field and its
    /// default, in the order of
    /// their names' text (see [Field defaults](#field-defaults)); only a
    /// program with one has entries.
    record_fills: RecordFills,
}

/// What a `break`, `continue` or `return` item is for (see
/// [`Resolution::target`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JumpTarget {
    /// A loop: the one a `break` or `continue` is for, or the one a
    /// `return'L` names.
    Loop(ForId),
    /// The labelled block, by its expression, that a `return'L` names.
    Block(ExprId),
    /// The innermost function or `lazy` around an unlabelled `return`.
    Function,
}

/// What the checker found of a loop, for lowering: see
/// [`Resolution::loop_of`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoopResolution {
    /// Whether the loop cannot end: a `for BLOCK` that no `break` is for,
    /// whose ordinary type is `Never`, and which builds no list.
    pub endless: bool,
    /// Whether some `return'L` leaves it, so that it evaluates to the
    /// value returned.
    pub returned: bool,
}

impl Resolution {
    /// Where expression `id` reads its value from, or [`VarRef::None`] if it
    /// is not a variable use.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn expr_var(&self, id: ExprId) -> VarRef {
        self.expr_vars[id.index()]
    }

    /// Whether and how expression `id` is awaited where it is used:
    /// [`AwaitMark::None`] if it is used as it is. An awaited expression's
    /// type in [`Types`] is its type before the await.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn await_of(&self, id: ExprId) -> AwaitMark {
        self.awaits[id.index()]
    }

    /// What the checker found for function expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn function(&self, id: FnId) -> &FnResolution {
        &self.functions[id.index()]
    }

    /// The parameter each argument of `args`, a call's arguments, fills,
    /// in the order of [`Ast::args`]; `u32::MAX` for an argument of an
    /// erroneous call.
    ///
    /// # Panics
    ///
    /// Panics if `args` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn arg_params(&self, args: ArgRange) -> &[u32] {
        &self.arg_params[args.start()..args.start() + args.count()]
    }

    /// The parameters that the call `id` gives their defaults, each with
    /// its default, in parameter order: those it leaves unfilled that have
    /// one, if its callee is known (see [Calls](#calls)); none for any
    /// other call, and for an expression that is no call.
    #[must_use]
    pub fn call_defaults(&self, id: ExprId) -> &[(u32, ParamDefault)] {
        self.call_defaults.get(&id).map_or(&[], |defaults| defaults)
    }

    /// The attributes of function `function`, in source order: none for a
    /// function that is no function item, or has none (see
    /// [Attributes](#attributes)).
    #[must_use]
    pub fn attributes(&self, function: FnId) -> &[Attribute] {
        self.fn_attributes
            .get(&function)
            .map_or(&[], |attributes| attributes)
    }

    /// The fields that the record literal `block`, a record block or `{}`,
    /// lacks and is given, each with its default, in the order of their
    /// names' text: the defaulted fields of the record type it is written
    /// where it is expected (see [Field defaults](#field-defaults)); none
    /// for any other block, and for an expression that is no block.
    #[must_use]
    pub fn record_fills(&self, block: ExprId) -> &[(Name, Constant)] {
        self.record_fills.get(&block).map_or(&[], |fills| fills)
    }

    /// The slot written by item `id`, or [`Slot::NONE`] if it is not a
    /// declaration. For a spread item, the slot its operand's value is held
    /// in while its fields are read, which no binding owns (see
    /// [`Resolution::spread_slots`] for its fields').
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn item_slot(&self, id: ItemId) -> Slot {
        self.item_slots[id.index()]
    }

    /// Each top-level function of the input, by its item, in source order,
    /// with where its closure is made: what lowering stores in each one's
    /// slot, and when (see [When a function may be
    /// used](#when-a-function-may-be-used)).
    #[must_use]
    pub fn top_functions(&self) -> &[(ItemId, MadeAt)] {
        &self.top_functions
    }

    /// For a file of a workspace, the private slot of the file's own record
    /// of each node of its tree it shares with another file, by the node's
    /// index; [`Slot::NONE`] for the others.
    #[must_use]
    pub fn own_records(&self) -> &[Slot] {
        &self.own_records
    }

    /// For a file of a workspace, the children of the node `node` of its
    /// tree that other files declare, with their cells and types.
    #[must_use]
    pub fn shared_children(&self, node: u32) -> &[(Name, Slot, TyId)] {
        self.shared_children
            .get(node as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// The cell of each node of the input's tree of top-level fields, by
    /// its index in [`FieldSchedule::tree`]: the slot of the top level's
    /// frame its value is stored in, once, at its point.
    #[must_use]
    pub fn field_cells(&self) -> &[Slot] {
        &self.field_cells
    }

    /// The records the input, a file of a workspace, reads whole through
    /// its imports, assembled as it starts: each one's private slot and the
    /// name, cell and type of each of its fields, nested records first (see
    /// [Imports](#imports)).
    #[must_use]
    pub fn imported_records(&self) -> &[ImportedRecord] {
        &self.imported_records
    }

    /// For a file of a workspace, the record of the node of its view with a
    /// leaf of the file under it numbered `node` among the field nodes
    /// (after its tree's), if it is one: stored at the node's point (see
    /// [Imports](#imports)).
    #[must_use]
    pub fn view_record(&self, node: u32) -> Option<&ViewRecord> {
        let at = (node as usize).checked_sub(self.field_schedule.tree.nodes.len())?;
        self.view_records.get(at)
    }

    /// Every record of [`Resolution::view_record`], by the view node's
    /// index among the file's nodes of its view with a leaf of it under
    /// them.
    #[cfg(test)]
    pub(crate) fn view_records(&self) -> &[ViewRecord] {
        &self.view_records
    }

    /// The cell the input's result is stored in as its block ends, if it is
    /// a value file of a workspace that another file imports; else
    /// [`Slot::NONE`].
    #[must_use]
    pub const fn value_cell(&self) -> Slot {
        self.value_cell
    }

    /// Where the input is: the index of its directory in its run's table,
    /// which its code carries, and the texts `fileroot.this` and
    /// `fileroot.workspace` lower to in it.
    #[must_use]
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// Places the input at `place`: what an input checked with no
    /// workspace, which knows no place of its own, is given by a driver that
    /// runs it in one, as the call a workspace makes to invoke a definition
    /// by name (see [`crate::workspace`], "Def-types by name").
    pub(crate) fn set_place(&mut self, place: Place) {
        self.place = place;
    }

    /// The tree of the input's top-level fields and when each node is
    /// computed: what lowering stores in each cell, and before which item.
    #[must_use]
    pub fn field_schedule(&self) -> &FieldSchedule {
        &self.field_schedule
    }

    /// The slot each field of the spread item `id`'s operand is stored in,
    /// in the order of the fields of the operand's static type once
    /// awaited: a new slot for a name that is not yet a field of its block,
    /// and the field's one slot for a soft field it overrides (see the
    /// module documentation). Empty for every other item.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn spread_slots(&self, id: ItemId) -> &[Slot] {
        let (start, count) = self.item_spreads[id.index()];
        let (start, count) = (start as usize, count as usize);
        self.spread_slots
            .get(start..start.saturating_add(count))
            .unwrap_or_default()
    }

    /// The slot the variable of the `for` expression `id` is stored in,
    /// which each element is written to before the guard and the body run.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn for_slot(&self, id: ForId) -> Slot {
        self.for_slots[id.index()]
    }

    /// The loop that the `break` or `continue` item `id` is for: the
    /// innermost loop of its function whose body holds it, or the one its
    /// label names. `None` for any other item, and for an id this table
    /// does not have.
    #[must_use]
    pub fn jump_target(&self, id: ItemId) -> Option<ForId> {
        match self.target(id)? {
            JumpTarget::Loop(loop_id) => Some(loop_id),
            JumpTarget::Block(_) | JumpTarget::Function => None,
        }
    }

    /// What the `break`, `continue` or `return` item `id` is for: a loop,
    /// a labelled block or, for an unlabelled `return`, the innermost
    /// function or `lazy`. `None` for any other item, for one in error,
    /// and for an id this table does not have.
    #[must_use]
    pub fn target(&self, id: ItemId) -> Option<JumpTarget> {
        self.jumps.get(id.index()).copied().flatten()
    }

    /// What was found of the loop `id`: whether it cannot end, and whether
    /// a `return` leaves it. The default, a loop that can end and that no
    /// `return` leaves, for an id this table does not have.
    #[must_use]
    pub fn loop_of(&self, id: ForId) -> LoopResolution {
        self.loops.get(id.index()).copied().unwrap_or_default()
    }

    /// Whether some `return` leaves the labelled block `id`, so that it
    /// evaluates to the value returned.
    #[must_use]
    pub fn is_returned_block(&self, id: ExprId) -> bool {
        self.returned_blocks.contains(&id)
    }

    /// Whether the statement `id` is a call of a function that is not pure
    /// whose value, whatever it is, is discarded once awaited, as a bare
    /// `await`'s is.
    #[must_use]
    pub fn is_discarded(&self, id: ExprId) -> bool {
        self.discarded.contains(&id)
    }

    /// What was decided for the `is` or `as` expression `id`: its test's
    /// plan, and where an `is` that narrows stores the value it tests, or
    /// how an `as` names what it tests. `None` for any other expression.
    #[must_use]
    pub fn test_of(&self, id: ExprId) -> Option<&TestResolution> {
        self.tests.get(&id)
    }

    /// If the field access `id` reads a function's meta (see
    /// [Metas](#metas)): the meta's type, before any await, and how the
    /// meta is awaited before its field is read. `None` for any other
    /// expression.
    #[must_use]
    pub fn meta_read(&self, id: ExprId) -> Option<(TyId, AwaitMark)> {
        self.meta_reads.get(&id).copied()
    }

    /// Each definition made with a def-type that declares a meta, by its
    /// function, with the cell of the top level's frame that holds its
    /// meta's closure, which lowering gives its code (see [Metas](#metas)).
    #[must_use]
    pub fn metas(&self) -> &[(FnId, Slot)] {
        &self.metas
    }

    /// How the path of item `id` resolves: [`PathResolution::NONE`] if it
    /// is not a dotted declaration or builds its record from nothing.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`] this table was built
    /// for.
    #[must_use]
    pub fn item_path(&self, id: ItemId) -> PathResolution {
        self.item_paths[id.index()]
    }

    /// For each of the `path.existing` records along the path that `path`
    /// resolves, from its source down, whether it is awaited before it is
    /// updated, being a value that may be a future: empty if none is (see
    /// the module documentation). `None` if `path` names entries this
    /// resolution does not have.
    #[must_use]
    pub fn path_awaits(&self, path: PathResolution) -> Option<&[bool]> {
        if path.awaits == PathResolution::NOT_AWAITED {
            return Some(&[]);
        }
        let start = path.awaits as usize;
        self.path_awaits
            .get(start..start.checked_add(path.existing as usize)?)
    }

    /// The number of slots the top level's frame needs: one more than the
    /// highest slot used.
    #[must_use]
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// For each slot of the top level's frame, the name of the last
    /// variable assigned to it. Only for showing slots to people, as
    /// `--dump-ir` does.
    #[must_use]
    pub fn slot_names(&self) -> &[Name] {
        &self.slot_names
    }

    /// A resolution for a tree with `exprs` expressions and `items` items and
    /// no variables, functions or calls, for building checked trees by hand
    /// in tests.
    #[cfg(test)]
    pub(crate) fn without_variables(exprs: usize, items: usize) -> Self {
        Self {
            expr_vars: vec![VarRef::None; exprs],
            awaits: vec![AwaitMark::None; exprs],
            item_slots: vec![Slot::NONE; items],
            item_spreads: vec![(0, 0); items],
            spread_slots: Vec::new(),
            item_paths: vec![PathResolution::NONE; items],
            path_awaits: Vec::new(),
            top_functions: Vec::new(),
            field_cells: Vec::new(),
            own_records: Vec::new(),
            shared_children: Vec::new(),
            imported_records: Vec::new(),
            view_records: Vec::new(),
            value_cell: Slot::NONE,
            place: Place::default(),
            field_schedule: FieldSchedule::default(),
            frame_size: 0,
            slot_names: Vec::new(),
            functions: Vec::new(),
            arg_params: Vec::new(),
            for_slots: Vec::new(),
            jumps: vec![None; items],
            loops: Vec::new(),
            returned_blocks: HashSet::new(),
            discarded: HashSet::new(),
            tests: HashMap::new(),
            meta_reads: HashMap::new(),
            metas: Vec::new(),
            call_defaults: HashMap::new(),
            fn_attributes: HashMap::new(),
            record_fills: HashMap::new(),
        }
    }

    /// Forgets the parameter of every argument, as for an erroneous call,
    /// for testing what lowering makes of that.
    #[cfg(test)]
    pub(crate) fn unbind_arguments(&mut self) {
        self.arg_params.fill(u32::MAX);
    }

    /// Forgets the slot of every `for`'s variable, for testing what
    /// lowering makes of that.
    #[cfg(test)]
    pub(crate) fn forget_for_slots(&mut self) {
        self.for_slots.fill(Slot::NONE);
    }

    /// Number of expression entries in both expression tables, equal to
    /// the number of expressions in the [`Ast`].
    pub(crate) fn expr_len(&self) -> usize {
        self.expr_vars.len().min(self.awaits.len())
    }

    /// Number of function entries, equal to the number of function
    /// expressions in the [`Ast`].
    pub(crate) fn function_len(&self) -> usize {
        self.functions.len()
    }

    /// Number of argument entries, equal to the number of arguments in the
    /// [`Ast`].
    pub(crate) fn arg_len(&self) -> usize {
        self.arg_params.len()
    }

    /// Number of `for` entries, equal to the number of `for` expressions
    /// in the [`Ast`].
    pub(crate) fn for_len(&self) -> usize {
        self.for_slots.len()
    }

    /// Number of item entries in every item table, equal to the number of
    /// items in the [`Ast`].
    pub(crate) fn item_len(&self) -> usize {
        self.item_slots
            .len()
            .min(self.item_paths.len())
            .min(self.item_spreads.len())
            .min(self.jumps.len())
    }
}

/// The result of a successful check: the types and the resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The type of every expression.
    pub types: Types,
    /// Where every variable is stored.
    pub resolution: Resolution,
}

/// Everything a successful [`Checker::check`] produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOutput {
    /// The types and the resolution.
    pub checked: Checked,
    /// The outermost scope, extended with the body's top-level
    /// declarations.
    pub scope: Scope,
    /// The type of the body's value: its result's, the record of its fields,
    /// or `{}`.
    pub ty: TyId,
    /// The type of the cell of each node of the input's tree of top-level
    /// fields, by its index there; a `def`'s is its function's type.
    pub cell_types: Vec<TyId>,
    /// The warnings found, in source order.
    pub warnings: Vec<Diagnostic>,
}

/// A variable, field or callee in scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    name: Name,
    /// Its slot in its function's frame; [`Slot::NONE`] for a callee.
    slot: Slot,
    ty: TyId,
    /// Where it was declared: the span of the `let`, field or `:=` item, or
    /// of the parameter, or of the function expression for a callee; `None`
    /// if it was declared by an earlier input of a session, whose source is
    /// not this one.
    declared: Option<Span>,
    kind: BindingKind,
    /// The function expression a single-name `let`, field or named function
    /// item was initialised with directly, for the call-site form of the
    /// implicit-await error (see `Checker::argument_await_error`) and the
    /// defaults a call of it gives (see [Calls](#calls)); `None` for every
    /// other binding, and for one declared by an earlier input of a session,
    /// whose [`FnId`]s belong to another tree.
    function: Option<FnId>,
    /// For a callback parameter of an impure or memoised function being
    /// checked, its index in `Checker::callbacks`, where its calls and uses
    /// are recorded (see `Checker::narrow_callbacks`); `None` for every
    /// other binding.
    callback: Option<usize>,
}

impl Binding {
    /// The binding as a later input's [`Scope`] keeps it: without its span
    /// and function ids, which are meaningless for a later input, with its
    /// own source and tree.
    fn forgotten(self) -> Self {
        Self {
            declared: None,
            function: None,
            callback: None,
            ..self
        }
    }
}

/// What a check records of its input's names for an index of it, when it
/// is asked to ([`Checker::recording`]; see [Recording](#recording)), with
/// what its tables were as it ended, whether it succeeded or failed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Record {
    /// Each binding the check declared, in the order it declared them.
    pub(crate) decls: Vec<RecordedDecl>,
    /// Each name read from a binding declared in this input: the `Name`
    /// expression and where its binding was declared
    /// ([`RecordedDecl::declared`]). A name read through the file's view,
    /// or from a declaration another input committed, has none.
    pub(crate) uses: Vec<(ExprId, Span)>,
    /// Each `:=` that rebinds a binding declared in this input: its item,
    /// and where the binding was declared. A variable's `:=` declares a
    /// binding in its place, at the item, which an index takes as the same
    /// one; a mutable variable's, or a `for` header's, keeps its binding.
    pub(crate) rebinds: Vec<(Span, Span)>,
    /// The type of every expression as the check left it, indexed by
    /// [`ExprId`]: `Error` for one that failed, and, after a failed check,
    /// possibly untyped or provisional.
    pub(crate) types: Vec<TyId>,
    /// The type each `type` item of the input stands for, by item.
    pub(crate) declared_types: Vec<(ItemId, TyId)>,
    /// The type of the cell of each node of the input's tree of top-level
    /// fields, by its index there, as [`CheckOutput::cell_types`] has it.
    pub(crate) cells: Vec<TyId>,
    /// The type of the input's value, as [`CheckOutput::ty`] has it.
    pub(crate) value: Option<TyId>,
}

/// A binding a check declared, as [`Record`] keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecordedDecl {
    /// Where it was declared, as [`Binding`] says: a `let`'s, field's or
    /// `:=`'s item, a spread's item for its soft fields, a parameter, a
    /// `for`'s name, a function expression for its name for itself, and an
    /// `is` for the narrowed copy of the variable it tests.
    pub(crate) declared: Span,
    /// Its name.
    pub(crate) name: Name,
    /// Its type as it was declared.
    pub(crate) ty: TyId,
    /// What it is.
    pub(crate) kind: BindingKind,
}

/// What a [`Binding`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BindingKind {
    /// A variable: declared by a `let`, a `:=` or a parameter.
    Variable,
    /// A mutable variable, declared by a `let mut`, which `:=` rebinds in
    /// its one slot, from any block of its function, keeping its type and
    /// its binding.
    Mutable,
    /// A field, which `:=` may not rebind.
    Field,
    /// A function's name for itself, inside its own body, which `:=` may
    /// not rebind either; it has no slot.
    Callee,
    /// A variable declared by a plain `let` in the header of a `for`: one
    /// slot for its life, as a mutable variable's, which a `:=` in the
    /// header, after the condition, rebinds, keeping its type, but no `:=`
    /// in the body may (see [For](#for)).
    Header,
    /// The copy of a variable that a branch of `if NAME is TYPE` sees, of
    /// the narrowed type or the remainder, which `:=` may not rebind: the
    /// rebind would change the copy, not the variable it was copied from.
    Narrowed,
}

/// How the checker resolves the copies of declared types' definitions in a
/// file (see [Type declarations](#type-declarations)).
#[derive(Clone, Copy, Debug, Default)]
struct Quiet {
    /// Whether a copy of a declared type's definition, or a type of a file
    /// a workspace checked the `type` items of already, is being resolved:
    /// its errors are reported where it is written, not here.
    types: bool,
    /// Whether a type was erroneous with nothing of this file's to report,
    /// its error being reported where it is written: the check fails then,
    /// with no error of its own if it has none.
    failed: bool,
    /// Whether the input's view sees another file with syntax errors (see
    /// [`Checker::sees_broken`]).
    sees_broken: bool,
    /// How many values being checked, one inside another, are checked
    /// against an erroneous expected type, one whose error is reported
    /// elsewhere, or as the arguments of an erroneous callee: in one, an
    /// empty list, which the expected type would have typed, is no error of
    /// its own, the check failing quietly (see [`Checker::check_list`]).
    expecting_error: u32,
}

/// What a bare type written in a type expression means: see the module
/// documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Bareness {
    /// A bare type is present: `Int` is `Int`.
    Present,
    /// A bare type written as a name, a record type, a list type or a union
    /// with no `Future` member is the either shape: `Int` is `Int |
    /// Future<Int>`, `{a: Int}` is `{a: Int | Future<Int>} | Future<{a: Int
    /// | Future<Int>}>`, and `Int | String` is `Int | String | Future<Int |
    /// String>`. A function type is not (see `Checker::resolve_type`).
    Either,
}

/// What bare types mean in the parameters and in the return type of a
/// function of effect `effect`, as written in a function type or a function
/// expression's signature: present in both for a pure function; present in
/// the parameters, which are the key its results are memoised by, and the
/// either shape in the return type for a memoised one; the either shape in
/// both for an impure one. (A function expression then infers what its
/// pure parameters and its impure or memoised results are: see
/// `Checker::check_fn`.)
const fn bareness_of(effect: Effect) -> (Bareness, Bareness) {
    match effect {
        Effect::Pure => (Bareness::Present, Bareness::Present),
        Effect::Memo => (Bareness::Present, Bareness::Either),
        Effect::Impure => (Bareness::Either, Bareness::Either),
    }
}

/// The name the item `id` of the outermost block declares as a variable or
/// field: the first segment of the path of a `let`, `let mut`, field or
/// `:=` that is not a named function item or a `def`.
fn root_declares(ast: &Ast, id: ItemId) -> Option<Name> {
    let (ItemKind::Let {
        path,
        function: false,
        ..
    }
    | ItemKind::Field {
        path,
        function: false,
        ..
    }
    | ItemKind::Rebind { path, .. }) = ast.item(id).kind
    else {
        return None;
    };
    ast.path(path).first().and_then(PathSegment::name)
}

/// The tree of an input's top-level fields and when each of its nodes is
/// computed: what the checker found from [`Mentions`] and lowering stores
/// the cells by (see [Top-level fields](#top-level-fields)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldSchedule {
    /// The tree of the input's top-level fields.
    pub tree: FieldTree,
    /// The point of each node, and the order the nodes are computed in.
    pub schedule: Schedule,
}

/// What the checker knows of one node of the tree of the input's top-level
/// fields (see [Top-level fields](#top-level-fields)), or, in a workspace,
/// of one node of the file's view numbered after them (see
/// [Imports](#imports)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldCell {
    /// The slot of the top level's frame that holds its value.
    slot: Slot,
    /// For a root, the index in `bindings` of the binding of its name,
    /// declared as the input starts.
    binding: Option<usize>,
    /// Its type, once its initialiser is checked (a leaf) or its children
    /// are (an interior node); a reference to it before that, which only a
    /// cycle makes, is erroneous.
    ty: Option<TyId>,
    /// Once a leaf is checked, the position among the outermost block's
    /// items of the first item before which it can be computed: `0` if
    /// it reads nothing that stops it, its own item's if it reads a mutable
    /// variable, `usize::MAX` if it read a name it cannot see yet or is on a
    /// cycle (see [Top-level fields](#top-level-fields)).
    ready: usize,
    /// Why a leaf cannot be computed before `ready`, if it reads something
    /// that stops it; `None` on a cycle, which is reported once.
    blocker: Option<Blocker>,
    /// Whether it is on a cycle through fields, already reported: it is
    /// erroneous for its users.
    cyclic: bool,
}

impl FieldCell {
    /// A node's cell before the input is walked: no slot, binding or type
    /// yet, and nothing known that stops it.
    const NEW: Self = Self {
        slot: Slot::NONE,
        binding: None,
        ty: None,
        ready: 0,
        blocker: None,
        cyclic: false,
    };
}

/// Where the check of a top-level function's body is (see [Top-level
/// functions](#top-level-functions)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyState {
    /// Not checked yet: a use sees the declared type.
    Unchecked,
    /// Its group's bodies are being checked: a use from inside the group is
    /// treated as a recursive call through a function's own name is.
    Checking,
    /// Checked: a use sees the function fully resolved.
    Done,
}

/// One top-level function of the input being checked.
#[derive(Clone, Copy, Debug)]
struct TopFn {
    /// Its item, `fn NAME ...`, `def NAME ...` or `def PATH ...`.
    item: ItemId,
    /// Its function expression.
    expr: ExprId,
    /// Its function.
    function: FnId,
    /// Its name; `None` for a `def` named by a dotted path, which has no
    /// binding (see [Top-level functions](#top-level-functions)).
    name: Option<Name>,
    /// Its binding's index in `bindings`, declared as the input starts;
    /// `usize::MAX` for a `def` named by a dotted path, and, until
    /// `Checker::declare_shadowing_fns` declares it, for a `fn` that
    /// shadows a field root.
    binding: usize,
    /// For a `def` named by a dotted path, its leaf of the tree of
    /// top-level fields, whose cell holds its closure and its type: the
    /// type its signature declares, then, once its group is checked, the
    /// type its body gives it, as a plain function's binding has.
    leaf: Option<u32>,
    /// For a `def` named by a dotted path, once the walk has reached it,
    /// the walk's node of its leaf in the record of the input, which takes
    /// its type once its group is checked (see `Checker::nodes`); `NO_NODE`
    /// until then.
    node: u32,
    /// The number of bindings declared when the walk reached its
    /// declaration, once it has: the bindings its body sees.
    cut: Option<usize>,
    /// How far its body's check is.
    state: BodyState,
    /// Once its body is checked, the position among the outermost block's
    /// items of the first item before which its closure exists: `0` if it
    /// copies nothing of this input's top level, `usize::MAX` if its body
    /// read a name it cannot see yet (see [When a function may be
    /// used](#when-a-function-may-be-used)).
    ready: usize,
    /// Why it does not exist before `ready`, if it does not exist from the
    /// start.
    blocker: Option<Blocker>,
    /// Where its closure is made.
    made: MadeAt,
}

/// Why a top-level function does not exist before some item: what it
/// reads, and where that is bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Blocker {
    /// The name it reads.
    name: Name,
    /// The declaration labelled: the item binding the name, the spread
    /// that may define it, or, for a mutable variable, the function's own.
    span: Span,
    /// Which of those it is.
    kind: BlockerKind,
}

/// What a [`Blocker`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockerKind {
    /// A variable or field that is not mutable, bound by an item.
    Bound,
    /// A mutable variable, which the function copies where it is written.
    Mutable,
    /// A name a top-level spread may define.
    Spread,
}

/// Where the closure of a top-level function is made and stored in its
/// slot (see [When a function may be used](#when-a-function-may-be-used)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadeAt {
    /// As the input starts, before its first item: it copies nothing of
    /// the input's top level.
    Start,
    /// Just after the item given, the one that binds the last of the
    /// variables and fields it copies.
    After(ItemId),
    /// At its own declaration: it copies a mutable variable, or it is
    /// erroneous.
    Own,
}

/// A function expression or a `lazy` being checked: one entry of the
/// checker's stack of the functions it is inside, innermost last.
#[derive(Debug)]
struct Function {
    fn_id: FnId,
    /// The index in `Checker::bindings` of the function's first binding:
    /// every binding below it belongs to an enclosing function or the top
    /// level, and must be captured to be used.
    bindings_start: usize,
    /// The enclosing frame's slot allocation, restored on exit.
    outer_next_slot: u32,
    outer_frame_size: u32,
    outer_slot_names: Vec<Name>,
    /// What the function captures, in capture order.
    captures: Vec<Capture>,
    /// What the function may do, as declared: pure (`:`), memoised (`->>`)
    /// or impure (`->`); always impure for a `lazy`. Only
    /// [`Effect::Pure`] is pure.
    effect: Effect,
    /// Its `:`, `->>` or `->`, or a `lazy`'s keyword.
    marker: Span,
    /// Its `fn` or `lazy` keyword, where a `break` or `continue` that would
    /// leave it points.
    keyword: Span,
    /// For each parameter, where the body first measures it, in source
    /// order, so far; only a pure function's bare parameters, whose types
    /// are provisional, are ever measured. Empty for a `lazy`.
    demanded: Vec<Option<Span>>,
    /// The declared result type, as resolved, and where it is written,
    /// against which every `return` value is checked as the body's result
    /// is; `None` for a `lazy`, which declares none.
    result: Option<(TyId, Span)>,
    /// The `return`s for it so far, in the order they are checked: its
    /// result sites besides the body's own result (see
    /// [`return`](#return)).
    returns: Vec<Site>,
}

/// What the checker keeps while default functions are checked (see
/// [Calls](#calls)).
#[derive(Debug, Default)]
struct DefaultsChecked {
    /// The copies of the default functions of the top-level function being
    /// checked, which are its own too: its closure keeps their closures, so
    /// it is made where they can be.
    copies: Vec<usize>,
    /// While a parameter's default function is checked, that parameter and
    /// the ones after it, by name and where each is written, and those of
    /// each default it is in: what the default cannot see, which an
    /// "undefined variable" error of one of their names points at.
    unseen: Vec<(Name, Span)>,
}

/// One value that a `return` gives its target: a result site of a
/// function, a `lazy`, a labelled block or a loop (see
/// [`return`](#return)).
#[derive(Clone, Copy, Debug)]
struct Site {
    /// The value, or `None` for a `return` without one, which gives `{}`.
    value: Option<ExprId>,
    /// Where it is shown: the value, or the `return` and its label.
    at: Span,
    /// Its type: with an expected type, what it is once the awaits marked
    /// on it have run (see [`Checker::fit`]), and its own type otherwise.
    ty: TyId,
}

/// Where an await is decided, for the purity rule's error in a pure
/// function: see `Checker::await_here`.
#[derive(Clone, Copy, Debug)]
enum AwaitSite {
    /// Implicitly, where a value is used as its bits.
    Implicit,
    /// Explicitly, by an `await` whose keyword is at the span.
    Explicit(Span),
    /// Implicitly, at the top, as an argument of a call that fills a
    /// parameter the callee's body measures.
    Argument(CallSite),
}

/// An argument awaited because the parameter it fills is present by
/// inference, which the call-site form of the implicit-await error
/// describes.
#[derive(Clone, Copy, Debug)]
struct CallSite {
    /// The call.
    call: ExprId,
    /// Its callee.
    callee: ExprId,
    /// Where the callee's body first measures the parameter, and the
    /// parameter's name, if the callee's function is known: one of this
    /// input's function expressions.
    measured: Option<(Span, Name)>,
}

/// A callee whose function is known well enough to tell which of its
/// parameters are present because its body measures them, and which have
/// defaults (see [Calls](#calls)).
#[derive(Clone, Copy, Debug)]
enum KnownCallee {
    /// A function expression of the input being checked.
    Current(FnId),
    /// A binding of an earlier session input, by its index in
    /// `Checker::bindings`, which is also its index in
    /// `Checker::scope_measured`.
    Earlier(usize),
    /// A `def` of another file reached through an import, by its merged
    /// node in the workspace (see [Imports](#imports)), whose measured
    /// parameters the workspace context carries.
    Imported(u32),
}

/// One value a function captures.
#[derive(Clone, Copy, Debug)]
struct Capture {
    /// The index in `Checker::bindings` of the binding captured.
    binding: usize,
    /// Where the value is read from in the frame creating the closure.
    source: VarRef,
    /// Its type.
    ty: TyId,
}

/// A callback parameter of an impure or memoised function being checked,
/// in `Checker::callbacks`: what its body does with it so far, from which
/// its type is narrowed when the body ends. See the module documentation.
#[derive(Clone, Copy, Debug)]
struct Callback {
    /// The index of the parameter among its function's parameters.
    param: usize,
    /// Where the body first uses it as a value, other than as the callee of
    /// a call, in source order, so far.
    used: Option<Span>,
    /// The index in `Checker::callback_positions` of the first of its
    /// positions, one for each parameter of its function type, in order.
    positions: usize,
    /// The number of its positions.
    count: usize,
}

/// One parameter position of a callback parameter's function type, in
/// `Checker::callback_positions`.
#[derive(Clone, Copy, Debug)]
struct CallbackPosition {
    /// For a narrowable position, one written bare, the type expression it
    /// is written as and its present type, what it is narrowed to; `None`
    /// for any other.
    narrowable: Option<(TypeExprId, TyId)>,
    /// Where a call of the callback first gives the position an argument
    /// that does not fit its present type, in source order, so far.
    given: Option<Span>,
}

/// A field declared in the record block being checked. Its place in the
/// block's fields is where it was first defined, which an override keeps.
#[derive(Clone, Copy, Debug)]
struct FieldDecl {
    name: Name,
    /// Where the name is written in the field's first definition, or in
    /// the field item that overrode it, if one did; for a soft field, the
    /// operand of the spread whose definition stands.
    name_span: Span,
    ty: TyId,
    /// Its one slot, which every definition of it writes.
    slot: Slot,
    /// For an open field, one first defined by a dotted path, where its
    /// segments are tracked; `None` for a field defined whole.
    open: Option<OpenField>,
    /// For a soft field, one a spread defines, which a later item may
    /// define again, the operand of the spread whose definition stands so
    /// far; `None` for a hard field, one a field item defines.
    spread: Option<ExprId>,
    /// Whether a dotted path under it was rejected, which makes its block's
    /// record erroneous, so that the rejection does not cascade.
    rejected: bool,
}

/// What the path of a def-type reference names in its def-type's namespace
/// (see [Def-types](#def-types)).
#[derive(Clone, Debug, PartialEq, Eq)]
enum DefinitionAt {
    /// A definition.
    Leaf,
    /// A definition at a prefix of the path, whose read is this prefix of
    /// the reference's read, the rest of the path being read from its meta
    /// (see [Metas](#metas)): only for a def-type that declares one.
    Past(ExprId),
    /// A node above definitions, no definition itself.
    Interior,
    /// Nothing: with the paths of the definitions of the namespace the file
    /// sees, as written after the def-type.
    Missing(Vec<String>),
}

/// What a read of a meta reads it of, as its errors show it (see
/// [Metas](#metas)); `Checker::meta_of` says what each is.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MetaOf {
    /// "the meta of `func`".
    subject: String,
    /// Where the function is written.
    func: Span,
    /// Where the read is written.
    read: Span,
    /// For a def-type reference's path past its definition, the reference
    /// to the definition, `package dev.peios.testpkg`.
    past: Option<String>,
}

/// What the spread items of the innermost block checked so far have done,
/// in `Checker::spreads`. See the module documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BlockSpreads {
    /// The block has none yet, so every field of it is hard, and a field
    /// item need not look for a soft field of its name.
    #[default]
    None,
    /// It has some, and none was rejected.
    Some,
    /// It has some, and one was rejected: its operand is erroneous or not a
    /// record, or it defines a field defined before it. The block's record
    /// is erroneous, so that the rejection does not cascade.
    Rejected,
}

/// An open field: a record built by dotted paths.
#[derive(Clone, Copy, Debug)]
struct OpenField {
    /// Its root node in `Checker::nodes`.
    node: u32,
    /// The index of its binding in `Checker::bindings`, whose type grows
    /// with it.
    binding: usize,
}

/// The index of no node, where a node has no parent, child or sibling.
const NO_NODE: u32 = u32::MAX;

/// What a [`Node`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    /// A record being built by dotted paths, which further paths may
    /// extend: an open field, or a segment before the last of a dotted
    /// path.
    Open,
    /// The value at the end of a dotted path, defined whole.
    Leaf,
}

/// One segment of an open field, in `Checker::nodes`: the field itself, or
/// a segment within it. Its children are a linked list through
/// `first_child` and `next_sibling`, most recent first.
#[derive(Clone, Copy, Debug)]
struct Node {
    name: Name,
    /// The path from the field's name through this segment, where the item
    /// that first defined the segment writes it: where diagnostics about it
    /// point.
    span: Span,
    /// Its type: for an open node the record of its children's types, or
    /// `Error` if any of them is; for a leaf its value's.
    ty: TyId,
    kind: NodeKind,
    parent: u32,
    first_child: u32,
    next_sibling: u32,
}

/// The record type a record block is checked against, or the list type the
/// result of a block that is not one is checked against, or the record type
/// with defaults a record literal it results in is (see
/// [`Expecting::literal_only`]), or that the elements of a list literal it
/// results in are (see [`Expecting::elements_only`]), and where the
/// expectation comes from, for [`Checker::check_expr_expecting`].
#[derive(Clone, Copy, Debug)]
struct Expecting<'w> {
    /// The expected record type, present; or the expected list type or
    /// union, as it is expected, which may be its either shape.
    ty: TyId,
    /// The annotation, ascription, return type or callee stating it.
    context: Span,
    /// The label on `context`.
    why: &'w str,
    /// The block's closing brace, where a missing field is reported.
    close: Span,
    /// For a record literal, a record block or `{}`, whose expected record
    /// type is what is expected itself, once stripped, and not the one
    /// record member of a union: its id, by which the defaulted fields it
    /// lacks are recorded as given their defaults (see [Field
    /// defaults](#field-defaults)).
    fill: Option<ExprId>,
    /// For the result of a block that is not a record block: what of it
    /// `ty` reaches (see [`Reach`]).
    reach: Reach,
}

/// What the type an [`Expecting`] carries reaches of the result of a block
/// that is not a record block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// The whole result, which is checked against the type
    /// ([`Checker::fit`]).
    Whole,
    /// Only a record literal the result is, through blocks and the
    /// branches of an `if` with `else`, the type being a record type with
    /// defaults, to give it the defaults it lacks, the result being checked
    /// otherwise as it is with nothing expected (see
    /// [`Checker::fill_result`]).
    Literal,
    /// Only the elements of a list literal the result is, or the body of a
    /// `for` it is, through blocks and the branches of an `if` with `else`,
    /// the type being a record type with defaults that the elements are
    /// expected to be of, to give a record literal among them the defaults
    /// it lacks, the result being checked otherwise as it is with nothing
    /// expected (see [`Checker::fill_elements`]).
    Elements,
}

impl<'w> Expecting<'w> {
    /// The expectation of a value of type `ty`, stated at `context`,
    /// labelled `why`, of the block spanning `block`, which fills nothing.
    const fn new(ty: TyId, (context, why): (Span, &'w str), block: Span) -> Self {
        Self {
            ty,
            context,
            why,
            close: Span::new(block.end.saturating_sub(1), block.end),
            fill: None,
            reach: Reach::Whole,
        }
    }

    /// The expectation of the record type with defaults `record`, stated at
    /// `context`, labelled `why`, of the result of the block spanning
    /// `block`, which is not a record block, that reaches only a record
    /// literal the result is (see [`Reach::Literal`]).
    const fn literal_only(record: TyId, (context, why): (Span, &'w str), block: Span) -> Self {
        Self {
            reach: Reach::Literal,
            ..Self::new(record, (context, why), block)
        }
    }

    /// The expectation of the record type with defaults `record`, stated at
    /// `context`, labelled `why`, of the elements of the result of the
    /// block spanning `block`, which is not a record block, that reaches
    /// only the elements of a list literal the result is, or the body of a
    /// `for` it is (see [`Reach::Elements`]).
    const fn elements_only(record: TyId, (context, why): (Span, &'w str), block: Span) -> Self {
        Self {
            reach: Reach::Elements,
            ..Self::new(record, (context, why), block)
        }
    }
}

/// A type expected of a value, with where that is stated and its label.
type Stated<'w> = (TyId, (Span, &'w str));

/// What is expected of each branch of an `if`, for [`Checker::check_if`]
/// and [`Checker::check_branch`].
#[derive(Clone, Copy, Debug)]
enum Branch<'w> {
    /// Nothing: the branch is checked bottom-up.
    Free,
    /// A type, with where it is stated and its label, which the branch is
    /// checked against ([`Checker::fit`]).
    Fits(TyId, (Span, &'w str)),
    /// A record type with defaults, with where it is stated and its label,
    /// which reaches only a record literal the branch results in
    /// ([`Checker::fill_result`]).
    Fills(TyId, (Span, &'w str)),
    /// A record type with defaults that the elements of the branch, a list,
    /// are expected to be of, with where that is stated and its label,
    /// which reaches only the elements of a list literal the branch results
    /// in, or the body of a `for` it does ([`Checker::fill_elements`]).
    FillsElements(TyId, (Span, &'w str)),
}

/// What [`Checker::check_test`] found of an `is` or `as` that is not
/// erroneous. See the module documentation.
#[derive(Clone, Copy, Debug)]
struct Tested {
    /// The narrowed type: what the value is when the test passes.
    narrowed: TyId,
    /// The remainder: what the value is when the test fails; `None` if it
    /// always passes.
    remainder: Option<TyId>,
    /// Whether the remainder is the whole of the value's type: whether no
    /// member of it always passes.
    whole: bool,
    /// Whether the operand is marked to be awaited, which makes the value
    /// tested another than the operand's.
    awaited: bool,
    /// Whether the operand is of type `Never`, so the test is never run:
    /// the expression is `Never` too, nothing is planned and nothing
    /// narrows.
    unreached: bool,
}

/// A loop whose body, or a labelled block whose items, are being checked,
/// in `Checker::loops`, innermost last: what a `break`, `continue` or
/// `return` in it needs to find it, and what they give it (see [`break`
/// and `continue`](#break-and-continue), [Labels](#labels) and
/// [`return`](#return)). An unlabelled block has none.
#[derive(Clone, Debug)]
struct LoopFrame {
    /// The loop, or `None` for a labelled block.
    id: Option<ForId>,
    /// The loop or block expression.
    expr: ExprId,
    /// Its label, if it has one; a block's always has one.
    label: Option<Label>,
    /// The number of functions being checked when its body began: a
    /// `break` in a function or `lazy` nested in the body sees more, and
    /// cannot leave it for this loop.
    depth: usize,
    /// The element type expected of the loop, if a list type is, with where
    /// that is stated and its label: the value of each `break` and
    /// `continue` for it is checked against it.
    expected: Option<(TyId, Span, String)>,
    /// The value of each `break` and `continue` for it, in the order they
    /// are checked, with what it is: its type, or with an expected type,
    /// what it is once the awaits marked on it have run.
    values: Vec<(ExprId, TyId)>,
    /// Whether a `break` is for it, which a loop with no condition needs
    /// to end at all (see [Never-ending loops](#never-ending-loops)).
    broken: bool,
    /// The type expected of the whole loop or block, if one is, with where
    /// that is stated and its label: the value of each `return` for it is
    /// checked against it.
    returns_expected: Option<(TyId, Span, String)>,
    /// The `return`s for it so far, in the order they are checked.
    returns: Vec<Site>,
}

impl LoopFrame {
    /// The frame of the loop `id`, the expression `expr`, labelled `label`
    /// if it has one, in the body of the function at `depth`, of which the
    /// element type `expected` is expected, and the type `returns_expected`
    /// as a whole, if they are.
    fn new(
        (id, expr): (Option<ForId>, ExprId),
        label: Option<Label>,
        depth: usize,
        expected: Option<(TyId, Span, String)>,
        returns_expected: Option<(TyId, Span, String)>,
    ) -> Self {
        Self {
            id,
            expr,
            label,
            depth,
            expected,
            values: Vec::new(),
            broken: false,
            returns_expected,
            returns: Vec::new(),
        }
    }

    /// The target that a `break`, `continue` or `return` for it has.
    const fn target(&self) -> JumpTarget {
        match self.id {
            Some(id) => JumpTarget::Loop(id),
            None => JumpTarget::Block(self.expr),
        }
    }

    /// What it is, as diagnostics name it: "loop" or "block".
    const fn noun(&self) -> &'static str {
        match self.id {
            Some(_) => "loop",
            None => "block",
        }
    }
}

/// The bindings an `if` whose condition narrows a variable gives its
/// branches, and the slot they share. See the module documentation.
#[derive(Clone, Copy, Debug, Default)]
struct Narrowing {
    /// The `then` branch's binding.
    then: Option<Binding>,
    /// The `else` branch's binding, if it has one.
    else_: Option<Binding>,
    /// The next free slot before the slot the bindings share was taken,
    /// to which it goes back once the `if` is checked; `None` if the `if`
    /// does not narrow.
    next_slot: Option<u32>,
}

/// What an impure function's body is known to return at one position of
/// its result type, once every await the checker marked for fitting it to
/// the declared result has run, for [`Checker::inferred`]. See the module
/// documentation.
#[derive(Clone, Copy, Debug)]
enum BodyValue {
    /// A value that certainly has this type.
    Typed(TyId),
    /// The present record that a record block checked field by field
    /// against the declared record type builds: the block's items, whose
    /// fields' values say what the record holds.
    Fields(ItemRange),
    /// The present record that the dotted paths of such a block build at
    /// an open field or a segment of one: the index of its node in
    /// `Checker::built`, whose leaves' values say what the record holds.
    Open(u32),
}

/// One segment of an open field of a record block that an impure
/// function's body returns, rebuilt from the block's dotted paths for
/// [`Checker::inferred`], since the block's own [`Node`]s are gone once it
/// ends. Its children are a linked list through `first_child` and
/// `next_sibling`, most recent first, as a [`Node`]'s are.
#[derive(Clone, Copy, Debug)]
struct Built {
    name: Name,
    /// For the end of a dotted path, the value it is defined whole with;
    /// `None` for a record the paths build.
    leaf: Option<ExprId>,
    first_child: u32,
    next_sibling: u32,
}

/// How a field of a record block that an impure function's body returns
/// is defined, for [`Checker::infer_field`].
#[derive(Clone, Copy, Debug)]
enum Definition {
    /// Whole, with this value: a field, or the end of a dotted path.
    Whole(ExprId),
    /// Softly, by a spread, whose operand gives the field this type: the
    /// type of the value the spread reads from the operand and stores, as
    /// it is.
    Soft(TyId),
    /// By dotted paths: an open field or a segment of one, the index of
    /// its node in `Checker::built`.
    Dotted(u32),
}

/// The variables of an outermost scope, with their slots and types: what a
/// [`crate::session::Session`] carries from one input to the next.
///
/// Only a [`Checker`] produces a non-empty scope. Its slots are `0..n`, `n`
/// being its first free slot, which it keeps explicitly (`slot_names`) since
/// not every slot is a binding's: the outermost block's declarations are the
/// first to take slots and no block is left open around them, and a slot
/// that no name finds, such as a cell of a top-level dotted field, is
/// committed with them. The bindings occupy their slots in declaration
/// order; the definitions of a field that overrides a soft one share the
/// field's slot, so `n` is not the number of bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    bindings: Vec<Binding>,
    /// The name each slot `0..n` shows in a dump, `n` being the first slot
    /// the next input may take: the last binding in it, or for a slot no
    /// binding owns, what the checker named it.
    slot_names: Vec<Name>,
    /// For each binding, if it was initialised directly with a function
    /// expression, what a call of it needs to know of that function's
    /// parameters (see [`KnownParams`]): which are present because its body
    /// measures them, which the call-site form of the implicit-await error
    /// needs, and their defaults, of a function declared by an earlier
    /// input, whose tree and spans are gone.
    known: Vec<Option<Rc<KnownParams>>>,
}

impl Scope {
    /// An empty scope, as at the start of a file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of variables declared, counting shadowed ones and every
    /// definition of a field.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether no variable has been declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The slots of the scope's mutable variables, in declaration order. A
    /// later input checked from the scope can write no other of its slots:
    /// it may rebind a mutable variable from any block of its top level,
    /// and every declaration it makes takes a slot of its own, after the
    /// scope's.
    pub fn mutable_slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.bindings
            .iter()
            .filter(|binding| binding.kind == BindingKind::Mutable)
            .map(|binding| binding.slot)
    }

    /// A scope with the variables of each of `parts` in turn that its
    /// indices pick, in their order, whose first free slot is
    /// `slot_names.len()`, its slots named by `slot_names`: what a file of a
    /// workspace is checked from, the declarations of its directory chain
    /// of the names it spells (see [Directory scope](#directory-scope)).
    pub(crate) fn joined<'s>(
        parts: impl Iterator<Item = (&'s Scope, Vec<usize>)>,
        slot_names: Vec<Name>,
    ) -> Self {
        let mut scope = Self {
            slot_names,
            ..Self::default()
        };
        for (part, picked) in parts {
            for index in picked {
                if let (Some(&binding), Some(known)) =
                    (part.bindings.get(index), part.known.get(index))
                {
                    scope.bindings.push(binding);
                    scope.known.push(known.clone());
                }
            }
        }
        scope
    }

    /// The name of each of the scope's variables, in declaration order.
    pub(crate) fn names(&self) -> impl Iterator<Item = Name> + '_ {
        self.bindings.iter().map(|binding| binding.name)
    }

    /// Adds the fields of `from` after its first `start` whose names `keep`
    /// accepts, without their slots' names: a file's declarations, committed
    /// to its directory's scope. A variable is never one, even of such a
    /// name: a top-level `let` or `fn` that shadows a shadowable field of its
    /// file is the file's alone (see [Shadowed
    /// declarations](#shadowed-declarations)).
    pub(crate) fn commit(&mut self, from: &Scope, start: usize, keep: impl Fn(Name) -> bool) {
        let bindings = from.bindings.iter().zip(&from.known).skip(start);
        for (binding, known) in bindings {
            if binding.kind == BindingKind::Field && keep(binding.name) {
                self.bindings.push(*binding);
                self.known.push(known.clone());
            }
        }
    }

    /// Makes each variable from the `start`th on whose name `poisoned`
    /// accepts of type `Error`, with nothing known of a function it holds:
    /// a declaration of a workspace that the scan found conflicting, which
    /// the files checked after it read silently as erroneous (see
    /// [`crate::workspace`], "Scan errors").
    pub(crate) fn poison_from(&mut self, start: usize, poisoned: impl Fn(Name) -> bool) {
        let bindings = self.bindings.iter_mut().zip(&mut self.known).skip(start);
        for (binding, known) in bindings {
            if poisoned(binding.name) {
                binding.ty = TyId::ERROR;
                binding.function = None;
                *known = None;
            }
        }
    }

    /// The names of the scope's slots, `0..n`, `n` being its first free
    /// slot.
    pub(crate) fn into_slot_names(self) -> Vec<Name> {
        self.slot_names
    }

    /// The scope's first free slot.
    pub(crate) fn slot_count(&self) -> usize {
        self.slot_names.len()
    }

    /// The scope with no variables whose slots `0..n` are named by `names`:
    /// what a session's first input is checked from, the standard
    /// library's slots taken (see [`crate::session`]).
    pub(crate) fn of_slots(names: Vec<Name>) -> Self {
        Self {
            slot_names: names,
            ..Self::default()
        }
    }

    /// The scope with slots named by `names` added after its own: the cells
    /// of a session's input, which its check finds taken (see
    /// [`crate::workspace::check_input`]).
    pub(crate) fn with_slots(mut self, names: impl IntoIterator<Item = Name>) -> Self {
        self.slot_names.extend(names);
        self
    }

    /// The scope with `fields` added after its own variables, each a
    /// field of a name, read from a slot the scope's slots already include,
    /// with a type, and, for a `def`'s cell, what a call of it needs to know
    /// of its parameters: cells of a workspace, under names no source can
    /// write, which the call a workspace makes to invoke a definition by
    /// name reads, and so calls the def-type as a known callee (see
    /// [`crate::workspace`], "Def-types by name").
    pub(crate) fn with_fields(
        mut self,
        fields: impl IntoIterator<Item = (Name, Slot, TyId, Option<Rc<KnownParams>>)>,
    ) -> Self {
        for (name, slot, ty, known) in fields {
            self.bindings.push(Binding {
                name,
                slot,
                ty,
                declared: None,
                kind: BindingKind::Field,
                function: None,
                callback: None,
            });
            self.known.push(known);
        }
        self
    }

    /// For each slot bound, what a call of the function its latest binding
    /// holds needs to know of its parameters, if that binding was
    /// initialised directly with a function expression: what a call of an
    /// imported `def` needs (see [Imports](#imports)). One map for every
    /// slot, so that a file's cells are looked up in it in constant time
    /// each.
    pub(crate) fn known_by_slot(&self) -> HashMap<Slot, Option<&Rc<KnownParams>>> {
        self.bindings
            .iter()
            .zip(&self.known)
            .map(|(binding, known)| (binding.slot, known.as_ref()))
            .collect()
    }
}

/// Type-checks and resolves `ast` as a file, starting from an empty scope,
/// interning its types into `types`. On success, returns the tables and the
/// warnings found, in source order. The file is checked with no standard
/// library: it knows no name it does not declare, `len` among them, and
/// imports nothing; [`crate::run`] checks a file beside it.
///
/// # Errors
///
/// Returns every type and scope error found, together with the warnings, in
/// source order. Checking continues after an error by giving the offending
/// node [`TyId::ERROR`], which every enclosing operator accepts silently, so
/// one run reports each independent error exactly once.
pub fn check(
    ast: &Ast,
    types: &mut TypeTable,
) -> Result<(Checked, Vec<Diagnostic>), Vec<Diagnostic>> {
    Checker::new(ast, types)
        .check()
        .map(|output| (output.checked, output.warnings))
}

/// Checks the `type` items of `ast`, a file of a workspace whose types are
/// expanded ([`Ast::expand_types`]), interning types into `types`: what is
/// wrong in each declaration, reported once, here, before any file is
/// checked, since a file may use a type of a file checked after it (see
/// [Type declarations](#type-declarations)). Returns the errors, none for a
/// file with no `type` item. If the file's view `sees_broken`, a file with
/// syntax errors, an unknown type is none of them (see
/// [`Checker::unresolved`]).
pub(crate) fn check_types(ast: &Ast, types: &mut TypeTable, sees_broken: bool) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    checker.quiet.sees_broken = sees_broken;
    checker.type_items();
    checker.errors
}

/// The names of the built-in types: those of [`BuiltinTypes`], and the
/// type constructors, `List`, `Future` and `Present`. A `type` may not be
/// named like one: "`Int` is a built-in type".
pub(crate) const BUILTIN_TYPE_NAMES: [&str; 9] = [
    "Int", "Float", "Bool", "String", "Duration", "Size", "List", "Future", "Present",
];

/// The name of the checker's attribute that retries a call.
const RETRY: &str = "retry";

/// The name of the checker's attribute that limits a call's time.
const TIMEOUT: &str = "timeout";

/// The error for an `#[attribute]` type that is no record of constants.
const NOT_CONSTANT_RECORD: &str = "an attribute's arguments are a record of constants";

/// The note on [`NOT_CONSTANT_RECORD`].
const CONSTANT_FIELDS_NOTE: &str = "each field must be an `Int`, `Float`, `Bool`, `String`, \
     `Duration` or `Size`, or a list or record of them";

/// The fields of an attribute's record, by name, in its order, each with
/// its type and default, if it has one (see [Attributes](#attributes)).
type AttributeFields = Vec<(Box<str>, TyId, Option<Constant>)>;

/// The function of the item `item`, a function item that attributes apply
/// to: a named `fn`, a `def` or a definition (see [Attributes](#attributes)).
fn item_function(ast: &Ast, item: ItemId) -> Option<FnId> {
    let (ItemKind::Let {
        value,
        function: true,
        ..
    }
    | ItemKind::Field {
        value,
        function: true,
        ..
    }
    | ItemKind::DefTypeDef { value, .. }) = ast.item(item).kind
    else {
        return None;
    };
    match ast.get(value).kind {
        ExprKind::Fn(function) => Some(function),
        _ => None,
    }
}

/// The names of the built-in attributes: the parser's, `shadowable`,
/// `def_type`, `attribute` and `no_prelude`, and the checker's, `retry`
/// and `timeout` (see [Attributes](#attributes)). No `#[attribute]` type
/// may be named like one: "`retry` is a built-in attribute".
pub(crate) const BUILTIN_ATTRIBUTES: [&str; 6] = [
    "shadowable",
    "def_type",
    "attribute",
    "no_prelude",
    "retry",
    "timeout",
];

/// The type names every program can use, as `(name, type)` pairs. A name
/// the program never mentions has no [`Name`].
type BuiltinTypes = [(Option<Name>, TyId); 6];

/// The [`BuiltinTypes`] of a program whose names are `names`.
fn builtin_types(names: &Names) -> BuiltinTypes {
    [
        (names.lookup("Int"), TyId::INT),
        (names.lookup("Float"), TyId::FLOAT),
        (names.lookup("Bool"), TyId::BOOL),
        (names.lookup("String"), TyId::STRING),
        (names.lookup("Duration"), TyId::DURATION),
        (names.lookup("Size"), TyId::SIZE),
    ]
}

/// A check in progress. See the module documentation.
pub struct Checker<'a> {
    ast: &'a Ast,
    /// Where types are interned.
    types: &'a mut TypeTable,
    /// The type of each expression, indexed by [`ExprId`].
    expr_types: Vec<TyId>,
    expr_vars: Vec<VarRef>,
    /// Whether and how each expression is awaited, indexed by [`ExprId`].
    awaits: Vec<AwaitMark>,
    item_slots: Vec<Slot>,
    /// See [`Resolution::spread_slots`].
    item_spreads: Vec<(u32, u32)>,
    spread_slots: Vec<Slot>,
    item_paths: Vec<PathResolution>,
    /// Whether each existing record along the path of each dotted `let` or
    /// `:=` with an awaited one is awaited: see [`Resolution::path_awaits`].
    path_awaits: Vec<bool>,
    /// What has been found for each function expression, indexed by
    /// [`FnId`].
    fn_resolutions: Vec<FnResolution>,
    /// The parameter each argument fills, indexed like the argument pool.
    arg_params: Vec<u32>,
    /// The slot of each `for` expression's variable, indexed by [`ForId`].
    for_slots: Vec<Slot>,
    /// What each `break`, `continue` and `return` item is for, indexed by
    /// [`ItemId`]: see [`Resolution::target`].
    jumps: Vec<Option<JumpTarget>>,
    /// The loops whose bodies, and the labelled blocks whose items, are
    /// being checked, outermost first.
    loops: Vec<LoopFrame>,
    /// What was found of each loop, indexed by [`ForId`]: see
    /// [`Resolution::loop_of`].
    loop_resolutions: Vec<LoopResolution>,
    /// The labelled blocks some `return` leaves: see
    /// [`Resolution::is_returned_block`].
    returned_blocks: HashSet<ExprId>,
    /// The calls of a function or built-in that is not pure, by their ids,
    /// which a statement may discard the value of.
    impure_calls: HashSet<ExprId>,
    /// The statements whose values are discarded: see
    /// [`Resolution::is_discarded`].
    discarded: HashSet<ExprId>,
    /// The values of the `return`s for each labelled block that has any,
    /// by the block's id, which an untyped integer pushed into the block
    /// reaches as it reaches the block's result (see
    /// [`Checker::coerce`]).
    block_returns: HashMap<ExprId, Vec<ExprId>>,
    /// `Never`, interned into the table once, as the checker starts.
    never: TyId,
    /// Set by [`Checker::walk_path`] when it stops at a record or list of
    /// type `Never` along a path, which no value has, so that the update is
    /// unreachable and its caller declares quietly, with `Never`, rather
    /// than with `Error`.
    unreached_path: bool,
    /// Every variable, field and callee in scope, outermost block first.
    bindings: Vec<Binding>,
    /// For each binding of the scope the check started from, its entry of
    /// [`Scope`]'s `known`.
    scope_known: Vec<Option<Rc<KnownParams>>>,
    /// What the check gives lowering besides types and slots.
    lowering: ForLowering,
    /// What the input's top-level items mention, and the groups of its
    /// top-level functions (see [`crate::mentions`]).
    mentions: Mentions,
    /// The workspace the input is a file of, if it is one (see [Directory
    /// scope](#directory-scope)).
    workspace: Option<&'a Context<'a>>,
    /// Each reference through an import to a merged node, with the node:
    /// a call of one is of a known callee if it is a `def`'s (see
    /// [`KnownCallee::Imported`]).
    imported_callees: HashMap<ExprId, u32>,
    /// The records the file reads whole through its imports, each with its
    /// private slot and fields (see [`Resolution::imported_records`]).
    imported_records: ImportedRecords,
    /// The target of each of `imported_records`, by its index there, with
    /// the record's type.
    imported_targets: Vec<(Target, TyId)>,
    /// See [`Resolution::view_record`]: the record of each node of the
    /// file's view with a leaf of the file under it, by its index among
    /// them.
    view_records: Vec<ViewRecord>,
    /// See [`Resolution::value_cell`].
    value_cell: Slot,
    /// For each node of the input's tree that is a shared interior node of
    /// its directory, the private slot its own record is stored in, for the
    /// input's value; [`Slot::NONE`] for any other node.
    own_records: Vec<Slot>,
    /// The cell of each node of the input's tree of top-level fields, by
    /// its index in `mentions.tree` (see [Top-level
    /// fields](#top-level-fields)).
    field_cells: Vec<FieldCell>,
    /// For each node of the graph of what needs what (`Checker::need_index`),
    /// once known: the latest item before which something it needs, itself
    /// included, cannot be had (`Checker::closure_ready`), 0 if nothing it
    /// needs is ever blocked. Lets `Checker::check_uses` skip the search for
    /// a reason in the common case, so a chain of N fields costs N steps.
    closure_ready: Vec<Option<usize>>,
    /// When each node of the tree is computed.
    schedule: Schedule,
    /// How far `schedule.order` has been checked: the nodes before this
    /// index in it are.
    scheduled: usize,
    /// The end of the bindings of the input's top-level declarations in
    /// `bindings`: from `top_start`, those of its top-level functions, then
    /// those of its top-level field roots that are not `def`s.
    top_end: usize,
    /// The input's top-level functions, indexed as in `mentions`.
    top_fns: Vec<TopFn>,
    /// The index in `bindings` of the first top-level function's binding:
    /// those of `top_fns` follow it, in order.
    top_start: usize,
    /// For each top-level function, once its body is checked, the indexes
    /// in `bindings` of what it copies.
    top_copies: Vec<Vec<usize>>,
    /// What the check of the default functions being checked keeps.
    defaults_checked: DefaultsChecked,
    /// The top-level function whose body is being checked, if one is.
    checking_top: Option<usize>,
    /// The leaf of the tree of top-level fields whose initialiser is being
    /// checked, if one is (see [Top-level fields](#top-level-fields)).
    checking_leaf: Option<u32>,
    /// The top-level functions on a cycle through fields, already reported:
    /// erroneous for their users, as an erroneous function is.
    cyclic_fns: Vec<u32>,
    /// While a top-level function's body is checked before the walk has
    /// reached it: the positions among the outermost block's items of the
    /// item being checked and of the function's declaration. A read of a
    /// name an item between them declares is not resolved (see [When each
    /// body is checked](#top-level-functions)).
    early: Option<(usize, usize)>,
    /// The position among the outermost block's items of the item the walk
    /// is at.
    root_position: usize,
    /// While a block nested in the top level's code (not in a function) is
    /// being checked, the number of the top level's own bindings: those
    /// below are the nested blocks'.
    root_end: Option<usize>,
    /// The functions being checked, innermost last; empty at the top
    /// level.
    functions: Vec<Function>,
    /// Reused for collecting the parameters of provisional types in a type.
    claimed: Vec<u32>,
    /// The name the next function expression checked sees itself by: set by
    /// a single-name `let` or field whose initialiser is directly a
    /// function expression, and taken by that function when its check
    /// starts, so no function nested in it sees it.
    callee_name: Option<Name>,
    /// Reused for building function types: each parameter's name and type.
    /// Nested uses push above the entries of the use enclosing them and
    /// truncate back, as `sorted_fields` does.
    fn_params: Vec<(Option<Name>, TyId)>,
    /// Reused for binding a call's arguments: for each parameter, the index
    /// in the call's arguments of the argument filling it, or `u32::MAX`.
    filled: Vec<u32>,
    /// Index in `bindings` of the innermost block's first binding.
    block_start: usize,
    /// The fields of every block being checked, outermost block first.
    fields: Vec<FieldDecl>,
    /// Index in `fields` of the innermost block's first field.
    fields_start: usize,
    /// The roots of the namespaces of the def-types the input's
    /// definitions are made with, declared as the walk reaches them, as
    /// `fields` holds the outermost block's: apart from them, since a
    /// namespace is no field of the input's record (see
    /// [Def-types](#def-types)).
    def_fields: Vec<FieldDecl>,
    /// The definition made with a def-type each function expression of the
    /// input's outermost block is, if it is one.
    definitions: HashMap<FnId, DefTypeId>,
    /// The function expressions of the input's `def`s marked `#[def_type]`.
    def_type_fns: HashSet<FnId>,
    /// The definition's meta each function expression of the input's
    /// outermost block is, if it is one (see [Metas](#metas)).
    defmetas: HashMap<FnId, DefTypeId>,
    /// See [`Resolution::meta_read`].
    meta_reads: HashMap<ExprId, (TyId, AwaitMark)>,
    /// See [`Resolution::metas`].
    metas: Vec<(FnId, Slot)>,
    /// See [`Types::reference_past`].
    references_past: Vec<(ExprId, ExprId, TyId)>,
    /// While the path of a def-type reference past its definition is read
    /// from the definition's meta: the read of the definition, a prefix of
    /// the reference's read, and the reference, so that the meta's errors
    /// name the reference (see [Metas](#metas)).
    reading_past: Option<(ExprId, DefTypeId)>,
    /// What the innermost block's spread items have done so far.
    spreads: BlockSpreads,
    /// The next free slot.
    next_slot: u32,
    /// The highest `next_slot` reached.
    frame_size: u32,
    slot_names: Vec<Name>,
    /// The first slot past the outermost block's slots that no binding
    /// owns, such as the cells of the input's top-level fields, which a
    /// session commits with the input: the [`Scope`]'s first free slot is
    /// at least this.
    top_slots_end: u32,
    /// The last top-level item reported as "`a` is a field of this file",
    /// whose `:=` is then not reported again as a rebind of a field.
    field_clash: Option<ItemId>,
    /// The top-level items a later declaration shadows, each with the
    /// number of bindings declared when the walk passed it: checked after
    /// the last item, as written, in the view of the top level above them
    /// (see [Shadowed declarations](#shadowed-declarations)).
    shadowed_items: Vec<(ItemId, usize)>,
    /// Whether a shadowed declaration is being checked, whose reads of
    /// records it alone would need are erroneous.
    checking_shadowed: bool,
    builtin_types: BuiltinTypes,
    /// The name `Future`, which applied to a type is a future of it, if the
    /// program mentions it.
    future_name: Option<Name>,
    /// The name `Present`, which applied to a type is it with every bare
    /// type in it present, if the program mentions it.
    present_name: Option<Name>,
    /// The name `List`, which applied to a type is a list of it, if the
    /// program mentions it.
    list_name: Option<Name>,
    /// TEMPORARY (alias rule): the expression that may be a reference to a built-in function whose
    /// typing is special while it is checked: the callee of the call being checked, or the
    /// initialiser of the top-level field or `let` of one name being checked (see
    /// [`Checker::builtin_use`]). Replaced when built-in functions get real polymorphic types (see
    /// the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    alias_site: Option<ExprId>,
    /// The callee of the call being checked, while it is checked: where
    /// `fileroot.helper` may stand, which is called and never a value (see
    /// [Imports](#imports)).
    call_site: Option<ExprId>,
    /// Reused for the types of the elements of the list literals being
    /// checked, innermost literal's last: a stack, as `sorted_fields` is.
    element_types: Vec<TyId>,
    /// Reused for sorting the fields of a record block or a record type by
    /// name: each field's name, its position in source order and its type.
    /// Nested uses push above the entries of the use enclosing them and
    /// truncate back, so it is a stack and one allocation serves them all.
    /// Also where the fields of an open node are gathered to intern its
    /// type.
    sorted_fields: Vec<(Name, u32, TyId)>,
    /// The segments of the open fields of every block being checked,
    /// outermost block first; each block truncates it back when it ends.
    nodes: Vec<Node>,
    /// The segments of the open fields of every record block whose value
    /// an impure function's result is being inferred from, rebuilt from
    /// their dotted paths: a stack, as `sorted_fields` is, each block's
    /// segments above those of the block enclosing it.
    built: Vec<Built>,
    /// For each declared field of every record block with a spread whose
    /// value an impure function's result is being inferred from, the
    /// definition of it that stands so far, from the block's first spread
    /// on: a stack, as `built` is (see [`Checker::infer_block_fields`]).
    standing: Vec<Option<Definition>>,
    /// Reused for the records along the path of a dotted `let` or `:=` that
    /// exist: each one's type once measured, and whether it is awaited. A
    /// `:=` walks its path before its value is checked and declares its
    /// variable after, so an update nested in the value pushes above the
    /// entries of the one enclosing it and truncates back, as with
    /// `sorted_fields`.
    path_types: Vec<(TyId, bool)>,
    /// Reused as the work list of open nodes still to check against their
    /// expected types when a record block ends.
    pending: Vec<(u32, TyId)>,
    /// The callback parameters of every impure or memoised function being
    /// checked, outermost function first; each function truncates it back
    /// when it ends.
    callbacks: Vec<Callback>,
    /// The parameter positions of the function types of `callbacks`, each
    /// callback's together, in the same order.
    callback_positions: Vec<CallbackPosition>,
    /// See [`Resolution::test_of`].
    tests: HashMap<ExprId, TestResolution>,
    /// Whether types are being resolved quietly, and whether one was
    /// erroneous with nothing to report here.
    quiet: Quiet,
    /// The type of each `type` item of the file, for `--dump-types`.
    declared_types: Vec<(ItemId, TyId)>,
    /// What each node of a copy of a declared type's definition denotes in
    /// each mode it has been resolved in (see [`Checker::resolve_type`]).
    copies: HashMap<(TypeExprId, Bareness), TyId>,
    /// What is recorded of the input's names for an index of it, if the
    /// check was asked to record ([`Checker::recording`]); `None`, and so
    /// costing one test per binding and per name read, on every other path
    /// (see [Recording](#recording)).
    record: Option<Box<Record>>,
    /// The roots of the file's view with their ranks, for suggestions (see
    /// [Suggestions](#suggestions)): gathered at the first undefined
    /// variable, never on valid input.
    view_roots: Option<Vec<(Name, u8)>>,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    /// A checker for `ast` as a file, starting from an empty scope and
    /// interning types into `types`.
    #[must_use]
    pub fn new(ast: &'a Ast, types: &'a mut TypeTable) -> Self {
        Self::with_scope(ast, Scope::new(), types)
    }

    /// A checker for `ast`, a file of the workspace `workspace`, checked
    /// from `scope`, the declarations of its directory chain that earlier
    /// files committed (see [Directory scope](#directory-scope)).
    #[must_use]
    pub(crate) fn in_workspace(
        ast: &'a Ast,
        scope: Scope,
        types: &'a mut TypeTable,
        workspace: &'a Context<'a>,
    ) -> Self {
        let mut checker = Self::with_scope(ast, scope, types);
        checker.quiet.sees_broken = workspace.sees_broken();
        Self {
            workspace: Some(workspace),
            ..checker
        }
    }

    /// A checker for `ast` whose outermost block starts with the variables
    /// of `scope` already declared. `scope` must come from checking earlier
    /// input interned into the same [`crate::names::Names`] as `ast` and the
    /// same [`TypeTable`] as `types`.
    #[must_use]
    pub fn with_scope(ast: &'a Ast, scope: Scope, types: &'a mut TypeTable) -> Self {
        let names = ast.names();
        // The scope's slots are exactly `0..n`, each named as it was when
        // the scope was made; some, such as a dotted field's cells, belong
        // to no binding.
        let next_slot = u32::try_from(scope.slot_names.len()).unwrap_or(u32::MAX);
        Self {
            ast,
            never: types.never(),
            types,
            // Placeholder only: every node is written by `check_expr` before
            // it is read, and the final invariant check catches any that
            // are not.
            expr_types: vec![TyId::ERROR; ast.len()],
            expr_vars: vec![VarRef::None; ast.len()],
            awaits: vec![AwaitMark::None; ast.len()],
            item_slots: vec![Slot::NONE; ast.item_count()],
            item_spreads: vec![(0, 0); ast.item_count()],
            spread_slots: Vec::new(),
            item_paths: vec![PathResolution::NONE; ast.item_count()],
            path_awaits: Vec::new(),
            fn_resolutions: vec![FnResolution::default(); ast.fn_count()],
            arg_params: vec![u32::MAX; ast.arg_count()],
            for_slots: vec![Slot::NONE; ast.for_count()],
            jumps: vec![None; ast.item_count()],
            loops: Vec::new(),
            loop_resolutions: vec![LoopResolution::default(); ast.for_count()],
            returned_blocks: HashSet::new(),
            impure_calls: HashSet::new(),
            discarded: HashSet::new(),
            block_returns: HashMap::new(),
            unreached_path: false,
            bindings: scope.bindings,
            scope_known: scope.known,
            lowering: ForLowering::default(),
            mentions: Mentions::default(),
            workspace: None,
            imported_callees: HashMap::new(),
            imported_records: Vec::new(),
            imported_targets: Vec::new(),
            view_records: Vec::new(),
            value_cell: Slot::NONE,
            own_records: Vec::new(),
            field_cells: Vec::new(),
            closure_ready: Vec::new(),
            schedule: Schedule::default(),
            scheduled: 0,
            top_end: 0,
            top_fns: Vec::new(),
            top_copies: Vec::new(),
            defaults_checked: DefaultsChecked::default(),
            checking_top: None,
            checking_leaf: None,
            cyclic_fns: Vec::new(),
            early: None,
            root_position: 0,
            top_start: 0,
            root_end: None,
            functions: Vec::new(),
            claimed: Vec::new(),
            callee_name: None,
            fn_params: Vec::new(),
            filled: Vec::new(),
            block_start: 0,
            fields: Vec::new(),
            fields_start: 0,
            def_fields: Vec::new(),
            definitions: HashMap::new(),
            def_type_fns: HashSet::new(),
            defmetas: HashMap::new(),
            meta_reads: HashMap::new(),
            metas: Vec::new(),
            references_past: Vec::new(),
            reading_past: None,
            spreads: BlockSpreads::None,
            next_slot,
            frame_size: next_slot,
            slot_names: scope.slot_names,
            top_slots_end: next_slot,
            field_clash: None,
            shadowed_items: Vec::new(),
            checking_shadowed: false,
            builtin_types: builtin_types(names),
            future_name: names.lookup("Future"),
            present_name: names.lookup("Present"),
            list_name: names.lookup("List"),
            alias_site: None,
            call_site: None,
            element_types: Vec::new(),
            sorted_fields: Vec::new(),
            nodes: Vec::new(),
            built: Vec::new(),
            standing: Vec::new(),
            path_types: Vec::new(),
            pending: Vec::new(),
            callbacks: Vec::new(),
            callback_positions: Vec::new(),
            tests: HashMap::new(),
            quiet: Quiet::default(),
            declared_types: Vec::new(),
            copies: HashMap::new(),
            record: None,
            view_roots: None,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// The checker, recording what each name it reads refers to and what
    /// each binding it declares is, for an index of the input (see
    /// [Recording](#recording)): [`Checker::check_recorded`] returns it.
    #[must_use]
    pub(crate) fn recording(mut self) -> Self {
        self.record = Some(Box::default());
        self
    }

    /// The checker, taking a name that names nothing as no error, as in an
    /// input that sees a file with syntax errors (see
    /// [`Checker::unresolved`]): for a file checked for its index alone,
    /// which needs a file whose check failed and whose declarations it may
    /// read.
    #[must_use]
    pub(crate) fn blind(mut self) -> Self {
        self.quiet.sees_broken = true;
        self
    }

    /// Checks the file's body. On success, returns the tables, the
    /// outermost scope extended with the body's top-level declarations, the
    /// body's type and the warnings.
    ///
    /// # Errors
    ///
    /// As [`check`].
    pub fn check(self) -> Result<CheckOutput, Vec<Diagnostic>> {
        self.check_recorded().0
    }

    /// [`Checker::check`], with what it recorded if it was asked to
    /// ([`Checker::recording`]), whether it succeeded or failed (see
    /// [Recording](#recording)).
    pub(crate) fn check_recorded(
        mut self,
    ) -> (Result<CheckOutput, Vec<Diagnostic>>, Option<Box<Record>>) {
        let root = self.ast.root();
        // A tree with syntax errors is checked for the errors of what was
        // parsed, and is never lowered: its check always fails.
        self.quiet.failed |= self.ast.syntax() != Syntax::Clean;
        // A workspace has checked the file's `type` items already, before
        // any file's check ([`check_types`]): they are resolved quietly.
        self.check_declarations();
        self.declare_top_fns();
        let ty = self.check_body(root, None);
        // Finalisation: an integer result with no context defaults to Int.
        self.coerce_result(root, TyId::INT);
        let ty = if ty == TyId::UNTYPED_INT {
            TyId::INT
        } else {
            ty
        };
        let record = self.finish_record(ty);
        if !self.errors.is_empty() || self.quiet.failed {
            return (Err(diagnostic::merge(self.errors, self.warnings)), record);
        }
        // In a workspace whose scan found errors, which never runs, a read
        // of what the scan poisoned is erroneous, silently: the check fails
        // quietly (see [`crate::workspace`], "Scan errors").
        if self.workspace.is_some_and(Context::scan_failed) && !self.settled() {
            return (Err(diagnostic::merge(self.errors, self.warnings)), record);
        }
        if !self.settled() {
            let error = InternalError {
                message: "a type table entry is untyped, erroneous or provisional after a \
                          successful check",
                span: self.ast.root_span(),
            };
            return (Err(vec![error.into()]), record);
        }
        let scope_slot_names = self.scope_slot_names();
        let (cell_types, shared_children) = self.cell_tables();
        let known = self.known();
        let field_cells = self.tree_cells();
        let bindings = self.bindings.into_iter().map(Binding::forgotten).collect();
        let top_functions = self
            .top_fns
            .iter()
            .map(|top| (top.item, top.made))
            .collect();
        let field_schedule = FieldSchedule {
            tree: std::mem::take(&mut self.mentions.tree),
            schedule: std::mem::take(&mut self.schedule),
        };
        let checked = Checked {
            types: Types {
                others: OtherTypes {
                    declared: self.declared_types,
                    references_past: self.references_past,
                },
                ..Types::new(self.expr_types)
            },
            resolution: Resolution {
                expr_vars: self.expr_vars,
                awaits: self.awaits,
                item_slots: self.item_slots,
                item_spreads: self.item_spreads,
                spread_slots: self.spread_slots,
                item_paths: self.item_paths,
                path_awaits: self.path_awaits,
                top_functions,
                field_cells,
                own_records: self.own_records,
                shared_children,
                imported_records: self.imported_records,
                view_records: self.view_records,
                value_cell: self.value_cell,
                place: self
                    .workspace
                    .map(|workspace| workspace.place().clone())
                    .unwrap_or_default(),
                field_schedule,
                frame_size: self.frame_size,
                slot_names: self.slot_names,
                functions: self.fn_resolutions,
                arg_params: self.arg_params,
                for_slots: self.for_slots,
                jumps: self.jumps,
                loops: self.loop_resolutions,
                returned_blocks: self.returned_blocks,
                discarded: self.discarded,
                tests: self.tests,
                meta_reads: self.meta_reads,
                metas: self.metas,
                call_defaults: self.lowering.call_defaults,
                fn_attributes: self.lowering.fn_attributes,
                record_fills: self.lowering.record_fills,
            },
        };
        let mut warnings = self.warnings;
        warnings.sort_by_key(|warning| warning.span.start);
        let output = CheckOutput {
            checked,
            scope: Scope {
                bindings,
                slot_names: scope_slot_names,
                known,
            },
            ty,
            cell_types,
            warnings,
        };
        (Ok(output), record)
    }

    /// The cells of the nodes of the input's tree of top-level fields, by
    /// their indices there: the view's are in `view_records`.
    fn tree_cells(&self) -> Vec<Slot> {
        self.field_cells
            .iter()
            .take(self.mentions.tree.nodes.len())
            .map(|cell| cell.slot)
            .collect()
    }

    /// Whether, after a check that found no error, every expression has a
    /// type that is neither untyped, erroneous nor provisional, and every
    /// function's captures are settled: the invariant lowering relies on.
    /// A shadowed declaration's expressions are never lowered, and may read
    /// cells no file this one needs computes, whose types are erroneous
    /// (see [Shadowed declarations](#shadowed-declarations)): they are
    /// exempt.
    fn settled(&self) -> bool {
        let shadowed = self.shadowed_exprs();
        let types = &*self.types;
        let unsettled = |ty: TyId| types.contains_provisional(ty, None);
        let lowered = |at: usize| {
            !shadowed
                .iter()
                .any(|&(first, last)| (first..=last).contains(&at))
        };
        !self.expr_types.iter().enumerate().any(|(at, &ty)| {
            lowered(at)
                && (unsettled(ty) || matches!(types.kind(ty), TyKind::Error | TyKind::UntypedInt))
        }) && !self
            .fn_resolutions
            .iter()
            .any(|function| function.captures.iter().any(|&(_, ty)| unsettled(ty)))
    }

    /// Checks a nested block's body in a scope of its own, returning its
    /// type. With `expecting`, the block is a record block checked against
    /// an expected record type, or its result is checked against what is
    /// expected, as [`Checker::check_body`] says.
    fn check_block(&mut self, body: ItemRange, expecting: Option<Expecting<'_>>) -> TyId {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let outer_spreads = std::mem::take(&mut self.spreads);
        let nodes_start = self.nodes.len();
        let leaves_root = self.functions.is_empty() && self.root_end.is_none();
        if leaves_root {
            self.root_end = Some(self.bindings.len());
        }
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        let ty = self.check_body(body, expecting);
        if leaves_root {
            self.root_end = None;
        }
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        self.nodes.truncate(nodes_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        self.spreads = outer_spreads;
        ty
    }

    /// Checks the items of a body in the current scope, returning its type:
    /// for a record block, the record of its fields; otherwise the type of
    /// its result, the `= EXPR` or final expression statement, or `{}` if
    /// it has none. With `expecting`, a record block is checked against the
    /// expected record type, and a block that is not one has its result
    /// checked against the expected type, which it does only for a list
    /// type or a union (see [`Checker::check_list_expected`]), and has the
    /// type its result has once any await marked on it has run (see
    /// [`Checker::fit`]); or, for a record type with defaults that reaches
    /// only a record literal its result is ([`Expecting::literal_only`]),
    /// or only the elements of a list literal it is
    /// ([`Expecting::elements_only`]), has its result checked so
    /// ([`Checker::fill_result`], [`Checker::fill_elements`]), and the type
    /// its result has. A `break` or `continue` that is its last item
    /// makes it `Never`. The first item after one that never completes is
    /// warned about, "this is never run" (see [Never](#never)).
    fn check_body(&mut self, body: ItemRange, expecting: Option<Expecting<'_>>) -> TyId {
        let items = self.ast.items(body);
        let first_field = items.iter().copied().find(|&id| {
            matches!(
                self.ast.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        });
        let mut result = TyId::EMPTY;
        // Whether this is the input's outermost block, whose top-level
        // functions are order-free.
        let root = body == self.ast.root() && self.functions.is_empty();
        // The first item that never completes, and whether the item after
        // it has been warned about.
        let (mut leaves, mut warned) = (None, false);
        // Whether an item is one the parser could not parse.
        let mut broken = false;
        for (index, &id) in items.iter().enumerate() {
            let next = items.get(index + 1).copied();
            let item = *self.ast.item(id);
            if root {
                self.root_position = index;
                self.compute_fields(index);
                if self.set_aside_shadowed(index, id) || self.reach_top_fn(id) {
                    continue;
                }
                self.check_mentioned(index);
                self.check_uses(index);
                self.check_clash(id);
            }
            // A top-level field computed before its own item, because an
            // item above needed it, does not run here, nor does a `type`:
            // neither is warned about nor counted as never completing.
            let early = root && self.runs_elsewhere(index, item.kind);
            if let (Some(leaves), false, false) = (leaves, warned, early) {
                self.warnings.push(
                    Diagnostic::warning("this is never run", item.span)
                        .with_context(leaves, "this always leaves the block"),
                );
                warned = true;
            }
            // The result of a block that is not a record block, checked
            // against what is expected of the block.
            let result_expected = expecting.filter(|_| next.is_none() && first_field.is_none());
            match item.kind {
                ItemKind::Let {
                    path,
                    ty,
                    value,
                    mutable,
                    ..
                } => {
                    // TEMPORARY (alias rule): a top-level `let` of one name, neither annotated nor
                    // mutable, may bind a built-in function whose typing is special (see the
                    // [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
                    let alias = root && !mutable && ty.is_none() && self.ast.path(path).len() == 1;
                    let outer = alias.then(|| self.alias_site.replace(value));
                    self.check_let(id, path, ty, (value, mutable));
                    if let Some(outer) = outer {
                        self.alias_site = outer;
                    }
                }
                ItemKind::Rebind { path, value } => self.check_rebind(id, path, value),
                ItemKind::Field {
                    path, ty, value, ..
                } => {
                    self.check_field(id, path, (ty, value), expecting);
                }
                ItemKind::Spread(value) => self.check_spread_item(id, value, root),
                ItemKind::Import(_) => self.check_import(id, item.span),
                // A definition, and a meta, is placed as the walk reaches it,
                // whether or not it is a top-level function
                // (`Checker::reach_top_fn`).
                ItemKind::Type { .. } | ItemKind::DefTypeDef { .. } | ItemKind::DefMeta { .. } => {}
                ItemKind::Result(value) => {
                    let ty = self.expression_item(value, true, result_expected);
                    if self.result_item(item.span, (value, ty), (first_field, next)) {
                        result = ty;
                    }
                }
                ItemKind::Expr(value) => {
                    // In a record block, even the final expression is a
                    // statement.
                    let is_result = next.is_none() && first_field.is_none();
                    let ty = self.expression_item(value, is_result, result_expected);
                    if is_result {
                        result = ty;
                    } else {
                        self.check_statement(value, ty);
                    }
                }
                ItemKind::Break(jump) | ItemKind::Continue(jump) => {
                    self.check_jump((id, item), jump, first_field);
                    if next.is_none() && first_field.is_none() {
                        result = self.left_type(id);
                    }
                }
                ItemKind::Return(jump) => {
                    self.check_return((id, item), jump, first_field);
                    if next.is_none() && first_field.is_none() {
                        result = self.left_type(id);
                    }
                }
                // What the parser could not parse: the block's result or
                // fields are unknown.
                ItemKind::Error => broken = true,
            }
            if leaves.is_none() && !early && self.never_completes(id, item.kind) {
                leaves = Some(item.span);
            }
        }
        if root {
            self.finish_top_level(items.len());
        }
        self.body_type(first_field.is_some(), (result, broken), expecting)
    }

    /// The type of a body, a record block if `record`, whose result's type
    /// is `result`, as [`Checker::check_body`] gives it: `Error` if it has
    /// an error item (`broken`), whose fields' and result's types are
    /// unknown, nothing then being expected of its record.
    fn body_type(
        &mut self,
        record: bool,
        (result, broken): (TyId, bool),
        expecting: Option<Expecting<'_>>,
    ) -> TyId {
        match (record, broken) {
            (true, true) => {
                self.record_type(None);
                TyId::ERROR
            }
            (true, false) => self.record_type(expecting),
            (false, true) => TyId::ERROR,
            (false, false) => result,
        }
    }

    /// After the walk of the outermost block, whose items are `count`:
    /// computes the top-level fields left, checks the bodies left, settles
    /// the types of the open nodes, and checks the shadowed declarations.
    fn finish_top_level(&mut self, count: usize) {
        self.root_position = count;
        self.compute_fields(count);
        self.check_remaining_bodies();
        self.settle_top_nodes();
        self.check_shadowed_items();
    }

    /// When the walk of the outermost block reaches its item `id`, at
    /// `position`: if a later declaration shadows it, sets it aside, with
    /// the bindings declared so far, to be checked once the walk is done
    /// ([`Checker::check_shadowed_items`]), and returns `true`.
    fn set_aside_shadowed(&mut self, position: usize, id: ItemId) -> bool {
        let shadowed = self.mentions.shadowed.get(position) == Some(&true);
        if shadowed {
            self.shadowed_items.push((id, self.bindings.len()));
        }
        shadowed
    }

    /// After the walk of the outermost block: checks each shadowed
    /// declaration's value as it is written, in the view of the top level
    /// above its own item, as a body checked late is (see [Shadowed
    /// declarations](#shadowed-declarations)), declaring nothing.
    fn check_shadowed_items(&mut self) {
        let ast = self.ast;
        for (id, cut) in std::mem::take(&mut self.shadowed_items) {
            let (path, ty, value) = match ast.item(id).kind {
                ItemKind::Field {
                    path, ty, value, ..
                } => (Some(path), ty, value),
                // A definition made with a def-type has no name to see
                // itself by.
                ItemKind::DefTypeDef { value, .. } => (None, None, value),
                _ => continue,
            };
            let cut = cut.max(self.top_end).min(self.bindings.len());
            let below = self.bindings.split_off(cut);
            let outer = std::mem::replace(&mut self.checking_shadowed, true);
            if let Some(path) = path {
                self.name_callee(ast.path(path), value);
            }
            self.annotated_value(ty, value);
            self.callee_name = None;
            self.checking_shadowed = outer;
            self.bindings.extend(below);
        }
    }

    /// The expressions of the outermost block's shadowed declarations, as
    /// ranges of ids: never lowered, they may hold erroneous types without
    /// an error, where they read a cell no file they need computes.
    fn shadowed_exprs(&self) -> Vec<(usize, usize)> {
        let root = self.ast.items(self.ast.root());
        root.iter()
            .enumerate()
            .filter(|&(position, _)| self.mentions.shadowed.get(position) == Some(&true))
            .filter_map(|(_, &id)| match self.ast.item(id).kind {
                ItemKind::Field { value, .. } => {
                    let first = self.ast.subtree(value).next()?;
                    Some((first.index(), value.index()))
                }
                // A definition's `DT` comes just before its function.
                ItemKind::DefTypeDef { def, value } => {
                    let first = self.ast.subtree(self.ast.def_type(def).dt).next()?;
                    Some((first.index(), value.index()))
                }
                _ => None,
            })
            .collect()
    }

    /// Checks the placement of the result item spanning `span`, `= VALUE`,
    /// `value` being of type `ty`, in a block whose first field, if it has
    /// one, is `first_field`, and whose item after it, if any, is `next`:
    /// it cannot be in a record block, and it must be the last item.
    /// Returns whether it is the block's result; otherwise, its value is
    /// settled and the error reported.
    fn result_item(
        &mut self,
        span: Span,
        (value, ty): (ExprId, TyId),
        (first_field, next): (Option<ItemId>, Option<ItemId>),
    ) -> bool {
        let eq = Span::new(span.start, span.start + 1);
        if let Some(first_field) = first_field {
            self.settle(value, ty);
            self.errors.push(
                Diagnostic::error("a block with fields cannot have a result", eq)
                    .with_context(self.ast.item(first_field).span, "first field declared here")
                    .with_note(
                        "a block is either a record of its fields or a single value; bind the \
                         value with `let` or make it a field",
                    ),
            );
            return false;
        }
        let Some(next) = next else {
            return true;
        };
        self.settle(value, ty);
        self.errors.push(
            Diagnostic::error("the result must be the last item in its block", eq)
                .with_context(self.ast.item(next).span, "this item comes after it"),
        );
        false
    }

    /// Checks the expression item `value`, or the value of a result item,
    /// `= VALUE`, which is its block's result if `is_result`, and a
    /// statement otherwise, against what is expected of the block's result,
    /// if anything is, and returns its type.
    fn expression_item(
        &mut self,
        value: ExprId,
        is_result: bool,
        expected: Option<Expecting<'_>>,
    ) -> TyId {
        match expected {
            Some(expected) => {
                let context = (expected.context, expected.why);
                match expected.reach {
                    Reach::Whole => self.fit(value, expected.ty, context),
                    Reach::Literal => self.fill_result(value, expected.ty, context),
                    Reach::Elements => self.fill_elements(value, expected.ty, context),
                }
            }
            None if !is_result && self.is_empty_list(value) => {
                // A statement `[]` holds nothing, and is the unit-like
                // `List<{}>`.
                let ty = self.types.list(TyId::EMPTY);
                self.expr_types[value.index()] = ty;
                ty
            }
            None => self.check_expr(value),
        }
    }

    /// The type of a block whose last item is `id`, a `break`, a `continue`
    /// or a `return`, checked: `Never`, or erroneous if the item is an
    /// error and so has nothing to leave for, so that nothing more is
    /// reported of the block.
    fn left_type(&self, id: ItemId) -> TyId {
        if self.jumps[id.index()].is_some() {
            self.never
        } else {
            TyId::ERROR
        }
    }

    /// Checks the import item `id`, spanning `span` (see the parser's
    /// [Imports](crate::parser#imports)). Checked with no workspace at all,
    /// as [`check`] checks a file alone, there is nothing for its path to
    /// name, which is an error at the import, reported here with the
    /// input's other errors. A lone file and a session's input are checked
    /// beside the standard library, whose modules the scan has resolved; a
    /// path of anything else is the error, "`lib` is not a built-in or
    /// standard module, and there is no `workspace.gcsl` above this file".
    /// In a workspace the scan has resolved it, and its names are read
    /// where they are used (see [Imports](#imports)): the item itself
    /// checks nothing.
    fn check_import(&mut self, id: ItemId, span: Span) {
        let Some(workspace) = self.workspace else {
            self.errors.push(Diagnostic::error(
                "`import` needs a workspace: no `workspace.gcsl` above this file",
                span,
            ));
            return;
        };
        if !workspace.needs_workspace(id) {
            return;
        }
        let ItemKind::Import(decl) = self.ast.item(id).kind else {
            return;
        };
        let names = self.ast.names();
        let segments = self.ast.path(self.ast.import_decl(decl).path);
        let path: Vec<&str> = segments
            .iter()
            .filter_map(|segment| Some(names.get(segment.name()?)))
            .collect();
        // A module of one name may be a standard one misspelled.
        let suggestion = match (segments, path.as_slice()) {
            ([segment], [text]) => {
                let modules = crate::STANDARD
                    .iter()
                    .map(|&(stem, _)| stem)
                    .chain([crate::workspace::BUILTINS, Fileroot::MODULE])
                    .map(|module| (module, 0));
                suggest::best(text, modules).map(|best| (segment.span, best.to_owned()))
            }
            _ => None,
        };
        self.errors.push(
            Diagnostic::error(
                format!(
                    "`{}` is not a built-in or standard module, and there is no \
                     `workspace.gcsl` above this file",
                    path.join("/")
                ),
                span,
            )
            .with_suggestion(suggestion),
        );
    }

    /// Resolves the file's `type` items, each's type with every bare type
    /// present, recording what each stands for (see [Type
    /// declarations](#type-declarations)): reporting what is wrong in each,
    /// with "`Int` is a built-in type" for one named like a built-in type
    /// ([`BUILTIN_TYPE_NAMES`]) and "type `Foo` is declared twice" at a
    /// second of one name, labelled "first declared here", unless resolving
    /// quietly, as a workspace's check of a file does, the workspace having
    /// checked them first ([`check_types`]).
    fn type_items(&mut self) {
        let ast = self.ast;
        let mut declared: Vec<(Name, Span)> = Vec::new();
        for &id in ast.items(ast.root()) {
            let ItemKind::Type {
                name,
                name_span,
                ty,
            } = ast.item(id).kind
            else {
                continue;
            };
            let text = ast.names().get(name);
            let first = declared
                .iter()
                .find(|&&(earlier, _)| earlier == name)
                .map(|&(_, span)| span);
            let error = if BUILTIN_TYPE_NAMES.contains(&text) {
                Some(Diagnostic::error(
                    format!("`{text}` is a built-in type"),
                    name_span,
                ))
            } else if let Some(first) = first {
                Some(
                    Diagnostic::error(format!("type `{text}` is declared twice"), name_span)
                        .with_context(first, "first declared here"),
                )
            } else {
                declared.push((name, name_span));
                None
            };
            if let Some(error) = error {
                if self.quiet.types {
                    self.quiet.failed = true;
                } else {
                    self.errors.push(error);
                }
            }
            let errors = self.errors.len();
            let resolved = self.resolve_type(ty, Bareness::Present);
            if ast.is_attribute(id)
                && resolved != TyId::ERROR
                && !self.holds_constants(resolved, true)
            {
                self.errors.push(
                    Diagnostic::error(NOT_CONSTANT_RECORD, ast.type_expr(ty).span)
                        .with_note(CONSTANT_FIELDS_NOTE),
                );
            }
            if self.quiet.types && self.errors.len() > errors {
                self.errors.truncate(errors);
                self.quiet.failed = true;
            }
            if resolved != TyId::ERROR {
                self.declared_types.push((id, resolved));
            }
        }
    }

    /// Whether the item `id`, of `kind`, checked, never completes, so that
    /// no item after it in its block runs: a `break`, a `continue` or a
    /// `return`, but one that is an error and so has nothing to leave for,
    /// and an expression item, a result, a `let` or a `:=` whose value is
    /// of type `Never`.
    fn never_completes(&self, id: ItemId, kind: ItemKind) -> bool {
        match kind {
            ItemKind::Break(_) | ItemKind::Continue(_) | ItemKind::Return(_) => {
                self.jumps[id.index()].is_some()
            }
            ItemKind::Expr(value)
            | ItemKind::Result(value)
            | ItemKind::Let { value, .. }
            | ItemKind::Rebind { value, .. } => self.expr_types[value.index()] == self.never,
            ItemKind::Field { .. }
            | ItemKind::Spread(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::DefTypeDef { .. }
            | ItemKind::DefMeta { .. }
            | ItemKind::Error => false,
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

    /// Whether `value` is directly the empty list literal, `[]`.
    fn is_empty_list(&self, value: ExprId) -> bool {
        matches!(
            self.ast.get(value).kind,
            ExprKind::List(elements) if self.ast.elements(elements).is_empty()
        )
    }

    /// Applies the statement rule to `value`, an expression item that is
    /// not its block's result, of type `ty` (see the module documentation).
    /// A bare `await` is run for its effects, and its value discarded,
    /// whatever it is. Otherwise a statement is a position expecting a
    /// unit-like value (see [`TypeTable::is_unit_like`]): one that is
    /// unit-like as it is needs nothing; one that is unit-like once awaited,
    /// at the top as a `Future<{}>` is or deeply as a `List<Future<{}>>`
    /// is, is awaited here, so that its effects are not lost, subject to
    /// the purity rule, a parameter of this function being measured. A call
    /// of a function that is not pure, outside a pure function, is run for
    /// its effects, as a bare `await` is: it is awaited deeply, so that no
    /// effect it defers is lost, and its value discarded, whatever it is.
    /// Any other is an unused value.
    fn check_statement(&mut self, value: ExprId, ty: TyId) {
        if self.is_await(value) {
            self.settle(value, ty);
            return;
        }
        if ty == TyId::ERROR || self.types.is_unit_like(ty) {
            return;
        }
        if self.types.strip(ty) == TyId::EMPTY {
            self.measure(value, ty);
            return;
        }
        let present = self.types.present(ty);
        if self.types.is_unit_like(present) {
            self.measure_deep(value, ty);
        } else if self.impure_calls.contains(&value)
            && self
                .functions
                .last()
                .is_none_or(|function| function.effect != Effect::Pure)
        {
            // In a pure function the call is an error already.
            self.measure_deep(value, ty);
            self.discarded.insert(value);
        } else {
            self.unused_value(value, ty);
        }
    }

    /// Reports the statement `value`, of type `ty`, which is neither `{}`
    /// nor erroneous, as an unused value.
    fn unused_value(&mut self, value: ExprId, ty: TyId) {
        let ty = self.settle(value, ty);
        let shown = self.in_message(ty);
        let resolved = self.shown(ty);
        let note = if self.types.laziness(resolved) == Laziness::Present {
            "make it the block's result with `= ...`, bind it with `let`, or remove it"
        } else {
            "await it with `await`, bind it with `let`, or remove it"
        };
        self.errors.push(
            Diagnostic::error(
                format!("unused value of type {shown}"),
                self.ast.get(value).span,
            )
            .with_note(note),
        );
    }

    /// Checks `let PATH (: TYPE)? = VALUE` and declares the variable: with
    /// the value's type, or for a dotted path the type of the update. A
    /// `let mut` (`mutable`), whose path is a single name, is checked in
    /// the same way and declares a mutable variable, which remembers no
    /// function expression, since it may come to hold another function (see
    /// the module documentation).
    fn check_let(
        &mut self,
        id: ItemId,
        path: PathRange,
        annotation: Option<TypeExprId>,
        (value, mutable): (ExprId, bool),
    ) {
        let expected = annotation.map(|annotation| {
            (
                self.resolve_type(annotation, Bareness::Present),
                self.ast.type_expr(annotation).span,
            )
        });
        let segments = self.ast.path(path);
        self.name_callee(segments, value);
        let ty = match expected {
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
            Some((expected, context)) => {
                let found = self.check_expr_expecting(
                    value,
                    expected,
                    context,
                    "expected because of this annotation",
                );
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
        };
        self.callee_name = None;
        match segments {
            [variable] => {
                // A `let` path's segments are all names.
                let Some(name) = variable.name() else {
                    return;
                };
                if mutable {
                    self.declare(name, ty, id, BindingKind::Mutable);
                } else {
                    let function = self.direct_function(segments, value);
                    self.declare_initialised(name, ty, id, (BindingKind::Variable, function));
                }
            }
            segments => self.declare_update(id, segments, ty),
        }
    }

    /// The function expression that the initialiser `value` of a `let` or
    /// field whose path is `segments` directly is, if the path is a single
    /// name: the same condition as [`Checker::name_callee`]'s.
    fn direct_function(&self, segments: &[PathSegment], value: ExprId) -> Option<FnId> {
        match (segments, self.ast.get(value).kind) {
            ([_], ExprKind::Fn(function)) => Some(function),
            _ => None,
        }
    }

    /// Before the initialiser `value` of a `let` or field whose path is
    /// `segments` is checked: if the path is a single name and `value` is
    /// directly a function expression, lets that function see itself by
    /// the name (see the module documentation). The caller clears the name
    /// afterwards, in case the function did not take it.
    fn name_callee(&mut self, segments: &[PathSegment], value: ExprId) {
        self.callee_name = match (segments, self.ast.get(value).kind) {
            ([single], ExprKind::Fn(_)) => single.name(),
            _ => None,
        };
    }

    /// Checks the field item `id`, `PATH = VALUE`, and declares or extends
    /// the field. With `expecting`, its block is checked against an
    /// expected record type: a field the type has is checked against that
    /// field's type and then has it, and one the type lacks is a warning.
    /// A single name that is a soft field of the block overrides it (see
    /// the module documentation): the field keeps its place and its slot,
    /// is hard from then on, and the name is bound anew. An annotated field,
    /// `PATH: TYPE = VALUE`, is checked against its annotation, as an
    /// annotated `let` is, and has its type (see [Records](#records)).
    fn check_field(
        &mut self,
        id: ItemId,
        path: PathRange,
        (annotation, value): (Option<TypeExprId>, ExprId),
        expecting: Option<Expecting<'_>>,
    ) {
        let segments = self.ast.path(path);
        let [field] = segments else {
            self.check_dotted_field(id, segments, (annotation, value), expecting);
            return;
        };
        // A field's path's segments are all names.
        let Some(name) = field.name() else {
            self.check_alone(value);
            return;
        };
        let name_span = field.span;
        // The soft field of the name that this overrides, if any; only a
        // block with a spread can have one.
        let soft = match self.spreads {
            BlockSpreads::None => None,
            BlockSpreads::Some | BlockSpreads::Rejected => self
                .block_field(name)
                .filter(|&index| self.fields[index].spread.is_some()),
        };
        // A top-level field of the input: its leaf was computed at its
        // point, and its root declared as the input started.
        if let Some(ty) = self.computed_value(value) {
            self.reach_top_field(id, (name, name_span), ty, soft);
            return;
        }
        let expected =
            expecting.map(|expecting| (expecting, self.types.field_or_rest(expecting.ty, name)));
        self.name_callee(segments, value);
        let ty = match expected {
            _ if annotation.is_some() => {
                let ty = self.annotated_value(annotation, value);
                if let Some((expecting, expected)) = expected {
                    let named = (name, name_span);
                    self.annotated_against(annotation, ty, (expecting, expected), named);
                }
                ty
            }
            Some((expecting, Some(expected))) => {
                let found =
                    self.check_expr_expecting(value, expected, expecting.context, expecting.why);
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
            Some((expecting, None)) => {
                let found = self.check_expr(value);
                let ty = self.settle(value, found);
                let text = self.ast.names().get(name);
                let record = self.in_message(expecting.ty);
                self.warnings.push(
                    Diagnostic::warning(
                        format!("field `{}` is not in {record}", FieldName(text)),
                        name_span,
                    )
                    .with_context(expecting.context, expecting.why)
                    .with_note(EXTRA_FIELD_NOTE),
                );
                ty
            }
            None => {
                let found = self.check_expr(value);
                self.settle(value, found)
            }
        };
        self.callee_name = None;
        let function = self.direct_function(segments, value);
        let Some(index) = soft else {
            self.declare_initialised(name, ty, id, (BindingKind::Field, function));
            self.fields.push(FieldDecl {
                name,
                name_span,
                ty,
                slot: self.item_slots[id.index()],
                open: None,
                spread: None,
                rejected: false,
            });
            return;
        };
        // The field keeps its place and its slot, and is hard from now on;
        // the name is bound anew, as a `let` shadows.
        let slot = self.fields[index].slot;
        self.bind(Binding {
            name,
            slot,
            ty,
            declared: Some(self.ast.item(id).span),
            kind: BindingKind::Field,
            function,
            callback: None,
        });
        self.item_slots[id.index()] = slot;
        let field = &mut self.fields[index];
        field.ty = ty;
        field.name_span = name_span;
        field.spread = None;
    }

    /// The type of the leaf of a dotted field item of an inner block, whose
    /// value is `value`, annotated `annotation` if it is, and whose last
    /// segment is `last`, its block checked against an expected record type
    /// that has the type `expected` at the item's path if it does: the
    /// annotation's, which applies to the last segment, as a dotted `let`'s
    /// does, the value checked against it ([`Checker::annotated_value`]);
    /// else the expected type, the value checked against it; else the
    /// value's.
    fn dotted_leaf(
        &mut self,
        (annotation, value): (Option<TypeExprId>, ExprId),
        expected: Option<(Expecting<'_>, TyId)>,
        last: Option<&PathSegment>,
    ) -> TyId {
        if annotation.is_some() {
            let ty = self.annotated_value(annotation, value);
            if let (Some((expecting, expected)), Some(last)) = (expected, last)
                && let Some(name) = last.name()
            {
                let named = (name, last.span);
                self.annotated_against(annotation, ty, (expecting, Some(expected)), named);
            }
            return ty;
        }
        let Some((expecting, expected)) = expected else {
            return self.check_alone(value);
        };
        let found = self.check_expr_expecting(value, expected, expecting.context, expecting.why);
        if found == TyId::ERROR {
            TyId::ERROR
        } else {
            expected
        }
    }

    /// Checks `value`, the value of a field annotated `annotation`, against
    /// the annotation exactly as an annotated `let`'s value is checked
    /// against its own ([`Checker::check_let`]): the annotation resolved
    /// with every bare type present, the value checked against it, a
    /// literal adapting, a record block field by field, and a value that
    /// fits once awaited awaited, subject to the purity rule, a parameter
    /// of a pure function being measured. Returns the field's type: the
    /// annotation's, not the value's, or `Error`. Without an annotation, the
    /// value checked on its own, and its type.
    fn annotated_value(&mut self, annotation: Option<TypeExprId>, value: ExprId) -> TyId {
        let Some(annotation) = annotation else {
            return self.check_alone(value);
        };
        let expected = self.resolve_type(annotation, Bareness::Present);
        let context = self.ast.type_expr(annotation).span;
        let found = self.check_expr_expecting(
            value,
            expected,
            context,
            "expected because of this annotation",
        );
        if found == TyId::ERROR {
            TyId::ERROR
        } else {
            expected
        }
    }

    /// Where a field annotated `annotation`, and so of type `ty`, is an item
    /// of a record block checked against an expected record type,
    /// `expecting`, whose type for the field is `expected`, if it has one:
    /// a field whose type is not a subtype of that is "mismatched types",
    /// at the annotation, and one the expected type lacks, `name` written
    /// at `name_span`, is the warning that any field it lacks is.
    fn annotated_against(
        &mut self,
        annotation: Option<TypeExprId>,
        ty: TyId,
        (expecting, expected): (Expecting<'_>, Option<TyId>),
        (name, name_span): (Name, Span),
    ) {
        let Some(annotation) = annotation else {
            return;
        };
        let Some(expected) = expected else {
            let text = FieldName(self.ast.names().get(name)).to_string();
            let record = self.in_message(expecting.ty);
            self.warnings.push(
                Diagnostic::warning(format!("field `{text}` is not in {record}"), name_span)
                    .with_context(expecting.context, expecting.why)
                    .with_note(EXTRA_FIELD_NOTE),
            );
            return;
        };
        if ty == TyId::ERROR || expected == TyId::ERROR || self.types.is_subtype(ty, expected) {
            return;
        }
        let label = format!(
            "expected {}, found {}",
            self.in_label(expected),
            self.in_label(ty)
        );
        self.errors.push(
            Diagnostic::error("mismatched types", self.ast.type_expr(annotation).span)
                .with_label(label)
                .with_context(expecting.context, expecting.why),
        );
    }

    /// When the walk reaches the item `id` of a top-level field whose path
    /// is the single name `name`, written at `name_span`, and whose value
    /// has the type `ty`, computed at its point: gives the item its root's
    /// cell and adds the field to the record of the input, or makes the
    /// soft field `soft` of its name hard, keeping its place. Nothing is
    /// declared: the root's binding was, as the input started.
    fn reach_top_field(
        &mut self,
        id: ItemId,
        (name, name_span): (Name, Span),
        ty: TyId,
        soft: Option<usize>,
    ) {
        let slot = self
            .mentions
            .tree
            .root(name)
            .and_then(|root| self.field_cells.get(root as usize))
            .map_or(Slot::NONE, |cell| cell.slot);
        self.item_slots[id.index()] = slot;
        match soft {
            Some(index) => {
                let field = &mut self.fields[index];
                (field.ty, field.slot, field.name_span) = (ty, slot, name_span);
                field.spread = None;
            }
            None => self.fields.push(FieldDecl {
                name,
                name_span,
                ty,
                slot,
                open: None,
                spread: None,
                rejected: false,
            }),
        }
    }

    /// The index in `fields` of the innermost block's field named `name`,
    /// by its first declaration, if it has one.
    fn block_field(&self, name: Name) -> Option<usize> {
        self.fields[self.fields_start..]
            .iter()
            .position(|field| field.name == name)
            .map(|index| index + self.fields_start)
    }

    /// Whether `ty`, the type of a spread item's operand, written at
    /// `operand`, has a rest-field, which is reported at the item, at
    /// `item_span`: a spread defines the fields its type names, and those a
    /// rest-field covers are known only when it runs.
    fn rest_spread(&mut self, ty: TyId, item_span: Span, operand: Span) -> bool {
        if self.types.record_rest(ty).is_none() {
            return false;
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        self.errors.push(
            Diagnostic::error(
                format!("cannot spread a value of type {shown}: it has a rest-field"),
                item_span,
            )
            .with_context(operand, label)
            .with_note(SPREAD_REST_NOTE),
        );
        true
    }

    /// Checks the spread item `id`, `...VALUE` (see the module
    /// documentation). Its operand is checked on its own and measured at
    /// the top (see [`Checker::measure`]), and must then be a record:
    /// "cannot spread a value of type T: it is not a record" otherwise, and
    /// one whose type has no rest-field, "cannot spread a value of type T:
    /// it has a rest-field" otherwise, since a spread defines the fields
    /// its type names, and the fields a rest-field covers are known only at
    /// runtime.
    /// Each field of the record's type, in the order the type table holds
    /// them, by name, is then softly assigned: declared as a soft field of
    /// the block, with the type the record's type gives it, and bound anew
    /// as a name in scope, in the field's one slot if it is a soft field
    /// already, which this overrides, keeping its place, and in the next
    /// free slot otherwise. A hard field of one of the names is not
    /// replaced, but reported: "field `x` is defined twice", at the
    /// operand, once for the spread, naming the first such field and
    /// counting the others. A spread rejected either way makes its block's
    /// record erroneous. The item's own slot, which holds the operand's
    /// value while its fields are read, is the next free slot once its
    /// fields are declared, and is left free.
    fn check_spread_item(&mut self, id: ItemId, value: ExprId, top: bool) {
        let found = self.check_expr(value);
        let found = self.settle(value, found);
        let ty = self.measure(value, found);
        if self.spreads == BlockSpreads::None {
            self.spreads = BlockSpreads::Some;
        }
        let item_span = self.ast.item(id).span;
        let operand = self.ast.get(value).span;
        // An erroneous operand has been reported already.
        if ty == TyId::ERROR || self.rest_spread(ty, item_span, operand) {
            self.spreads = BlockSpreads::Rejected;
            return;
        }
        // An operand of type `Never` never gives a record: it spreads
        // nothing, and nothing after it runs.
        let fields = if ty == self.never {
            Some(&[][..])
        } else {
            self.types.record_fields(ty)
        };
        let Some(fields) = fields else {
            let shown = self.in_message(ty);
            let label = format!("this is {}", self.in_label(ty));
            self.errors.push(
                Diagnostic::error(
                    format!("cannot spread a value of type {shown}: it is not a record"),
                    item_span,
                )
                .with_context(operand, label),
            );
            self.spreads = BlockSpreads::Rejected;
            return;
        };
        let base = self.sorted_fields.len();
        self.sorted_fields
            .extend(fields.iter().map(|&(name, ty)| (name, 0, ty)));
        let end = self.sorted_fields.len();
        let start = self.spread_slots.len();
        // The first field defined before the spread, hard, with its type in
        // the operand and its declaration, and the number of others.
        let mut conflict: Option<(Name, TyId, FieldDecl)> = None;
        let mut others = 0_usize;
        for at in base..end {
            let Some(&(name, _, ty)) = self.sorted_fields.get(at) else {
                continue;
            };
            // A top-level declaration of the input overrides the spread's
            // field whichever is written first: the spread defines nothing
            // and writes no slot for its name. A spread above it holds the
            // name's place in the record for it, as a soft field would.
            if top && self.spread_over_top_field((name, ty), (value, operand)) {
                continue;
            }
            let slot = match self.block_field(name) {
                None => {
                    let slot = self.declare_binding(name, ty, Some(item_span), BindingKind::Field);
                    self.fields.push(FieldDecl {
                        name,
                        name_span: operand,
                        ty,
                        slot,
                        open: None,
                        spread: Some(value),
                        rejected: false,
                    });
                    slot
                }
                Some(index) if self.fields[index].spread.is_some() => {
                    let slot = self.fields[index].slot;
                    self.bind(Binding {
                        name,
                        slot,
                        ty,
                        declared: Some(item_span),
                        kind: BindingKind::Field,
                        function: None,
                        callback: None,
                    });
                    let field = &mut self.fields[index];
                    field.ty = ty;
                    field.name_span = operand;
                    field.spread = Some(value);
                    slot
                }
                Some(index) => {
                    match conflict {
                        None => conflict = Some((name, ty, self.fields[index])),
                        Some(_) => others += 1,
                    }
                    Slot::NONE
                }
            };
            self.spread_slots.push(slot);
        }
        self.sorted_fields.truncate(base);
        self.item_spreads[id.index()] = (
            position(start),
            position(self.spread_slots.len().saturating_sub(start)),
        );
        self.item_slots[id.index()] = Slot(self.next_slot);
        self.frame_size = self.frame_size.max(self.next_slot.saturating_add(1));
        if let Some(conflict) = conflict {
            self.report_spread_conflict(operand, conflict, others);
            self.spreads = BlockSpreads::Rejected;
        }
    }

    /// Reports that the spread whose operand is at `operand` has a field
    /// named `name`, of type `ty`, that the hard field `hard` was defined as
    /// before it, and `others` more such fields: "field `x` is defined
    /// twice", once for the spread (see the module documentation).
    fn report_spread_conflict(
        &mut self,
        operand: Span,
        (name, ty, hard): (Name, TyId, FieldDecl),
        others: usize,
    ) {
        let text = FieldName(self.ast.names().get(name)).to_string();
        let note = spread_first_note(others);
        let label = format!("`{text}` is a field of this, of type {}", self.in_label(ty));
        self.errors.push(
            Diagnostic::error(format!("field `{text}` is defined twice"), operand)
                .with_label(label)
                .with_context(hard.name_span, "first defined here")
                .with_note(note),
        );
    }

    /// Checks the dotted field item `id`, `a.b.c = VALUE`, whose path is
    /// `segments`, and declares the open field `a` or extends it (see the
    /// module documentation). A path that breaks the rules is reported, and
    /// its value checked on its own, without declaring anything.
    fn check_dotted_field(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (annotation, value): (Option<TypeExprId>, ExprId),
        expecting: Option<Expecting<'_>>,
    ) {
        // A field's path's segments are all names.
        let Some((first, first_name)) = segments
            .first()
            .and_then(|first| Some((first, first.name()?)))
        else {
            self.check_alone(value);
            return;
        };
        // Where the path attaches to what the block already has: the field's
        // declaration and the deepest existing node along the path, with the
        // number of segments that exist.
        let declared = self.block_field(first_name);
        let attach = match declared {
            None => None,
            Some(index) => {
                let field = self.fields[index];
                let Some(open) = field.open else {
                    let shown = self.ast.show_path(&segments[..1]);
                    self.errors.push(
                        Diagnostic::error(
                            format!("field `{shown}` is already defined"),
                            path_span(segments),
                        )
                        .with_context(field.name_span, "defined whole here")
                        .with_note(WHOLE_FIELD_NOTE),
                    );
                    self.fields[index].rejected = true;
                    self.check_rejected(value);
                    return;
                };
                let Some((node, existing)) = self.attach_point(open.node, segments) else {
                    self.fields[index].rejected = true;
                    self.check_rejected(value);
                    return;
                };
                Some((index, open, node, existing))
            }
        };
        let existing = attach.map_or(0, |(_, _, _, existing)| existing);
        let expected = expecting.and_then(|expecting| {
            self.expected_at(expecting, segments, existing)
                .map(|ty| (expecting, ty))
        });
        // A top-level field's leaf was computed at its point.
        let computed = self.computed_value(value);
        let leaf_ty = match computed {
            Some(ty) => ty,
            None => self.dotted_leaf((annotation, value), expected, segments.last()),
        };
        // The new nodes, from the first segment that does not exist yet.
        let (parent, root) = if let Some((_, open, node, _)) = attach {
            (node, open.node)
        } else {
            let root = self.add_node(NO_NODE, first_name, first.span, NodeKind::Open);
            (root, root)
        };
        let parent = self.add_path_nodes(parent, segments, existing.max(1));
        // `parent` is now the leaf. Its type is the value's, and the type of
        // each node above it changes: at once in an inner block, whose later
        // items see the record built so far, and once the block ends for a
        // top-level field, whose readers see its cells instead
        // (`Checker::settle_top_nodes`), so that a root of N dotted items
        // costs N steps, not N^2.
        self.nodes[parent as usize].ty = leaf_ty;
        let top_root = computed.and_then(|_| self.top_root_cell(first_name));
        if top_root.is_none() {
            let mut node = self.nodes[parent as usize].parent;
            while node != NO_NODE {
                let ty = self.node_type(node);
                let entry = &mut self.nodes[node as usize];
                entry.ty = ty;
                node = entry.parent;
            }
        }
        let ty = self.nodes[root as usize].ty;
        match attach {
            None => {
                let binding = self.open_binding(top_root, (first_name, ty), id);
                self.fields.push(FieldDecl {
                    name: first_name,
                    name_span: first.span,
                    ty,
                    slot: self.item_slots[id.index()],
                    open: Some(OpenField {
                        node: root,
                        binding,
                    }),
                    spread: None,
                    rejected: false,
                });
            }
            Some((index, open, _, existing)) => {
                self.fields[index].ty = ty;
                let binding = &mut self.bindings[open.binding];
                // A top-level root's binding has the whole root's type.
                if top_root.is_none() {
                    binding.ty = ty;
                }
                self.item_slots[id.index()] = binding.slot;
                // The field is in this block, so in the current frame. The
                // record a dotted field extends is one its block is building,
                // never a future.
                let source = VarRef::Local(binding.slot);
                self.item_paths[id.index()] = PathResolution::unawaited(source, position(existing));
            }
        }
    }

    /// Adds a node for each segment of the dotted field path `segments` from
    /// the one at `from` on, each a child of the one before it and the first
    /// of `parent`, the last a leaf and the others open, spanning the path up
    /// to it. Returns the last, the leaf.
    fn add_path_nodes(&mut self, parent: u32, segments: &[PathSegment], from: usize) -> u32 {
        let last = segments.len() - 1;
        let mut parent = parent;
        for (index, segment) in segments.iter().enumerate().skip(from) {
            // A field's path's segments are all names.
            let Some(name) = segment.name() else {
                continue;
            };
            let span = path_span(&segments[..=index]);
            let kind = if index == last {
                NodeKind::Leaf
            } else {
                NodeKind::Open
            };
            parent = self.add_node(parent, name, span, kind);
        }
        parent
    }

    /// Finds where the path `segments`, whose first segment names the open
    /// field whose root node is `root`, attaches to that field: the deepest
    /// node along the path that exists, and the number of the path's
    /// segments that exist, at least 1 and short of all of them. Reports the
    /// path and returns `None` if it breaks a rule: a segment that exists as
    /// a leaf, or a last segment that exists at all.
    fn attach_point(&mut self, root: u32, segments: &[PathSegment]) -> Option<(u32, usize)> {
        let mut node = root;
        let last = segments.len() - 1;
        for (index, segment) in segments.iter().enumerate().skip(1) {
            let Some(child) = segment.name().and_then(|name| self.child(node, name)) else {
                return Some((node, index));
            };
            let found = self.nodes[child as usize];
            let prefix = self.ast.show_path(&segments[..=index]);
            let path = path_span(segments);
            if index == last {
                self.errors.push(
                    Diagnostic::error(format!("field `{prefix}` is defined twice"), path)
                        .with_context(found.span, "first defined here"),
                );
                return None;
            }
            match found.kind {
                NodeKind::Open => node = child,
                // Reported already.
                NodeKind::Leaf if found.ty == TyId::ERROR => return None,
                // A record, or one that may be or is a future, as one
                // defined whole where an either shape is expected is.
                NodeKind::Leaf if self.types.is_record(self.types.strip(found.ty)) => {
                    self.errors.push(
                        Diagnostic::error(format!("field `{prefix}` is already defined"), path)
                            .with_context(found.span, "defined whole here")
                            .with_note(WHOLE_FIELD_NOTE),
                    );
                    return None;
                }
                NodeKind::Leaf => {
                    let shown = self.ast.show_path(segments);
                    let ty = self.in_message(found.ty);
                    self.errors.push(
                        Diagnostic::error(
                            format!("cannot define `{shown}`: `{prefix}` is {ty}, not a record"),
                            path,
                        )
                        .with_context(found.span, format!("`{prefix}` defined here")),
                    );
                    return None;
                }
            }
        }
        // The last segment either exists or does not, and both return
        // above.
        None
    }

    /// The type `expecting` expects at the end of the path `segments`, of
    /// which the first `existing` segments are defined already, or `None`
    /// if it has none there. Walking the expected record type along the
    /// path stops at a segment it does not have, which is warned about as
    /// an extra field if this path defines it first, or at a type that is
    /// not a record, which is reported as a mismatch when the block ends.
    fn expected_at(
        &mut self,
        expecting: Expecting<'_>,
        segments: &[PathSegment],
        existing: usize,
    ) -> Option<TyId> {
        let mut expected = expecting.ty;
        for (index, segment) in segments.iter().enumerate() {
            // The record the path builds is present, which the either shape
            // of a record type accepts.
            expected = self.present_member(expected);
            // Not a record: reported when the block ends.
            self.types.record_fields(expected)?;
            let name = segment.name()?;
            let Some(ty) = self.types.field_or_rest(expected, name) else {
                if index >= existing {
                    let text = self.ast.names().get(name);
                    let record = self.in_message(expected);
                    self.warnings.push(
                        Diagnostic::warning(
                            format!("field `{}` is not in {record}", FieldName(text)),
                            segment.span,
                        )
                        .with_context(expecting.context, expecting.why)
                        .with_note(EXTRA_FIELD_NOTE),
                    );
                }
                return None;
            };
            expected = ty;
        }
        Some(expected)
    }

    /// The type a present value is checked against where `expected` is
    /// expected: the present member of an either shape, which a present
    /// value satisfies by satisfying that, and `expected` itself otherwise,
    /// a `Future` included, which no present value satisfies.
    fn present_member(&self, expected: TyId) -> TyId {
        if self.types.laziness(expected) == Laziness::Either {
            self.types.strip(expected)
        } else {
            expected
        }
    }

    /// Adds a node named `name`, at `span`, as the newest child of `parent`
    /// (or as a root, for [`NO_NODE`]), and returns its index. Its type is
    /// set by the caller.
    fn add_node(&mut self, parent: u32, name: Name, span: Span, kind: NodeKind) -> u32 {
        let index = position(self.nodes.len());
        let next_sibling = match self.nodes.get_mut(parent as usize) {
            Some(parent) => std::mem::replace(&mut parent.first_child, index),
            None => NO_NODE,
        };
        self.nodes.push(Node {
            name,
            span,
            ty: TyId::EMPTY,
            kind,
            parent,
            first_child: NO_NODE,
            next_sibling,
        });
        index
    }

    /// The child of `node` named `name`, if it has one.
    fn child(&self, node: u32, name: Name) -> Option<u32> {
        let mut child = self.nodes.get(node as usize)?.first_child;
        while let Some(entry) = self.nodes.get(child as usize) {
            if entry.name == name {
                return Some(child);
            }
            child = entry.next_sibling;
        }
        None
    }

    /// The type of the open node `node`: the record of its children's
    /// types, or `Error` if any of them is.
    fn node_type(&mut self, node: u32) -> TyId {
        let base = self.sorted_fields.len();
        let mut erroneous = false;
        let mut child = self
            .nodes
            .get(node as usize)
            .map_or(NO_NODE, |n| n.first_child);
        while let Some(entry) = self.nodes.get(child as usize) {
            erroneous |= entry.ty == TyId::ERROR;
            self.sorted_fields.push((entry.name, 0, entry.ty));
            child = entry.next_sibling;
        }
        let ty = if erroneous {
            TyId::ERROR
        } else {
            let fields = self.sorted_fields.get(base..).unwrap_or_default();
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        self.sorted_fields.truncate(base);
        ty
    }

    /// Once the input's outermost block has had its last item: gives every
    /// open node the record of its children's types, children first, and
    /// every open field its root's type. A top-level dotted item leaves the
    /// nodes above its leaf as they were (see `Checker::check_dotted_field`),
    /// and this settles them in one pass over the arena, whose nodes come
    /// after their parents.
    fn settle_top_nodes(&mut self) {
        for node in (0..self.nodes.len()).rev() {
            if self.nodes[node].kind == NodeKind::Open {
                let ty = self.node_type(u32::try_from(node).unwrap_or(NO_NODE));
                self.nodes[node].ty = ty;
            }
        }
        for index in 0..self.fields.len() {
            if let Some(open) = self.fields[index].open
                && let Some(node) = self.nodes.get(open.node as usize)
            {
                self.fields[index].ty = node.ty;
            }
        }
    }

    /// Checks `value` with no expected type, settling an untyped integer to
    /// `Int`, and returns its type.
    fn check_alone(&mut self, value: ExprId) -> TyId {
        let found = self.check_expr(value);
        self.settle(value, found)
    }

    /// The type of the record block whose fields are the innermost block's,
    /// once they are all declared: the record of the fields, or `Error` if
    /// a field is defined twice, has an erroneous type or had a dotted path
    /// under it rejected, or a spread was rejected. With `expecting`,
    /// also checks each soft field that stands against the expected type
    /// (see [`Checker::check_soft_fields`]), and reports each field of the
    /// expected type that the block lacks,
    /// at any level of an open field, and each open field or segment of one
    /// that the expected type has as something other than a record, which
    /// make the type `Error` too.
    fn record_type(&mut self, expecting: Option<Expecting<'_>>) -> TyId {
        let mut erroneous = self.spreads == BlockSpreads::Rejected;
        if self.spreads != BlockSpreads::None
            && let Some(expecting) = expecting
        {
            erroneous |= self.check_soft_fields(expecting);
        }
        let fields = self.fields.get(self.fields_start..).unwrap_or_default();
        let base = self.sorted_fields.len();
        self.sorted_fields.extend(
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| (field.name, position(index), field.ty)),
        );
        erroneous |= fields
            .iter()
            .any(|field| field.ty == TyId::ERROR || field.rejected);
        let sorted = self.sorted_fields.get_mut(base..).unwrap_or_default();
        sorted.sort_unstable_by_key(|&(name, index, _)| (name, index));
        // A name's first definition sorts first among its definitions, and
        // each later one is reported against it.
        let mut first: Option<(Name, &FieldDecl)> = None;
        for &(name, index, _) in &*sorted {
            let Some(field) = fields.get(index as usize) else {
                continue;
            };
            match first {
                Some((first_name, first)) if first_name == name => {
                    let text = self.ast.names().get(name);
                    self.errors.push(
                        Diagnostic::error(
                            format!("field `{}` is defined twice", FieldName(text)),
                            field.name_span,
                        )
                        .with_context(first.name_span, "first defined here"),
                    );
                    erroneous = true;
                }
                _ => first = Some((name, field)),
            }
        }
        if let Some(expecting) = expecting {
            erroneous |= self.report_missing_fields(base, expecting);
            erroneous |= self.check_open_fields(expecting);
        }
        let rest = expecting.and_then(|expecting| {
            let rest = self.types.record_rest(expecting.ty)?;
            erroneous |= self.check_rest_fields(expecting, rest);
            Some(rest)
        });
        let ty = if erroneous {
            TyId::ERROR
        } else {
            let sorted = self.sorted_fields.get(base..).unwrap_or_default();
            self.types.record_with_rest(
                sorted.iter().map(|&(name, _, ty)| (name, ty)),
                rest,
                Vec::new(),
            )
        };
        self.sorted_fields.truncate(base);
        ty
    }

    /// Checks that each field of the innermost block that `expecting`'s
    /// record type does not name fits its rest-field, `rest`, so that the
    /// block's record can have it (see [Records](#records)), and returns
    /// whether one does not. A field item, a spread's field and an
    /// annotated field were checked against `rest` as they were defined,
    /// and one that did not fit reported; an open field, whose record its
    /// dotted paths build has no rest-field of its own, may not fit a
    /// `rest` that has one, which is "mismatched types" at its first
    /// definition.
    fn check_rest_fields(&mut self, expecting: Expecting<'_>, rest: TyId) -> bool {
        let mut erroneous = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            if field.ty == TyId::ERROR
                || rest == TyId::ERROR
                || self.types.field(expecting.ty, field.name).is_some()
                || self.types.is_subtype(field.ty, rest)
            {
                continue;
            }
            erroneous = true;
            if field.open.is_none() {
                continue;
            }
            let label = format!(
                "expected {}, found {}",
                self.in_label(rest),
                self.in_label(field.ty)
            );
            self.errors.push(
                Diagnostic::error("mismatched types", field.name_span)
                    .with_label(label)
                    .with_context(expecting.context, expecting.why)
                    .with_note(DOTTED_REST_NOTE),
            );
        }
        erroneous
    }

    /// Checks each soft field of the innermost block that stands, one that
    /// no later item overrode, and that `expecting`'s record type has,
    /// against the type it has there, as a field's value is (see the module
    /// documentation), except that it cannot be awaited to fit, since there
    /// is no expression to await: a provisional type in it where the
    /// expected type is present is measured, at the spread's operand, and a
    /// field that still does not fit is "mismatched types" at the operand,
    /// "expected E, found F for field `a`", with a note if awaiting it would
    /// have made it fit, and one if it lacks a defaulted field
    /// ([`Checker::default_note`]). A field that fits takes the expected type, as a
    /// field item's does. A soft field the type lacks is not warned about:
    /// spreading a wider record is normal. Returns whether anything was
    /// reported.
    fn check_soft_fields(&mut self, expecting: Expecting<'_>) -> bool {
        let mut reported = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            let Some(operand) = field.spread else {
                continue;
            };
            let Some(expected) = self.types.field_or_rest(expecting.ty, field.name) else {
                continue;
            };
            // An unknown type has been reported.
            if expected == TyId::ERROR || field.ty == TyId::ERROR {
                continue;
            }
            let span = self.ast.get(operand).span;
            let found = if self.types.is_subtype(field.ty, expected) {
                None
            } else {
                match self.claim_against_at(span, field.ty, expected) {
                    Some(claimed) if self.types.is_subtype(claimed, expected) => None,
                    Some(claimed) => Some(claimed),
                    None => Some(field.ty),
                }
            };
            let Some(found) = found else {
                self.fields[index].ty = expected;
                continue;
            };
            let text = FieldName(self.ast.names().get(field.name)).to_string();
            let label = format!(
                "expected {}, found {} for field `{text}`",
                self.in_label(expected),
                self.in_label(found)
            );
            let mut error = Diagnostic::error("mismatched types", span)
                .with_label(label)
                .with_context(expecting.context, expecting.why);
            if self.types.needs_await(found, expected).is_some() {
                let example = self
                    .path_text(operand)
                    .map(|path| format!(", as in `{text} = {path}.{text}`"))
                    .unwrap_or_default();
                error = error.with_note(format!(
                    "a spread's field cannot be awaited; define the field with its value{example}"
                ));
            }
            if let Some(note) = self.default_note(found, expected) {
                error = error.with_note(note);
            }
            self.errors.push(error);
            reported = true;
        }
        reported
    }

    /// Reports each field of `expecting`'s record type that is not among the
    /// block's fields, `sorted_fields[base..]`, in alphabetical order,
    /// unless the block fills it (see [`Expecting::fill`]) and the type has
    /// a default for it: such a field is added to the block's fields, at the
    /// end of `sorted_fields`, with the type's type of it, and recorded,
    /// with its default, as given it ([`Resolution::record_fills`]; see
    /// [Field defaults](#field-defaults)). Returns whether any was missing.
    fn report_missing_fields(&mut self, base: usize, expecting: Expecting<'_>) -> bool {
        let names = self.ast.names();
        let present = self.sorted_fields.get(base..).unwrap_or_default();
        let mut missing: Vec<&str> = Vec::new();
        let mut filled: Vec<(Name, TyId, Constant)> = Vec::new();
        for &(name, ty) in self.types.record_fields(expecting.ty).unwrap_or_default() {
            if present
                .binary_search_by_key(&name, |&(present, _, _)| present)
                .is_ok()
            {
                continue;
            }
            let default = self
                .types
                .field_default(expecting.ty, name)
                .filter(|_| expecting.fill.is_some());
            match default {
                Some(default) => filled.push((name, ty, default.clone())),
                None => missing.push(names.get(name)),
            }
        }
        if let Some(block) = expecting.fill
            && !filled.is_empty()
        {
            filled.sort_unstable_by(|(a, _, _), (b, _, _)| names.get(*a).cmp(names.get(*b)));
            self.sorted_fields
                .extend(filled.iter().map(|&(name, ty, _)| (name, u32::MAX, ty)));
            let fills = filled
                .into_iter()
                .map(|(name, _, default)| (name, default))
                .collect();
            self.lowering.record_fills.insert(block, fills);
        }
        if missing.is_empty() {
            return false;
        }
        missing.sort_unstable();
        let record = self.in_message(expecting.ty);
        for text in missing {
            self.errors.push(
                Diagnostic::error(
                    format!("missing field `{}` in record", FieldName(text)),
                    expecting.close,
                )
                .with_context(expecting.context, expecting.why)
                .with_note(format!("the type {record} needs it")),
            );
        }
        true
    }

    /// Checks each open field of the innermost block that `expecting`'s
    /// record type has against the type it has there, at every level of
    /// nesting, through a work list rather than recursion. A segment is
    /// checked against a record type by its fields: each one it lacks is
    /// reported at the block's closing brace as "missing field `a.x` in
    /// record", and each open segment is checked in turn (a leaf was checked
    /// against its type when it was defined). A segment expected to be
    /// something other than a record is a mismatch, reported at its first
    /// definition. Returns whether anything was reported.
    fn check_open_fields(&mut self, expecting: Expecting<'_>) -> bool {
        let mut reported = false;
        for index in self.fields_start..self.fields.len() {
            let field = self.fields[index];
            let Some(open) = field.open else {
                continue;
            };
            // An erroneous field has been reported already.
            if field.ty == TyId::ERROR {
                continue;
            }
            let Some(expected) = self.types.field_or_rest(expecting.ty, field.name) else {
                continue;
            };
            self.pending.push((open.node, expected));
            while let Some((node, expected)) = self.pending.pop() {
                reported |= self.check_open_node(node, expected, expecting);
            }
        }
        reported
    }

    /// Checks the open node `node` against the type `expected`, reporting a
    /// mismatch or each missing field, and queues its open children that
    /// the type has on `pending`. Returns whether anything was reported.
    fn check_open_node(&mut self, node: u32, expected: TyId, expecting: Expecting<'_>) -> bool {
        let entry = self.nodes[node as usize];
        // An unknown type has been reported, and a record type never has a
        // field of one, but nothing is to be checked against it either way.
        if expected == TyId::ERROR || entry.ty == TyId::ERROR {
            return false;
        }
        // An open node is a present record, which the either shape of a
        // record type accepts.
        let record = self.present_member(expected);
        let Some(fields) = self.types.record_fields(record) else {
            let label = format!(
                "expected {}, found {}",
                self.in_label(expected),
                self.in_label(entry.ty)
            );
            self.errors.push(
                Diagnostic::error("mismatched types", entry.span)
                    .with_label(label)
                    .with_context(expecting.context, expecting.why),
            );
            return true;
        };
        let names = self.ast.names();
        let mut missing: Vec<&str> = Vec::new();
        for &(name, ty) in fields {
            match self.child(node, name) {
                None => missing.push(names.get(name)),
                Some(child) if self.nodes[child as usize].kind == NodeKind::Open => {
                    self.pending.push((child, ty));
                }
                Some(_) => {}
            }
        }
        // An open child the type does not name is checked against its
        // rest-field, if it has one.
        if let Some(rest) = self.types.record_rest(record) {
            let mut child = entry.first_child;
            while let Some(&found) = self.nodes.get(child as usize) {
                if found.kind == NodeKind::Open && self.types.field(record, found.name).is_none() {
                    self.pending.push((child, rest));
                }
                child = found.next_sibling;
            }
        }
        if missing.is_empty() {
            return false;
        }
        missing.sort_unstable();
        let path = self.node_path(node);
        let record = self.in_message(record);
        for text in missing {
            self.errors.push(
                Diagnostic::error(
                    format!("missing field `{path}.{}` in record", FieldName(text)),
                    expecting.close,
                )
                .with_context(expecting.context, expecting.why)
                .with_note(format!("the type {record} needs it")),
            );
        }
        true
    }

    /// The path from its field's name to the node `node`, as it is
    /// written, for diagnostics.
    fn node_path(&self, node: u32) -> String {
        let names = self.ast.names();
        let mut texts = Vec::new();
        let mut current = node;
        while let Some(entry) = self.nodes.get(current as usize) {
            texts.push(FieldName(names.get(entry.name)).to_string());
            current = entry.parent;
        }
        texts.reverse();
        texts.join(".")
    }

    /// Checks `PATH := VALUE` and declares the new binding: with the value's
    /// type, or for a dotted path the type of the update. The binding is
    /// declared even when the rebind is rejected, so that later uses of the
    /// name do not cascade into "undefined variable" errors. A rebind of a
    /// mutable variable declares nothing: see
    /// [`Checker::check_mutable_rebind`]. The positions of the path's index
    /// segments are checked once each, before the value: as the path is
    /// walked, or on their own if it is not.
    fn check_rebind(&mut self, id: ItemId, path: PathRange, value: ExprId) {
        let segments = self.ast.path(path);
        // A path's first segment is a name.
        let Some((name, name_span)) = segments
            .first()
            .and_then(|first| Some((first.name()?, first.span)))
        else {
            return;
        };
        let text = self.ast.names().get(name);
        // The binding being rebound, with its index in `bindings`, once the
        // `:=` rules pass.
        let target = match self.lookup(name) {
            // A top-level field of this input, already reported at the item
            // as a field of the file.
            Some((_, binding))
                if binding.kind == BindingKind::Field && self.field_clash == Some(id) =>
            {
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Field => {
                let mut error = Diagnostic::error(
                    format!("cannot rebind `{text}`: it is a field, not a variable"),
                    name_span,
                );
                // A field from an earlier session input has no item here to
                // point at.
                if let Some(declared) = binding.declared {
                    error = error.with_context(declared, "declared here");
                }
                self.errors.push(error.with_note(format!(
                    "rebind with `{text} = ...` is not allowed either: a field is defined once"
                )));
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Callee => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: it is the function being defined"),
                        name_span,
                    )
                    .with_note("shadow it with `let` instead"),
                );
                None
            }
            Some((index, binding)) if binding.kind == BindingKind::Mutable => {
                self.record_rebind(id, binding.declared);
                self.check_mutable_rebind(id, segments, (index, binding), value);
                return;
            }
            Some((index, binding)) if binding.kind == BindingKind::Header => {
                self.record_rebind(id, binding.declared);
                if self.header_rebind(id, segments, (index, binding), value) {
                    return;
                }
                None
            }
            Some((_, binding)) if binding.kind == BindingKind::Narrowed => {
                let error = narrowed_rebind_error(text, name_span, binding);
                self.errors.push(error);
                // Nothing is declared: the copy stands, so nothing cascades.
                self.check_positions_alone(segments);
                self.check_alone(value);
                return;
            }
            Some((index, binding)) if index >= self.block_start => Some((index, binding)),
            Some((index, binding)) => {
                let error = self.enclosing_rebind_error(name_span, (index, binding));
                self.errors.push(error);
                None
            }
            None => {
                self.errors.push(
                    Diagnostic::error(
                        format!("cannot rebind `{text}`: no variable named `{text}` is declared"),
                        name_span,
                    )
                    .with_note(format!("declare it with `let {text} = ...`")),
                );
                None
            }
        };
        let Some((index, target)) =
            target.filter(|(_, target)| target.ty != TyId::ERROR && target.ty != self.never)
        else {
            self.rejected_rebind(id, (segments, name), value, target.map(|(_, t)| t.ty));
            return;
        };
        self.record_rebind(id, target.declared);
        // `:=` is assignment-compatible: the value must fit the variable's
        // type (or the path's leaf type), and the binding keeps that type.
        let context = target.declared.unwrap_or(name_span);
        if segments.len() == 1 {
            self.check_expr_expecting(value, target.ty, context, "the variable was declared here");
            self.declare(name, target.ty, id, BindingKind::Variable);
            return;
        }
        // The path is walked, measuring its records and lists and checking
        // its positions, before the value is checked, since the value is
        // checked against its leaf's type; its records and lists stay on
        // `path_types` until the variable is declared.
        let base = self.path_types.len();
        if let Some(leaf) = self.rebind_leaf(segments, (target.ty, index)) {
            self.check_expr_expecting(value, leaf, path_span(segments), leaf_label(segments));
            self.finish_update(id, segments, (Some((index, target)), base), leaf);
        } else {
            self.check_alone(value);
            // A path through a value of type `Never` is never walked.
            let ty = if std::mem::take(&mut self.unreached_path) {
                self.never
            } else {
                TyId::ERROR
            };
            self.declare(name, ty, id, BindingKind::Variable);
        }
    }

    /// Checks `PATH := VALUE`, the item `id` whose path is `segments`, of
    /// `target`, at `index` in `bindings`, a variable a plain `let` of a
    /// `for` header declared (see [For](#for)): from the header, its own
    /// block, and of its own function, it is a rebind of the variable in
    /// its one slot, keeping its type, as a mutable variable's is (see
    /// [`Checker::check_mutable_rebind`]), and this returns `true`;
    /// otherwise it is reported (see [`Checker::header_rebind_error`]), and
    /// this returns `false`, the caller checking the rest as for any
    /// rejected `:=`.
    fn header_rebind(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (index, target): (usize, Binding),
        value: ExprId,
    ) -> bool {
        let own_function = self
            .functions
            .last()
            .is_none_or(|function| index >= function.bindings_start);
        if index >= self.block_start && own_function {
            self.check_mutable_rebind(id, segments, (index, target), value);
            return true;
        }
        let name_span = segments.first().map_or(Span::new(0, 0), |first| first.span);
        let error = self.header_rebind_error(name_span, (index, target));
        self.errors.push(error);
        false
    }

    /// Checks the `:=` item `id`, whose path is `segments`, of the variable
    /// `name`, with the value `value`, which was rejected, or whose
    /// variable, of type `ty`, is erroneous or `Never`: the positions and
    /// the value are checked on their own, and the name declared with what
    /// it has, so later uses do not cascade. The path of a variable that may
    /// be a future is not walked: awaiting it would be reported of a
    /// variable that cannot be rebound anyway. Nor is one with an index
    /// segment, whose walk would check the positions again. A variable of
    /// type `Never` never holds a value, so the `:=` is never run, and the
    /// name is declared `Never` again, so that nothing after it is an
    /// error.
    fn rejected_rebind(
        &mut self,
        id: ItemId,
        (segments, name): (&[PathSegment], Name),
        value: ExprId,
        ty: Option<TyId>,
    ) {
        self.check_positions_alone(segments);
        let value_ty = self.check_alone(value);
        if ty == Some(self.never) {
            self.declare(name, self.never, id, BindingKind::Variable);
            return;
        }
        let lazy = self.lookup(name).is_some_and(|(_, binding)| {
            binding.ty != TyId::ERROR && self.types.laziness(binding.ty) != Laziness::Present
        });
        let indexed = segments.iter().any(|segment| segment.position().is_some());
        if segments.len() == 1 {
            self.declare(name, value_ty, id, BindingKind::Variable);
        } else if lazy || indexed {
            self.declare(name, TyId::ERROR, id, BindingKind::Variable);
        } else {
            self.declare_update(id, segments, value_ty);
        }
    }

    /// The error for a `:=`, whose name is at `name_span`, of the variable
    /// `binding`, at `index` in `bindings`, a plain `let` of a `for` header,
    /// from outside the header: "cannot rebind `i` here: it is declared in
    /// the header of a `for`", labelled at the declaration, with a note
    /// that a `let mut` in the header can be rebound from the body; or, from
    /// a function or a `lazy` nested in the variable's own, the error for
    /// any variable declared in an enclosing block.
    fn header_rebind_error(
        &self,
        name_span: Span,
        (index, binding): (usize, Binding),
    ) -> Diagnostic {
        let outside = self
            .functions
            .last()
            .is_some_and(|function| index < function.bindings_start);
        if outside {
            return self.enclosing_rebind_error(name_span, (index, binding));
        }
        let text = self.ast.names().get(binding.name);
        let mut error = Diagnostic::error(
            format!("cannot rebind `{text}` here: it is declared in the header of a `for`"),
            name_span,
        );
        if let Some(declared) = binding.declared {
            error = error.with_context(declared, "declared here");
        }
        error.with_note(format!(
            "a variable of a `for` header is rebound in the header, after the condition; declare \
             it with `let mut {text} = ...` in the header to rebind it from the body too"
        ))
    }

    /// Checks `PATH := VALUE`, the item `id` whose path is `segments`, of
    /// the mutable variable `target`, at `index` in `bindings`, which is the
    /// innermost binding of the path's first segment (see the module
    /// documentation). From the variable's own function, or from the top
    /// level for a variable of the top level, the rebind is allowed in any
    /// block, and writes the variable's own slot: the value must fit the
    /// variable's type as for a `:=` of a variable that is not mutable, in
    /// its declaring block, and a dotted path is walked and its leaf
    /// checked as for one too, but nothing is declared, and the variable
    /// keeps its type, its declaration and its place in `bindings`. A
    /// dotted rebind's record must therefore be of the variable's type (see
    /// [`Checker::kept_type`]). From a function expression or a `lazy`
    /// nested in that function, the rebind is rejected: the function holds
    /// its own copy of the variable. A rejected rebind, and one of a
    /// variable that is erroneous already, check the positions of the path's
    /// index segments and the value on their own and declare nothing, so
    /// nothing cascades.
    fn check_mutable_rebind(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (index, target): (usize, Binding),
        value: ExprId,
    ) {
        let Some(first) = segments.first() else {
            return;
        };
        if let Some(function) = self.functions.last()
            && index < function.bindings_start
        {
            let error =
                self.outside_rebind_error((target.name, first.span), target, function.fn_id);
            self.errors.push(error);
            self.check_positions_alone(segments);
            self.check_alone(value);
            return;
        }
        if target.ty == TyId::ERROR || target.ty == self.never {
            self.check_positions_alone(segments);
            self.check_alone(value);
            // A variable of type `Never` never holds a value, so the `:=`
            // is never run, but is lowered as any other is.
            if target.ty == self.never {
                self.item_slots[id.index()] = target.slot;
            }
            return;
        }
        if segments.len() == 1 {
            let context = target.declared.unwrap_or(first.span);
            self.check_expr_expecting(value, target.ty, context, "the variable was declared here");
            self.item_slots[id.index()] = target.slot;
            return;
        }
        // As for any dotted `:=`, the path is walked before the value is
        // checked against its leaf's type.
        let base = self.path_types.len();
        let Some(leaf) = self.rebind_leaf(segments, (target.ty, index)) else {
            self.check_alone(value);
            if std::mem::take(&mut self.unreached_path) {
                self.item_slots[id.index()] = target.slot;
            }
            return;
        };
        self.check_expr_expecting(value, leaf, path_span(segments), leaf_label(segments));
        let updated = self.record_update(id, segments, (Some((index, target)), base), leaf);
        self.kept_type(segments, target, updated);
        self.item_slots[id.index()] = target.slot;
    }

    /// The error for a `:=`, whose name is at `name_span`, of the variable
    /// `binding`, at `index` in `bindings`, which is not mutable and is
    /// declared in a block enclosing the `:=`'s own: "cannot rebind `x`
    /// here: it is declared in an enclosing block", labelled at the
    /// declaration when there is one. Its note says to shadow the variable,
    /// and that a `let mut` variable can be rebound from an inner block,
    /// unless the `:=` is in a function or a `lazy` nested in the
    /// variable's own, where `let mut` would not help either.
    fn enclosing_rebind_error(
        &self,
        name_span: Span,
        (index, binding): (usize, Binding),
    ) -> Diagnostic {
        let text = self.ast.names().get(binding.name);
        let mut error = Diagnostic::error(
            format!("cannot rebind `{text}` here: it is declared in an enclosing block"),
            name_span,
        );
        // A binding from an earlier session input has no item here to
        // point at.
        if let Some(declared) = binding.declared {
            error = error.with_context(declared, "declared here");
        }
        let outside = self
            .functions
            .last()
            .is_some_and(|function| index < function.bindings_start);
        error.with_note(if outside {
            format!("shadow it with `let {text} = ...` instead")
        } else {
            format!(
                "shadow it with `let {text} = ...` instead; only a variable declared with `let \
                 mut` can be rebound from an inner block"
            )
        })
    }

    /// The error for a `:=` of the mutable variable `target`, the first
    /// segment of whose path is `name`, written at `span`, from inside the
    /// function expression or `lazy` `function`, nested in the variable's
    /// own function or in the top level: "cannot rebind `x` here: it is
    /// declared outside this function" (or "this `lazy`"), labelled at the
    /// declaration when there is one.
    fn outside_rebind_error(
        &self,
        (name, span): (Name, Span),
        target: Binding,
        function: FnId,
    ) -> Diagnostic {
        let text = self.ast.names().get(name);
        // A `lazy` is the one function that declares no return type.
        let kind = if self.ast.fn_decl(function).ret.is_none() {
            "`lazy`"
        } else {
            "function"
        };
        let mut error = Diagnostic::error(
            format!("cannot rebind `{text}` here: it is declared outside this {kind}"),
            span,
        );
        // A variable from an earlier session input has no item here to
        // point at.
        if let Some(declared) = target.declared {
            error = error.with_context(declared, "declared here");
        }
        error.with_note(format!(
            "a {kind} holds its own copy of each variable it uses, made when the {kind} is \
             created; rebinding the copy would not change `{text}`"
        ))
    }

    /// Checks that `updated`, the record a dotted `:=` of the path
    /// `segments` makes of the mutable variable `target`, is of the
    /// variable's type, which a mutable variable keeps: it is whenever the
    /// variable and the records along the path are present or the either
    /// shape, since the update makes them present. Otherwise, as for a
    /// variable declared exactly `Future<{a: Int}>`, reports "cannot rebind
    /// `r.a`: `r` is declared `Future<{a: Int}>`, and the update makes it
    /// `{a: Int}`" at the path.
    fn kept_type(&mut self, segments: &[PathSegment], target: Binding, updated: TyId) {
        if updated == TyId::ERROR || self.types.is_subtype(updated, target.ty) {
            return;
        }
        let path = self.ast.show_path(segments).to_string();
        let text = self.ast.names().get(target.name);
        let declared = self.in_message(target.ty);
        let made = self.in_message(updated);
        let mut error = Diagnostic::error(
            format!(
                "cannot rebind `{path}`: `{text}` is declared {declared}, and the update makes \
                 it {made}"
            ),
            path_span(segments),
        );
        if let Some(span) = target.declared {
            error = error.with_context(span, "declared here");
        }
        self.errors.push(error.with_note(
            "a mutable variable keeps its type; declare it with a type that allows both, or \
             await it where it is declared",
        ));
    }

    /// The type of the leaf of the dotted `:=` path `segments`, whose first
    /// segment's variable, at `binding` in `bindings`, has type `root`, once
    /// its path is walked (see [`Checker::walk_path`]), which leaves the
    /// records and lists along it on `path_types` for
    /// [`Checker::finish_update`]: a field's type, or a list's element type
    /// as it is for an index. Every segment must already exist, since `:=`
    /// never adds a field: a missing one, or a segment along the way that is
    /// not a record or a list as its segment needs, is reported and `None`
    /// returned, with `path_types` as it was. An index segment always
    /// exists: its position is a runtime matter.
    fn rebind_leaf(
        &mut self,
        segments: &[PathSegment],
        (root, binding): (TyId, usize),
    ) -> Option<TyId> {
        let base = self.path_types.len();
        let root = self.walk_path(segments, (root, binding))?;
        let records = self.path_types.get(base..)?;
        let (walked, &(record, _), last) = (records.len(), records.last()?, segments.last()?);
        // The first segment that the record before it lacks: one the walk
        // stopped at, or the leaf.
        let missing = if walked + 1 < segments.len() {
            walked
        } else {
            let leaf = match last.kind {
                SegmentKind::Name(name) => self.types.field(record, name),
                SegmentKind::Index { .. } => self.types.list_element(record),
            };
            match leaf {
                Some(leaf) => return Some(leaf),
                None => walked,
            }
        };
        self.path_types.truncate(base);
        // Only a name can be missing.
        let name = segments[missing].name()?;
        let path = self.ast.show_path(segments);
        let prefix = self.ast.show_path(&segments[..missing]);
        let field = FieldName(self.ast.names().get(name));
        let label = format!("this is {}", self.in_label(root));
        let error = Diagnostic::error(
            format!("cannot rebind `{path}`: `{prefix}` has no field `{field}`"),
            path_span(segments),
        )
        .with_context(segments[0].span, label);
        // A `let` path cannot index a list, so a path with an index segment
        // has no `let` to suggest.
        let indexed = segments.iter().any(|segment| segment.position().is_some());
        self.errors.push(if indexed {
            error
        } else {
            error.with_note(format!(
                "`:=` cannot add a field; add it with `let {path} = ...`"
            ))
        });
        None
    }

    /// Walks the path `segments` of a dotted `let` or `:=` whose first
    /// segment's variable, at `binding` in `bindings`, has type `old`, which
    /// is not erroneous. Updating a record or a list reads it, so each one
    /// along the path that exists is measured at the top (see
    /// [`Checker::measure_path_record`]): the variable's, and each following
    /// segment's, short of the last, that the record before it has, or that
    /// is an element of the list before it. Each is pushed onto
    /// `path_types`, stripped, with whether it is awaited; the walk stops at
    /// the first segment the record before it lacks, and before the last,
    /// which is replaced, not read. The position of an index segment, which
    /// only a `:=` path has, is checked where the walk reaches it, after its
    /// list is measured (see [`Checker::check_position`]).
    ///
    /// Returns `old` as measured, for labelling a later error. A value that
    /// is not a record, before a name, or a list, before an index, once
    /// stripped is reported instead, "cannot update `a.b.c`: `a.b` is Int,
    /// not a record" or "cannot update `xs[0]`: `xs` is Int, not a list",
    /// showing its type before stripping, and `None` returned, with
    /// `path_types` as it was; so is an index whose position is not an
    /// `Int`. Every position the walk does not reach is checked on its own,
    /// so that each is checked exactly once whatever happens.
    fn walk_path(
        &mut self,
        segments: &[PathSegment],
        (old, binding): (TyId, usize),
    ) -> Option<TyId> {
        let base = self.path_types.len();
        let mut root = old;
        // The type of `segments[..index]`, which exists.
        let mut current = old;
        for (index, next) in segments.iter().enumerate().skip(1) {
            let prefix = &segments[..index];
            let variable = (index == 1).then_some(binding);
            let position = next.position();
            let (measured, awaited) =
                self.measure_path_record(prefix, current, (variable, position.is_some()));
            if index == 1 {
                root = measured;
            }
            let record = self.types.strip(measured);
            if record == self.never {
                // No value has it, so the update is never run: the path
                // stops quietly, and the caller declares `Never`.
                self.path_types.truncate(base);
                self.check_positions_alone(&segments[index..]);
                self.unreached_path = true;
                return None;
            }
            let inner = match position {
                None if self.types.is_record(record) => {
                    next.name().map(|name| self.types.field(record, name))
                }
                Some(_) => self.types.list_element(record).map(Some),
                None => None,
            };
            let Some(inner) = inner else {
                self.path_types.truncate(base);
                self.not_updatable(segments, prefix, (measured, root));
                self.check_positions_alone(&segments[index..]);
                return None;
            };
            self.path_types.push((record, awaited));
            if let Some(position) = position
                && !self.check_position(position)
            {
                self.path_types.truncate(base);
                self.check_positions_alone(&segments[index + 1..]);
                return None;
            }
            // After the last record or list comes the leaf, which is never
            // measured.
            match inner {
                Some(ty) if index + 1 < segments.len() => current = ty,
                _ => {
                    self.check_positions_alone(&segments[index + 1..]);
                    break;
                }
            }
        }
        Some(root)
    }

    /// Reports that the update whose path is `segments` cannot select its
    /// next segment in the value at `prefix`, of type `measured`, the
    /// variable's being `root` (see [`Checker::walk_path`]): "cannot update
    /// `a.b.c`: `a.b` is Int, not a record" before a name, or "... not a
    /// list" before an index, labelled at the variable with its type.
    fn not_updatable(
        &mut self,
        segments: &[PathSegment],
        prefix: &[PathSegment],
        (measured, root): (TyId, TyId),
    ) {
        let needed = match segments.get(prefix.len()).and_then(PathSegment::position) {
            Some(_) => "list",
            None => "record",
        };
        let message = format!(
            "cannot update `{}`: `{}` is {}, not a {needed}",
            self.ast.show_path(segments),
            self.ast.show_path(prefix),
            self.in_message(measured)
        );
        let label = format!("this is {}", self.in_label(root));
        self.errors.push(
            Diagnostic::error(message, path_span(segments)).with_context(segments[0].span, label),
        );
    }

    /// Checks the position of each index segment of `segments` on its own,
    /// for a `:=` that is rejected, or whose walk stops before reaching
    /// them, so that their mistakes are reported and nothing cascades.
    fn check_positions_alone(&mut self, segments: &[PathSegment]) {
        for position in segments.iter().filter_map(PathSegment::position) {
            self.check_alone(position);
        }
    }

    /// Where the record or list at `prefix`, a proper prefix of the path of
    /// a dotted `let` or `:=`, of type `ty`, is read by the update, as
    /// [`Checker::measure`] measures the record of a field access or the
    /// list of an index: if it is a parameter of a claimable function (see
    /// [`Checker::claimable`]) whose type is provisional, the parameter is
    /// measured, which makes it present, and its present type returned; if
    /// it may be a future, it is to be awaited at the top, subject to the
    /// purity rule, reported at `prefix` (see
    /// [`Checker::implicit_await_error`]) if it is what the next segment
    /// needs once stripped, a list if `list` and a record otherwise, and
    /// `ty` is returned with `true`; otherwise, or if it is erroneous, `ty`
    /// is returned with `false`. `variable` is where the value is in
    /// `bindings` if it is the variable the path updates, the path's first
    /// segment, whose name the purity error's note suggests awaiting before
    /// the pure function is created if it can be (see
    /// [`Checker::awaitable_outside`]).
    fn measure_path_record(
        &mut self,
        prefix: &[PathSegment],
        ty: TyId,
        (variable, list): (Option<usize>, bool),
    ) -> (TyId, bool) {
        if ty == TyId::ERROR || self.types.laziness(ty) == Laziness::Present {
            return (ty, false);
        }
        let span = path_span(prefix);
        let claimed = self.claim_top(span, ty);
        if claimed != ty {
            return (claimed, false);
        }
        // A future, an either, or a provisional type of an enclosing
        // function, which may be a future for all this body knows. A path
        // is not an expression, so the await is recorded with the item's
        // path, by `finish_update`. A value that is not a record, or a list,
        // once awaited cannot be updated, awaited or not, which `walk_path`
        // reports instead.
        let stripped = self.types.strip(ty);
        let updatable = if list {
            self.types.list_element(stripped).is_some()
        } else {
            self.types.is_record(stripped)
        };
        if let Some(marker) = self.pure_marker()
            && updatable
        {
            let capture = match (variable, prefix) {
                (Some(binding), [first]) if self.awaitable_outside(binding) => first.name(),
                _ => None,
            };
            let error = self.implicit_await_error(span, ty, marker, capture);
            self.errors.push(error);
        }
        (ty, true)
    }

    /// Declares the variable of the dotted `let` or `:=` item `id`, whose
    /// path is `segments` and whose value has type `leaf_ty`: a new binding
    /// of the path's first segment, `a`, whose type is that of `a` in scope,
    /// or of `{}` if there is none, with the rest of the path set to
    /// `leaf_ty` (see the module documentation). Walks the path first (see
    /// [`Checker::walk_path`]). If `a` or a segment along the way is not a
    /// record, that is reported and the binding is `Error`.
    fn declare_update(&mut self, id: ItemId, segments: &[PathSegment], leaf_ty: TyId) {
        let Some(name) = segments.first().and_then(PathSegment::name) else {
            return;
        };
        let base = self.path_types.len();
        // A dotted `let` of a top-level field's name has been reported as
        // "`a` is a field of this file"; the field is not what it updates.
        let clash = self.field_clash == Some(id);
        let mut found = if clash { None } else { self.lookup(name) };
        let mut failed = leaf_ty == TyId::ERROR || clash;
        if let Some((index, old)) = found {
            // An erroneous variable has been reported already.
            failed |= old.ty == TyId::ERROR || self.walk_path(segments, (old.ty, index)).is_none();
        }
        // A path through a value of type `Never` is never walked: the
        // update builds its records from nothing, as if no variable were in
        // scope, and is never run.
        if std::mem::take(&mut self.unreached_path) {
            (failed, found) = (leaf_ty == TyId::ERROR, None);
        }
        let leaf_ty = if failed { TyId::ERROR } else { leaf_ty };
        self.finish_update(id, segments, (found, base), leaf_ty);
    }

    /// Declares the variable of the dotted `let` or `:=` item `id`, whose
    /// path is `segments`, whose leaf is set to a value of type `leaf_ty`,
    /// `Error` if the update was rejected, and whose path has been walked
    /// (see [`Checker::walk_path`]): the records and lists along it that
    /// exist are the entries of `path_types` from `base` on, which this
    /// truncates back. `found` is the binding of the path's first segment in
    /// scope, with its index in `bindings`, if there is one. The new
    /// binding's type is built from the leaf outwards: each record along
    /// the path, as it was once measured or created empty, with its segment
    /// set, and each list as it was once measured, its element type kept
    /// (see [`Checker::updated_list`]). Records for lowering where the old
    /// record or list is read from, how many records and lists along the
    /// path exist, and which of them are awaited, as a [`PathResolution`].
    fn finish_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (found, base): (Option<(usize, Binding)>, usize),
        leaf_ty: TyId,
    ) {
        let Some(name) = segments.first().and_then(PathSegment::name) else {
            return;
        };
        let ty = self.record_update(id, segments, (found, base), leaf_ty);
        self.declare(name, ty, id, BindingKind::Variable);
    }

    /// The type of the record or list that the dotted `let` or `:=` item
    /// `id`, whose path is `segments`, makes, as [`Checker::finish_update`]
    /// describes it, for whatever the item declares or rebinds; records
    /// for lowering how its path resolves, and truncates `path_types` back
    /// to `base`. `Error` if `leaf_ty` is, if an updated element does not
    /// fit its list, or if `segments` is empty.
    fn record_update(
        &mut self,
        id: ItemId,
        segments: &[PathSegment],
        (found, base): (Option<(usize, Binding)>, usize),
        leaf_ty: TyId,
    ) -> TyId {
        let Some((_, rest)) = segments.split_first() else {
            self.path_types.truncate(base);
            return TyId::ERROR;
        };
        let existing = self.path_types.len().saturating_sub(base);
        let mut ty = leaf_ty;
        for (level, segment) in rest.iter().enumerate().rev() {
            if ty == TyId::ERROR {
                break;
            }
            let record = self
                .path_types
                .get(base + level)
                .map_or(TyId::EMPTY, |&(record, _)| record);
            ty = match segment.kind {
                SegmentKind::Name(name) => self
                    .types
                    .with_field(record, name, ty)
                    .unwrap_or(TyId::ERROR),
                // The list's prefix is the path up to it.
                SegmentKind::Index { .. } => {
                    self.updated_list(segments, &segments[..=level], (record, ty))
                }
            };
        }
        if let Some((index, old)) = found
            && existing > 0
        {
            // The old record may belong to an enclosing function, and is
            // then read from the closure's captures.
            let source = self.resolve(index, old);
            let records = self.path_types.get(base..).unwrap_or_default();
            // Most updates await nothing, and add no entries.
            let awaits = if records.iter().any(|&(_, awaited)| awaited) {
                let start = position(self.path_awaits.len());
                self.path_awaits
                    .extend(records.iter().map(|&(_, awaited)| awaited));
                start
            } else {
                PathResolution::NOT_AWAITED
            };
            self.item_paths[id.index()] = PathResolution {
                source,
                existing: position(existing),
                awaits,
            };
        }
        self.path_types.truncate(base);
        ty
    }

    /// The type of the list `list`, at `prefix` of the path `segments` of a
    /// `:=`, once its element is replaced by one of type `element`: `list`
    /// itself, present, since a list's type never changes, if the element
    /// is a subtype of its element type, as it is whenever the element type
    /// is present or the either shape. Otherwise, as for elements declared
    /// exactly `Future<{a: Int}>` of a path continuing through one, reports
    /// "cannot rebind `xs[0].a`: the elements of `xs` are `Future<{a:
    /// Int}>`, and the update makes one `{a: Int}`", at the path, and
    /// returns `Error`.
    fn updated_list(
        &mut self,
        segments: &[PathSegment],
        prefix: &[PathSegment],
        (list, element): (TyId, TyId),
    ) -> TyId {
        let Some(declared) = self.types.list_element(list) else {
            return TyId::ERROR;
        };
        if self.types.is_subtype(element, declared) {
            return list;
        }
        let message = format!(
            "cannot rebind `{}`: the elements of `{}` are {}, and the update makes one {}",
            self.ast.show_path(segments),
            self.ast.show_path(prefix),
            self.in_message(declared),
            self.in_message(element)
        );
        self.errors
            .push(Diagnostic::error(message, path_span(segments)).with_note(
                "a list keeps its element type; declare the elements with a type that allows both",
            ));
        TyId::ERROR
    }

    /// As the input starts: finds what its top-level items mention (see
    /// [`crate::mentions`]) and declares each top-level function, in source
    /// order, with its declared type, so the bindings of the input's
    /// top-level functions take its first slots and come first among its
    /// bindings (see [Top-level functions](#top-level-functions)).
    fn declare_top_fns(&mut self) {
        self.mentions = match self.workspace {
            Some(workspace) => workspace.file().mentions.clone(),
            None => mentions::find(self.ast),
        };
        self.find_def_types();
        self.top_start = self.bindings.len();
        for index in 0..self.mentions.functions.len() {
            let Some(&top) = self.mentions.functions.get(index) else {
                break;
            };
            let kind = match self.ast.item(top.item).kind {
                ItemKind::Field { .. } => BindingKind::Field,
                _ => BindingKind::Variable,
            };
            let span = self.ast.item(top.item).span;
            // A `def` named by a dotted path has no binding: its leaf's
            // cell, a slot after the roots', holds it, and is typed by it
            // (`Checker::declare_field_cells`).
            let binding = match top.name {
                None => usize::MAX,
                Some(name) => {
                    if kind == BindingKind::Variable {
                        self.check_directory_clash(top.item, name);
                    }
                    // A `fn` named like a shadowable field root of the input
                    // shadows it in the whole input: its binding is declared
                    // after the roots' (`Checker::declare_shadowing_fns`).
                    if kind == BindingKind::Variable && self.shadowable_root(name) {
                        usize::MAX
                    } else {
                        let ty = self.declared_type(top.function);
                        let binding = self.bindings.len();
                        self.declare_binding(name, ty, Some(span), kind);
                        binding
                    }
                }
            };
            let leaf = match top.name {
                None => self
                    .mentions
                    .tree
                    .item_leaf
                    .get(top.position)
                    .copied()
                    .flatten(),
                Some(_) => None,
            };
            self.top_fns.push(TopFn {
                item: top.item,
                expr: top.expr,
                function: top.function,
                name: top.name,
                binding,
                leaf,
                node: NO_NODE,
                cut: None,
                state: BodyState::Unchecked,
                ready: 0,
                blocker: None,
                made: MadeAt::Own,
            });
        }
        self.top_copies = vec![Vec::new(); self.top_fns.len()];
        self.declare_field_cells();
        self.type_dotted_defs();
        self.closure_ready = vec![None; self.mentions.node_count() + self.top_fns.len()];
    }

    /// As the input starts, once the cells are declared: gives the cell of
    /// each `def` named by a dotted path the type its signature declares,
    /// which a use of it sees until its group is checked, as a plain
    /// function's binding has (see [Top-level
    /// functions](#top-level-functions)).
    fn type_dotted_defs(&mut self) {
        // A definition made with a def-type has the type the def-type gives
        // it, which may be a dotted `def` of the input: those after the
        // others (see [Def-types](#def-types)).
        for definitions in [false, true] {
            for index in 0..self.top_fns.len() {
                let Some(top) = self.top_fns.get(index).copied() else {
                    continue;
                };
                let Some(leaf) = top.leaf else {
                    continue;
                };
                let defmeta = self.defmetas.get(&top.function).copied();
                let ty = match (self.definitions.get(&top.function), defmeta, definitions) {
                    (Some(&def), _, true) => self.definition_type(def),
                    // A meta is the function its def-type's meta makes.
                    (None, Some(def), true) => self.defmeta_type(def),
                    (None, None, false) => self.declared_type(top.function),
                    _ => continue,
                };
                if let Some(cell) = self.field_cells.get_mut(leaf as usize) {
                    cell.ty = Some(ty);
                }
            }
        }
    }

    /// As the input starts: finds which function expressions of its
    /// outermost block are definitions made with a def-type, which are
    /// definitions' metas, and which are `def`s marked `#[def_type]` (see
    /// [Def-types](#def-types)).
    fn find_def_types(&mut self) {
        let ast = self.ast;
        for &id in ast.items(ast.root()) {
            match ast.item(id).kind {
                ItemKind::DefTypeDef { def, value } => {
                    if let ExprKind::Fn(function) = ast.get(value).kind {
                        self.definitions.insert(function, def);
                    }
                }
                ItemKind::DefMeta { def, value } => {
                    if let ExprKind::Fn(function) = ast.get(value).kind {
                        self.defmetas.insert(function, def);
                    }
                }
                ItemKind::Field {
                    value,
                    function: true,
                    ..
                } if ast.is_def_type(id) => {
                    if let ExprKind::Fn(function) = ast.get(value).kind {
                        self.def_type_fns.insert(function);
                    }
                }
                _ => {}
            }
        }
    }

    /// The type of the definition `def` made with a def-type, before its
    /// body is checked: the function type the def-type's second parameter
    /// has, read quietly, or `Error` if its def-type is not one (which its
    /// own check reports).
    fn definition_type(&mut self, def: DefTypeId) -> TyId {
        let (errors, warnings) = (self.errors.len(), self.warnings.len());
        let dt = self.ast.def_type(def).dt;
        let ty = self.check_expr(dt);
        self.errors.truncate(errors);
        self.warnings.truncate(warnings);
        self.def_type_signature(ty)
            .map_or(TyId::ERROR, |(function, _)| function)
    }

    /// The type of the definition's meta `def`, before its body is checked:
    /// the function of no parameters, of the effect of its marker, returning
    /// its type, that the meta of the function type its def-type gives its
    /// definitions makes, read quietly; `Error` if its def-type is not one or
    /// declares no meta, which its own check reports (see [Metas](#metas)).
    fn defmeta_type(&mut self, def: DefTypeId) -> TyId {
        let function = self.definition_type(def);
        match self.types.fn_sig(function).and_then(|sig| sig.meta) {
            Some(meta) => {
                let none: [(Option<Name>, TyId); 0] = [];
                self.types.fn_type(none, meta.ty, meta.effect)
            }
            None => TyId::ERROR,
        }
    }

    /// The function type of the definitions a def-type of type `ty` makes,
    /// its second parameter's, present, and its own signature, if it is the
    /// type of a def-type: a function whose first parameter is a `String`
    /// and whose second is a function, present or not (see
    /// [Def-types](#def-types)).
    fn def_type_signature(&self, ty: TyId) -> Option<(TyId, FnTy)> {
        let sig = self.types.fn_sig(self.types.strip(ty))?.clone();
        let (&(_, name), &(_, function)) = (sig.params.first()?, sig.params.get(1)?);
        let function = self.types.strip(function);
        (self.types.strip(name) == TyId::STRING && self.types.fn_sig(function).is_some())
            .then_some((function, sig))
    }

    /// Whether `name` is the root of a top-level field of the input all of
    /// whose leaves are marked `#[shadowable]`, which a top-level `let` or
    /// `fn` of the input shadows silently (see [Shadowed
    /// declarations](#shadowed-declarations)).
    fn shadowable_root(&self, name: Name) -> bool {
        let tree = &self.mentions.tree;
        tree.root(name)
            .and_then(|root| tree.nodes.get(root as usize))
            .is_some_and(|root| root.shadowable)
    }

    /// The binding of the `def` of one name whose item is `leaf`'s, if it
    /// is one: its cell is its function's binding. A `def` named by a
    /// dotted path has none, and its cell is a slot of its own.
    fn plain_def_binding(&self, leaf: Option<FieldLeaf>) -> Option<usize> {
        let leaf = leaf?;
        self.top_fns
            .iter()
            .find(|top| top.item == leaf.item && top.name.is_some())
            .map(|top| top.binding)
    }

    /// As the input starts, once its field roots are declared: declares each
    /// top-level `fn` named like a shadowable field root of the input
    /// ([`Checker::shadowable_root`]), so that its binding, later than the
    /// root's, is the one its name finds in the whole input.
    fn declare_shadowing_fns(&mut self) {
        for index in 0..self.top_fns.len() {
            let Some(top) = self.top_fns.get(index).copied() else {
                continue;
            };
            let (usize::MAX, Some(name)) = (top.binding, top.name) else {
                continue;
            };
            let ty = self.declared_type(top.function);
            let binding = self.bindings.len();
            let span = self.ast.item(top.item).span;
            self.declare_binding(name, ty, Some(span), BindingKind::Variable);
            if let Some(entry) = self.top_fns.get_mut(index) {
                entry.binding = binding;
            }
        }
    }

    /// As the input starts, after its top-level functions: declares the
    /// root of every top-level field that is not a `def` (a `def`'s is its
    /// function's binding), in order of first item, with no type yet, and
    /// gives every other node of the tree a slot of its own after them,
    /// which no name finds (see [Top-level fields](#top-level-fields)).
    /// Finds when each node is computed.
    fn declare_field_cells(&mut self) {
        self.schedule = match self.workspace {
            Some(workspace) => workspace.file().schedule.clone(),
            None => self.mentions.schedule(),
        };
        self.scheduled = 0;
        let count = self.mentions.tree.nodes.len();
        if let Some(workspace) = self.workspace {
            self.declare_workspace_cells(workspace);
            return;
        }
        self.field_cells = Vec::with_capacity(count);
        let root_items = self.ast.items(self.ast.root());
        for index in 0..count {
            let Some(node) = self.mentions.tree.nodes.get(index) else {
                break;
            };
            let (name, parent, first) = (node.name, node.parent, node.first);
            let def = self.plain_def_binding(node.leaf);
            let cell = match (parent, def) {
                (_, Some(binding)) => FieldCell {
                    slot: self.bindings.get(binding).map_or(Slot::NONE, |b| b.slot),
                    binding: Some(binding),
                    ..FieldCell::NEW
                },
                (None, None) => {
                    let span = root_items.get(first).map(|&item| self.ast.item(item).span);
                    let binding = self.bindings.len();
                    let slot = self.declare_binding(name, TyId::ERROR, span, BindingKind::Field);
                    FieldCell {
                        slot,
                        binding: Some(binding),
                        ..FieldCell::NEW
                    }
                }
                (Some(_), None) => FieldCell::NEW,
            };
            self.field_cells.push(cell);
        }
        self.declare_shadowing_fns();
        self.top_end = self.bindings.len();
        self.report_cycles();
        // The cells of the nodes that are not roots, after the roots'.
        for index in 0..count {
            let named = self
                .mentions
                .tree
                .nodes
                .get(index)
                .is_some_and(|node| node.parent.is_none());
            if named {
                continue;
            }
            let slot = Slot(self.next_slot);
            self.next_slot = self.next_slot.saturating_add(1);
            self.frame_size = self.frame_size.max(self.next_slot);
            let name = self.mentions.tree.nodes.get(index).map(|node| node.name);
            if let Some(name) = name {
                self.slot_names.push(name);
            }
            if let Some(cell) = self.field_cells.get_mut(index) {
                cell.slot = slot;
            }
        }
        self.top_slots_end = self.top_slots_end.max(self.next_slot);
    }

    /// As the check ends: what a call of each of this input's functions in
    /// a later input, whose tree is gone, needs to know of its parameters
    /// ([`KnownParams`]: what the call-site form of the implicit-await error
    /// needs, and the defaults), for every binding of the outermost scope.
    fn known(&mut self) -> Vec<Option<Rc<KnownParams>>> {
        let mut known = std::mem::take(&mut self.scope_known);
        for binding in self.bindings.get(known.len()..).unwrap_or_default() {
            known.push(binding.function.and_then(|function| {
                let attributes = self
                    .lowering
                    .fn_attributes
                    .get(&function)
                    .map_or(&[][..], |a| a);
                self.fn_resolutions
                    .get(function.index())
                    .map(|resolution| KnownParams::of(resolution, attributes))
            }));
        }
        known
    }

    /// As the check ends: the type of each node's cell (`Error` for a
    /// `def`'s), and, for a file of a workspace, the children of each node
    /// of its tree that it shares with other files, in the merged node's
    /// order, with their cells and types, its own children's as this check
    /// found them; empty outside a workspace (see [Directory
    /// scope](#directory-scope)).
    fn cell_tables(&self) -> (Vec<TyId>, SharedChildren) {
        let tree = self.mentions.tree.nodes.len();
        let types = self.cell_types();
        let Some(workspace) = self.workspace else {
            return (types, Vec::new());
        };
        let shared = (0..tree)
            .map(|node| {
                let node = u32::try_from(node).unwrap_or(u32::MAX);
                let children = workspace.merged_children(node).into_iter();
                children
                    .map(|(name, own, slot, ty)| match own {
                        Some(own) => self
                            .field_cells
                            .get(own as usize)
                            .map_or((name, slot, TyId::ERROR), |cell| {
                                (name, cell.slot, cell.ty.unwrap_or(TyId::ERROR))
                            }),
                        None => (name, slot, ty),
                    })
                    .collect()
            })
            .collect();
        (types, shared)
    }

    /// The type of each node's cell of the input's tree of top-level fields,
    /// by its index there, as the check has found it so far: a `def`'s is
    /// its function's binding's.
    fn cell_types(&self) -> Vec<TyId> {
        let tree = self.mentions.tree.nodes.len();
        self.field_cells
            .iter()
            .take(tree)
            .map(|cell| {
                cell.ty
                    .filter(|&ty| ty != TyId::ERROR)
                    .or_else(|| Some(self.bindings.get(cell.binding?)?.ty))
                    .unwrap_or(TyId::ERROR)
            })
            .collect()
    }

    /// [`Checker::declare_field_cells`] for a file of a workspace: each
    /// node's cell is the slot of its merged node, allocated by the
    /// workspace, a `def`'s excepted (its function's binding's); each root
    /// that is not a `def` is bound in its cell; and each node shared with
    /// another file takes a private slot for the file's own record of it
    /// (see [Directory scope](#directory-scope)).
    fn declare_workspace_cells(&mut self, workspace: &'a Context<'a>) {
        let count = self.mentions.tree.nodes.len();
        let file = workspace.file();
        self.field_cells = Vec::with_capacity(count);
        self.own_records = vec![Slot::NONE; count];
        let root_items = self.ast.items(self.ast.root());
        for index in 0..count {
            let Some(node) = self.mentions.tree.nodes.get(index) else {
                break;
            };
            let (name, parent, first) = (node.name, node.parent, node.first);
            let global = file.global.get(index).copied().unwrap_or(u32::MAX);
            let slot = workspace.slot(global);
            let def = self.plain_def_binding(node.leaf);
            let cell = match (parent, def) {
                (_, Some(binding)) => FieldCell {
                    slot: self.bindings.get(binding).map_or(Slot::NONE, |b| b.slot),
                    binding: Some(binding),
                    ..FieldCell::NEW
                },
                (None, None) => {
                    let binding = self.bindings.len();
                    self.bind(Binding {
                        name,
                        slot,
                        ty: TyId::ERROR,
                        declared: root_items.get(first).map(|&item| self.ast.item(item).span),
                        kind: BindingKind::Field,
                        function: None,
                        callback: None,
                    });
                    FieldCell {
                        slot,
                        binding: Some(binding),
                        ..FieldCell::NEW
                    }
                }
                (Some(_), None) => FieldCell {
                    slot,
                    ..FieldCell::NEW
                },
            };
            // A session's input's `def` takes no slot of its own.
            if slot != Slot::NONE {
                self.frame_size = self.frame_size.max(slot.0.saturating_add(1));
            }
            self.field_cells.push(cell);
            if workspace.shared(global) {
                let own = Slot(self.next_slot);
                self.next_slot = self.next_slot.saturating_add(1);
                self.frame_size = self.frame_size.max(self.next_slot);
                self.slot_names.push(name);
                if let Some(entry) = self.own_records.get_mut(index) {
                    *entry = own;
                }
            }
        }
        // The nodes of the file's view with a leaf of the file under them:
        // a private slot each for the record the file assembles, if it
        // reads it whole (see [Imports](#imports)).
        self.view_records = self
            .mentions
            .views
            .iter()
            .map(|view| (Slot::NONE, view.first, Vec::new()))
            .collect();
        for at in 0..self.mentions.views.len() {
            let number = count + at;
            let scheduled = self
                .schedule
                .point
                .get(number)
                .is_some_and(|&point| point != usize::MAX);
            let mut cell = FieldCell::NEW;
            if scheduled && let Some(view) = self.mentions.views.get(at) {
                cell.slot = Slot(self.next_slot);
                self.next_slot = self.next_slot.saturating_add(1);
                self.frame_size = self.frame_size.max(self.next_slot);
                self.slot_names.push(view.name);
            }
            self.field_cells.push(cell);
        }
        self.declare_shadowing_fns();
        self.top_end = self.bindings.len();
        self.report_cycles();
        self.declare_imports(workspace);
        self.top_slots_end = self.top_slots_end.max(self.next_slot);
    }

    /// The cells of the file's imports (see [Imports](#imports)): its
    /// result's cell if it is a value file another file imports, and a
    /// private slot for each record it reads whole through an import, from
    /// the targets its mentions reach.
    fn declare_imports(&mut self, workspace: &'a Context<'a>) {
        if let Some(cell) = workspace.value_slot() {
            self.value_cell = cell;
            self.frame_size = self.frame_size.max(cell.0.saturating_add(1));
        }
        // A shadowed declaration's reads are not assembled for it (see
        // [Shadowed declarations](#shadowed-declarations)).
        let shadowed = &self.mentions.shadowed;
        let mut reached: Vec<Target> = self
            .mentions
            .foreign
            .iter()
            .enumerate()
            .filter(|&(position, _)| shadowed.get(position) != Some(&true))
            .flat_map(|(_, foreign)| foreign)
            .filter_map(|&(need, _)| match need {
                Need::Field(node) => workspace.scope.target(node),
                Need::Function(_) => None,
            })
            .collect();
        // The records under the view's nodes this file assembles at their
        // points are assembled as it starts, as an imported record is.
        let tree = self.mentions.tree.nodes.len();
        for at in 0..self.mentions.views.len() {
            let number = u32::try_from(tree + at).unwrap_or(u32::MAX);
            if self
                .field_cells
                .get(tree + at)
                .is_some_and(|cell| cell.slot != Slot::NONE)
            {
                reached.extend(
                    workspace
                        .scope
                        .view_children(number)
                        .into_iter()
                        .filter_map(|(_, child)| workspace.scope.target(child)),
                );
            }
        }
        for target in reached {
            self.imported_record(workspace, target);
        }
    }

    /// The private slot and type of the record of `target`, read whole
    /// through an import, allocated with its nested records the first time
    /// (see [Imports](#imports)); `None` for a target that is a cell, and
    /// for the `fileroot` module and its members, which have no record.
    fn imported_record(
        &mut self,
        workspace: &'a Context<'a>,
        target: Target,
    ) -> Option<(Slot, TyId)> {
        if matches!(target, Target::Node(_) | Target::Fileroot(_))
            || workspace.value_of(target).is_some()
        {
            return None;
        }
        if let Some(at) = self.imported_targets.iter().position(|&(t, _)| t == target) {
            let slot = self.imported_records.get(at)?.0;
            let ty = self.imported_targets.get(at)?.1;
            return Some((slot, ty));
        }
        let mut fields = Vec::new();
        let mut erroneous = false;
        for (name, child) in workspace.target_fields(target) {
            // A def-type's namespace, or its metas', binds nothing a record
            // holds (see [Def-types](#def-types)).
            if crate::names::is_namespace_key(self.ast.names().get(name)) {
                continue;
            }
            let (slot, ty) = match child {
                Target::Node(node) => (workspace.cell_slot(node), workspace.cell_type(node)),
                part => self
                    .imported_record(workspace, part)
                    .unwrap_or((Slot::NONE, TyId::ERROR)),
            };
            erroneous |= ty == TyId::ERROR || slot == Slot::NONE;
            fields.push((name, slot, ty));
        }
        let ty = if erroneous {
            TyId::ERROR
        } else {
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        // Every slot is named, so that `slot_names` stays aligned with the
        // slots. A record is only reached through an import of this file,
        // so a name is always found; without one, no slot is taken and the
        // reference is left unresolved rather than misnaming later slots.
        let first = fields.first().map(|&(name, _, _)| name);
        let name = workspace.target_name(target).or(first)?;
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        self.slot_names.push(name);
        self.imported_records.push((slot, fields));
        self.imported_targets.push((target, ty));
        Some((slot, ty))
    }

    /// The cell a reference through an import to `target` reads, and its
    /// type: a merged node's cell (a `def`'s is its function's slot), an
    /// imported value file's value, or the record the file assembles of a
    /// directory, a file or a part (see [Imports](#imports)). A record is
    /// only looked up here, never allocated: [`Self::declare_imports`]
    /// allocated one, among the top-level slots, for every whole read the
    /// file's mentions reach, before the walk; allocating one here, inside
    /// a function, would take a slot of the function's frame.
    fn import_cell(&self, workspace: &'a Context<'a>, target: Target) -> Option<(Slot, TyId)> {
        if let Target::Node(node) = target {
            return Some((workspace.cell_slot(node), workspace.cell_type(node)));
        }
        if let Some(cell) = workspace.value_of(target) {
            return Some((workspace.slot(cell), workspace.cell_type(cell)));
        }
        let at = self
            .imported_targets
            .iter()
            .position(|&(t, _)| t == target)?;
        let slot = self.imported_records.get(at)?.0;
        let ty = self.imported_targets.get(at)?.1;
        Some((slot, ty))
    }

    /// Records that the expression `id`, a reference through an import,
    /// reads `slot`: a `Load` at the top level, a `Top` inside a function.
    fn read_import(&mut self, id: ExprId, slot: Slot) {
        self.expr_vars[id.index()] = if self.functions.is_empty() {
            VarRef::Local(slot)
        } else {
            VarRef::Fn(slot)
        };
    }

    /// The type of the reference `id` to `node`, a node of another file of
    /// the workspace in the numbering of the file's scope, if `id` is a path
    /// through its root that nothing of this file shadows: read from the
    /// node's slot as a cell of this file is, the other file having been
    /// checked and run before this one. A root alone is read through its
    /// committed binding, as a variable of an earlier input is.
    fn foreign_reference(
        &mut self,
        workspace: &'a Context<'a>,
        node: u32,
        id: ExprId,
    ) -> Option<TyId> {
        let mut base = id;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let shadowed = self
            .lookup(name)
            .is_some_and(|(index, _)| index >= self.top_end);
        if shadowed {
            return None;
        }
        // Through an import, a root alone too, which has no binding (see
        // [Imports](#imports)).
        let (slot, ty) = match workspace.scope.target(node) {
            Some(Target::Fileroot(member)) => {
                return Some(self.fileroot_reference(id, (base, name), member));
            }
            Some(target) => {
                if let Target::Node(merged) = target {
                    self.imported_callees.insert(id, merged);
                }
                match self.import_cell(workspace, target) {
                    Some(cell) => cell,
                    // A record only a shadowed declaration reads is not
                    // assembled: erroneous for it, which is never run.
                    None if self.checking_shadowed => (Slot::NONE, TyId::ERROR),
                    None => return None,
                }
            }
            None if base == id => return None,
            None => {
                // A `def`'s cell is its function's slot, and now typed as
                // its function, so never its merged node's unwritten one.
                let global = workspace.scope.merged(node)?;
                // Called, a `def` named by a dotted path of another file of
                // the chain is a known callee, as one imported is.
                self.imported_callees.insert(id, global);
                (workspace.cell_slot(global), workspace.cell_type(global))
            }
        };
        self.read_cell(id, base, slot, ty);
        Some(ty)
    }

    /// The type of the reference `id`, through `base`, the name `name` that
    /// a whole import of the `fileroot` module binds, to the module's member
    /// `member`, or to the module itself (`None`), as nothing nearer shadows
    /// the name (see [Imports](#imports)): `String` for `this`, `workspace`
    /// and `caller`, which lowering makes a constant or a read of the
    /// running frame's caller; for `helper` as the callee of a call
    /// (`call_site`), a pure function of nothing to `{}`, to which the
    /// call's arguments are bound as any function's. The module read whole
    /// is "`fileroot`'s members are read as `fileroot.caller`", and `helper`
    /// anywhere but as a callee "`fileroot.helper` is not a value", each
    /// naming the module by the name the file imports it as. The module's
    /// own name has no value: it is typed `{}`, which nothing reads.
    fn fileroot_reference(
        &mut self,
        id: ExprId,
        (base, name): (ExprId, Name),
        member: Option<Fileroot>,
    ) -> TyId {
        let written = self.ast.names().get(name).to_owned();
        let span = self.ast.get(id).span;
        self.expr_types[base.index()] = TyId::EMPTY;
        match member {
            None => {
                self.errors.push(
                    Diagnostic::error(
                        format!(
                            "`{written}`'s members are read as `{written}.{}`",
                            Fileroot::Caller.name()
                        ),
                        span,
                    )
                    .with_note(FILEROOT_MEMBERS),
                );
                TyId::ERROR
            }
            Some(Fileroot::Helper) if self.call_site == Some(id) => {
                self.expr_vars[id.index()] = VarRef::Fileroot(Fileroot::Helper);
                let none: [(Option<Name>, TyId); 0] = [];
                self.types.fn_type(none, TyId::EMPTY, Effect::Pure)
            }
            Some(Fileroot::Helper) => {
                let helper = Fileroot::Helper.name();
                self.errors.push(
                    Diagnostic::error(format!("`{written}.{helper}` is not a value"), span)
                        .with_note(format!(
                            "it is only ever called, as `{written}.{helper}()`, which makes the \
                             calls after it pass on this function's caller"
                        )),
                );
                TyId::ERROR
            }
            Some(member) => {
                self.expr_vars[id.index()] = VarRef::Fileroot(member);
                TyId::STRING
            }
        }
    }

    /// The type of the field access `id`, `record.name`, if `record` is a
    /// name that a whole import of the `fileroot` module binds, which
    /// nothing nearer shadows, and `name` is none of its members: the error
    /// "`fileroot` has no `nope`", naming the module as the file imports it,
    /// for any field the module lacks (a member reaches
    /// [`Checker::fileroot_reference`] instead).
    fn fileroot_lacks(&mut self, id: ExprId, record: ExprId, name: Name) -> Option<TyId> {
        let workspace = self.workspace?;
        let ExprKind::Name(module) = self.ast.get(record).kind else {
            return None;
        };
        let (node, at) = workspace.scope.reference(self.ast, record)?;
        if at != record
            || workspace.scope.target(node) != Some(Target::Fileroot(None))
            || self
                .lookup(module)
                .is_some_and(|(index, _)| index >= self.top_end)
        {
            return None;
        }
        let names = self.ast.names();
        let text = names.get(name);
        let members = Fileroot::ALL.map(|member| (member.name(), 0));
        let suggestion = suggest::best(text, members).map(|best| {
            let span = self.ast.written_name(self.ast.get(id).span.end, text);
            (span, best.to_owned())
        });
        self.errors.push(
            Diagnostic::error(
                format!("`{}` has no `{text}`", names.get(module)),
                self.ast.get(id).span,
            )
            .with_note(FILEROOT_MEMBERS)
            .with_suggestion(suggestion),
        );
        self.expr_types[record.index()] = TyId::EMPTY;
        Some(TyId::ERROR)
    }

    /// Before the walk checks the top-level item `id`, a `let`, `let mut`,
    /// `:=` or private `fn` declaring `name`, which is not the root of a
    /// field of this file: "`a` is a field of this directory" if `name` is
    /// a declaration of another file of the directory or of a directory
    /// above (see [Directory scope](#directory-scope)).
    fn check_directory_clash(&mut self, id: ItemId, name: Name) {
        let Some(workspace) = self.workspace else {
            return;
        };
        // A root of this file is the clash of one file, its own errors'; a
        // name of the implicit prelude, or of an import a session's input
        // inherits, yields to the declaration silently; and a name the scan
        // reported a clash or a conflict of is not reported again (see
        // [`crate::workspace`], "Scan errors").
        if workspace.scope.root(name).is_none()
            || self.mentions.tree.root(name).is_some()
            || workspace.scope.yields(name)
            || workspace.scope.inherited(name)
            || workspace.poisoned(name)
        {
            return;
        }
        let text = self.ast.names().get(name);
        self.field_clash = Some(id);
        self.errors.push(
            Diagnostic::error(
                format!("`{text}` is a field of this directory"),
                self.name_span(id),
            )
            .with_note(
                "a field or `def` declared at the top level of a file is visible in every \
                 file of its directory and of the directories below it, so its name cannot \
                 also be a variable",
            ),
        );
    }

    /// Reports each cycle through the tree of top-level fields, once: "`a`
    /// depends on itself", at the item of the cycle's leaf that is first in
    /// the source, with a label on each mention along the cycle back to it
    /// (an interior node has its children, which it does not mention, so
    /// that step has none), and marks every node on it erroneous for its
    /// users (see [Top-level fields](#top-level-fields)).
    fn report_cycles(&mut self) {
        self.cyclic_fns.clear();
        for cycle in self.mentions.cycles() {
            for &member in &cycle {
                match member {
                    Need::Field(node) => {
                        if let Some(cell) = self.field_cells.get_mut(node as usize) {
                            cell.cyclic = true;
                            cell.ready = usize::MAX;
                        }
                    }
                    Need::Function(function) => self.cyclic_fns.push(function),
                }
            }
            let leaf = cycle
                .iter()
                .filter_map(|&member| match member {
                    Need::Field(node) => self
                        .mentions
                        .tree
                        .nodes
                        .get(node as usize)
                        .and_then(|entry| entry.leaf)
                        .map(|leaf| (node, leaf)),
                    Need::Function(_) => None,
                })
                .min_by_key(|&(_, leaf)| leaf.position);
            let Some((start, leaf)) = leaf else {
                continue;
            };
            let start = Need::Field(start);
            // Breadth first inside the cycle, back to its start.
            let count = self.mentions.node_count() + self.top_fns.len();
            let mut parent: Vec<Option<(Need, Option<Span>)>> = vec![None; count];
            let (mut queue, mut head) = (vec![start], 0);
            let mut back = None;
            while let Some(&need) = queue.get(head) {
                head += 1;
                for (next, at) in self.mentions.needs(need) {
                    if !cycle.contains(&next) {
                        continue;
                    }
                    if next == start {
                        back = Some((need, at));
                        break;
                    }
                    if let Some(entry) = parent.get_mut(self.need_index(next))
                        && entry.is_none()
                    {
                        *entry = Some((need, at));
                        queue.push(next);
                    }
                }
                if back.is_some() {
                    break;
                }
            }
            // The steps from the start round to it, in order.
            let mut steps = Vec::new();
            let mut edge = back.map(|(from, at)| (from, start, at));
            while let Some((from, to, at)) = edge {
                steps.push((from, to, at));
                edge = (from != start)
                    .then(|| parent.get(self.need_index(from)).copied().flatten())
                    .flatten()
                    .map(|(up, at)| (up, from, at));
            }
            steps.reverse();
            let text = self.need_text(start);
            let mut error = Diagnostic::error(
                format!("`{text}` depends on itself"),
                self.name_span(leaf.item),
            )
            .with_note("a field cannot be computed from itself, even through a function");
            for (from, to, at) in steps {
                if let Some(at) = at {
                    let label = format!(
                        "`{}` uses `{}` here",
                        self.need_text(from),
                        self.need_text(to)
                    );
                    error = error.with_context(at, label);
                }
            }
            self.errors.push(error);
        }
    }

    /// Before the walk checks the item at `position` of the outermost block
    /// (or, with the number of items, after the last): computes every node
    /// of the tree of top-level fields whose point is there or earlier and
    /// that is not computed yet, in the order of the schedule, which puts
    /// what a node needs first. A leaf's initialiser is checked here,
    /// exactly once, after the bodies of the functions it mentions, in the
    /// view of the top level the walk has reached, which is the bindings
    /// above whichever is higher, the point or the leaf's own item; an
    /// interior node's type is the record of its children's. The walk is
    /// driven by the schedule, never by a reference, so it does not nest.
    fn compute_fields(&mut self, position: usize) {
        while let Some(&node) = self.schedule.order.get(self.scheduled) {
            let point = self
                .schedule
                .point
                .get(node as usize)
                .copied()
                .unwrap_or(usize::MAX);
            if point > position {
                break;
            }
            self.scheduled += 1;
            self.compute_field(node);
        }
    }

    /// Computes the type of the node `node` of the tree of top-level fields
    /// (see [`Checker::compute_fields`]) and gives it to its root's binding.
    fn compute_field(&mut self, node: u32) {
        let Some(entry) = self.mentions.tree.nodes.get(node as usize) else {
            self.compute_view(node);
            return;
        };
        let (leaf, children) = (entry.leaf, entry.children.clone());
        let Some(cell) = self.field_cells.get(node as usize).copied() else {
            return;
        };
        let ty = match leaf {
            // A `def`: its function's binding has its type, which its
            // group's check settles, when something mentions it or after
            // the last item, as for any top-level function. Its body is
            // not checked here: it may read fields computed after this
            // point, since a closure reads them when it runs, not when
            // it is made. A `def` named by a dotted path has no binding:
            // its cell has its type (`Checker::type_dotted_defs`), which
            // its group's check settles likewise.
            Some(leaf) if self.top_fns.iter().any(|top| top.item == leaf.item) => {
                if self.plain_def_binding(Some(leaf)).is_some()
                    && let Some(cell) = self.field_cells.get_mut(node as usize)
                {
                    cell.ty = Some(TyId::ERROR);
                }
                return;
            }
            Some(leaf) => {
                self.check_mentioned(leaf.position);
                let ItemKind::Field {
                    path,
                    ty: annotation,
                    ..
                } = self.ast.item(leaf.item).kind
                else {
                    return;
                };
                let segments = self.ast.path(path);
                // Checked at an item above its own, it reads nothing the
                // top level declares between the two, as a function body
                // checked early does not.
                let early = (self.root_position < leaf.position)
                    .then_some((self.root_position, leaf.position));
                (self.early, self.checking_leaf) = (early, Some(node));
                self.name_callee(segments, leaf.value);
                // TEMPORARY (alias rule): a top-level field of one name, not annotated, may bind a
                // built-in function whose typing is special (see the [inference
                // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
                let alias = segments.len() == 1 && annotation.is_none();
                let outer = alias.then(|| self.alias_site.replace(leaf.value));
                // An annotated leaf has its annotation's type, which is what
                // every reader sees, in this file or another.
                let ty = self.annotated_value(annotation, leaf.value);
                if let Some(outer) = outer {
                    self.alias_site = outer;
                }
                self.callee_name = None;
                (self.early, self.checking_leaf) = (None, None);
                let function = self.direct_function(segments, leaf.value);
                if let Some(binding) = cell.binding.and_then(|b| self.bindings.get_mut(b)) {
                    binding.function = function;
                }
                if let Some(cell) = self.field_cells.get_mut(node as usize) {
                    let blocked = cell
                        .blocker
                        .is_some_and(|blocker| blocker.kind != BlockerKind::Mutable);
                    if blocked || cell.cyclic {
                        cell.ready = usize::MAX;
                    }
                }
                if cell.cyclic { TyId::ERROR } else { ty }
            }
            None => self.interior_type(node, &children, cell.cyclic),
        };
        if let Some(cell) = self.field_cells.get_mut(node as usize) {
            cell.ty = Some(ty);
        }
        if let Some(binding) = cell.binding.and_then(|b| self.bindings.get_mut(b)) {
            binding.ty = ty;
        }
    }

    /// The type of the interior node `node` of the tree of top-level
    /// fields, whose children are `children`, `cyclic` if it is on a
    /// cycle: the record of its children's types, with, in a workspace, its
    /// children of other files; `Error` if one of them is erroneous, or it
    /// is on a cycle.
    fn interior_type(&mut self, node: u32, children: &[u32], cyclic: bool) -> TyId {
        let mut erroneous = false;
        let mut fields = Vec::with_capacity(children.len());
        for &child in children {
            let ty = self
                .field_cells
                .get(child as usize)
                .and_then(|cell| cell.ty)
                .unwrap_or(TyId::ERROR);
            erroneous |= ty == TyId::ERROR;
            if let Some(name) = self.mentions.tree.nodes.get(child as usize) {
                fields.push((name.name, ty));
            }
        }
        // A shared node's children of other files, computed before this
        // file is checked (see [Directory scope](#directory-scope)).
        if let Some(workspace) = self.workspace {
            for (name, _, ty) in workspace.foreign_children(node) {
                erroneous |= ty == TyId::ERROR;
                fields.push((name, ty));
            }
        }
        if erroneous || cyclic {
            TyId::ERROR
        } else {
            self.types.record(fields)
        }
    }

    /// Once the group of the `def` named by a dotted path whose leaf is
    /// `leaf` is checked and has changed its leaf's type: gives each node
    /// above it already computed the type its children now have, and the
    /// roots' bindings theirs, as if its group had been checked before
    /// them, and so each node of the file's view with a leaf of the file
    /// under it (see [Top-level functions](#top-level-functions)). A node
    /// computed later reads the leaf's type as it is then.
    fn retype_above(&mut self, leaf: u32) {
        let mut changed = vec![leaf];
        let mut at = self
            .mentions
            .tree
            .nodes
            .get(leaf as usize)
            .and_then(|entry| entry.parent);
        while let Some(node) = at {
            let Some(entry) = self.mentions.tree.nodes.get(node as usize) else {
                break;
            };
            let (children, parent) = (entry.children.clone(), entry.parent);
            let Some(cell) = self.field_cells.get(node as usize).copied() else {
                break;
            };
            if cell.ty.is_none() {
                break;
            }
            let ty = self.interior_type(node, &children, cell.cyclic);
            if let Some(cell) = self.field_cells.get_mut(node as usize) {
                cell.ty = Some(ty);
            }
            if let Some(binding) = cell.binding.and_then(|b| self.bindings.get_mut(b)) {
                binding.ty = ty;
            }
            changed.push(node);
            at = parent;
        }
        // The view's nodes come parents first, so children are retyped
        // before their parents here.
        let tree = self.mentions.tree.nodes.len();
        for at in (0..self.mentions.views.len()).rev() {
            let number = u32::try_from(tree + at).unwrap_or(u32::MAX);
            let reads_changed = self
                .mentions
                .views
                .get(at)
                .is_some_and(|view| view.children.iter().any(|child| changed.contains(child)));
            let computed = self
                .field_cells
                .get(number as usize)
                .is_some_and(|cell| cell.ty.is_some());
            if reads_changed && computed {
                self.compute_view(number);
                changed.push(number);
            }
        }
    }

    /// Computes the type of the node `node` of the file's view with a leaf
    /// of the file under it (numbered after the tree's nodes), the record
    /// of its children in the view's order, and the record lowering
    /// assembles at its point: each child's cell and type, the file's own
    /// nodes and view nodes as computed by now (the schedule puts them
    /// first), every other one another file's cell or a record the file
    /// assembled as it started (see [Imports](#imports)).
    fn compute_view(&mut self, node: u32) {
        let Some(workspace) = self.workspace else {
            return;
        };
        let tree = self.mentions.tree.nodes.len();
        let mut fields = Vec::new();
        let mut erroneous = false;
        for (name, child) in workspace.scope.view_children(node) {
            let (slot, ty) = if let Some(cell) = self.field_cells.get(child as usize) {
                (cell.slot, cell.ty.unwrap_or(TyId::ERROR))
            } else if let Some(target) = workspace.scope.target(child) {
                self.import_cell(workspace, target)
                    .unwrap_or((Slot::NONE, TyId::ERROR))
            } else if let Some(global) = workspace.scope.merged(child) {
                (workspace.cell_slot(global), workspace.cell_type(global))
            } else {
                (Slot::NONE, TyId::ERROR)
            };
            erroneous |= ty == TyId::ERROR || slot == Slot::NONE;
            fields.push((name, slot, ty));
        }
        let cyclic = self
            .field_cells
            .get(node as usize)
            .is_some_and(|cell| cell.cyclic);
        let ty = if erroneous || cyclic {
            TyId::ERROR
        } else {
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        if let Some(cell) = self.field_cells.get_mut(node as usize) {
            cell.ty = Some(ty);
        }
        if let Some(record) = (node as usize)
            .checked_sub(tree)
            .and_then(|at| self.view_records.get_mut(at))
            && let Some(cell) = self.field_cells.get(node as usize)
        {
            record.0 = cell.slot;
            record.2 = fields;
        }
    }

    /// The type of the reference `id` to `node`, a node of the file's view
    /// with a leaf of the file under it, if `id` is a path through its root
    /// that nothing of this file shadows: read from the private slot the
    /// file assembles it into at its point (see [Imports](#imports)).
    fn view_reference(&mut self, node: u32, id: ExprId) -> Option<TyId> {
        let mut base = id;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let shadowed = self
            .lookup(name)
            .is_some_and(|(index, _)| index >= self.top_end);
        if shadowed {
            return None;
        }
        let cell = self.field_cells.get(node as usize).copied()?;
        let ty = cell.ty.unwrap_or(TyId::ERROR);
        self.read_cell(id, base, cell.slot, ty);
        Some(ty)
    }

    /// Records that the reference `id`, a path whose variable is `base`,
    /// reads the cell `slot` of type `ty`, as a reference through an import
    /// does: each prefix inside it typed as the record of the path below it.
    fn read_cell(&mut self, id: ExprId, base: ExprId, slot: Slot, ty: TyId) {
        self.read_import(id, slot);
        self.expr_vars[base.index()] = self.expr_vars[id.index()];
        let mut at = id;
        let mut inner_ty = ty;
        while let ExprKind::Field { expr, name } = self.ast.get(at).kind {
            inner_ty = if inner_ty == TyId::ERROR {
                TyId::ERROR
            } else {
                self.types.record([(name, inner_ty)])
            };
            self.expr_types[expr.index()] = inner_ty;
            at = expr;
        }
    }

    /// The type a field item's value `value` was given when its leaf was
    /// computed, if it is a top-level field's (see
    /// [`Checker::compute_fields`]): the walk does not check it again.
    fn computed_value(&self, value: ExprId) -> Option<TyId> {
        if !self.functions.is_empty() {
            return None;
        }
        let node = self
            .mentions
            .tree
            .item_leaf
            .get(self.root_position)
            .copied()
            .flatten()?;
        let entry = self.mentions.tree.nodes.get(node as usize)?;
        if entry.leaf.is_none_or(|leaf| leaf.value != value) {
            return None;
        }
        self.field_cells.get(node as usize)?.ty
    }

    /// The type of a reference to a top-level field of the input, `id`,
    /// if it is one: a variable naming a field root that nothing shadows,
    /// or a chain of field accesses directly on one, which means the
    /// longest prefix of its path the tree declares (see [Top-level
    /// fields](#top-level-fields)). The accesses beyond that prefix are
    /// left to the caller as ordinary accesses: this answers only for the
    /// expression that names the node, recording where its value is read.
    fn field_reference(&mut self, id: ExprId) -> Option<TyId> {
        if self.workspace.is_some_and(Context::has_poison) {
            let mut base = id;
            while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
                base = expr;
            }
            if let ExprKind::Name(name) = self.ast.get(base).kind
                && self.reads_poisoned(name, self.lookup(name))
            {
                return Some(TyId::ERROR);
            }
        }
        let (node, at) = match self.workspace {
            Some(workspace) => workspace.scope.reference(self.ast, id)?,
            None => self.mentions.tree.reference(self.ast, id)?,
        };
        if at != id {
            return None;
        }
        if let Some(workspace) = self.workspace
            && node as usize >= self.mentions.tree.nodes.len()
        {
            if (node as usize) < self.mentions.node_count() {
                return self.view_reference(node, id);
            }
            return self.foreign_reference(workspace, node, id);
        }
        let root = self.root_of(node);
        let binding = self.field_cells.get(root as usize)?.binding?;
        let mut base = id;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let (index, found) = self.lookup(name)?;
        if index != binding || node == root {
            return None;
        }
        let cell = self.field_cells.get(node as usize).copied()?;
        let root_source = self.resolve(index, found);
        let source = match root_source {
            VarRef::Fn(_) => VarRef::Fn(cell.slot),
            _ => VarRef::Local(cell.slot),
        };
        // The reference reads the node's cell; its variable, the root's.
        self.expr_vars[id.index()] = source;
        self.expr_vars[base.index()] = root_source;
        let ty = cell.ty.unwrap_or(TyId::ERROR);
        // The prefixes inside the reference are not read: each is typed as
        // the record of the path below it, down to the node.
        let mut at = id;
        let mut inner_ty = ty;
        while let ExprKind::Field { expr, name } = self.ast.get(at).kind {
            inner_ty = if inner_ty == TyId::ERROR {
                TyId::ERROR
            } else {
                self.types.record([(name, inner_ty)])
            };
            self.expr_types[expr.index()] = inner_ty;
            at = expr;
        }
        Some(ty)
    }

    /// For a field `name` of type `ty` that a top-level spread of the
    /// operand `value`, written at `operand`, defines: if `name` is a
    /// top-level field of the input, defines nothing and writes no slot,
    /// holding the name's place in the record if nothing has it yet, and
    /// returns `true`.
    fn spread_over_top_field(
        &mut self,
        (name, ty): (Name, TyId),
        (value, operand): (ExprId, Span),
    ) -> bool {
        let Some(slot) = self.top_declaration_slot(name) else {
            return false;
        };
        // A soft field's name as a dotted root is "already defined" in
        // either order: here the dotted paths came first.
        let tree = &self.mentions.tree;
        if let Some(root) = tree
            .root(name)
            .and_then(|root| tree.nodes.get(root as usize))
            && root.leaf.is_none()
            && let Some(&first) = self.ast.items(self.ast.root()).get(root.first)
            && self.ast.item(first).span.start < operand.start
        {
            let text = self.ast.names().get(name);
            self.errors.push(
                Diagnostic::error(format!("field `{text}` is already defined"), operand)
                    .with_context(self.name_span(first), "first defined here"),
            );
        }
        if self.block_field(name).is_none() {
            self.fields.push(FieldDecl {
                name,
                name_span: operand,
                ty,
                slot,
                open: None,
                spread: Some(value),
                rejected: false,
            });
        }
        self.spread_slots.push(Slot::NONE);
        true
    }

    /// The cell of the input's top-level field named `name`, a `def`
    /// included, if there is one. A private `fn` is not a field: a spread
    /// still defines the record's field of its name (slice 23).
    fn top_declaration_slot(&self, name: Name) -> Option<Slot> {
        let root = self.mentions.tree.root(name)?;
        self.field_cells.get(root as usize).map(|cell| cell.slot)
    }

    /// The binding of the field `name` of type `ty` that the dotted item
    /// `id` opens: the top-level root's, `top_root`, declared as the input
    /// started, whose cell the item takes, or a new one.
    fn open_binding(
        &mut self,
        top_root: Option<(usize, Slot)>,
        (name, ty): (Name, TyId),
        id: ItemId,
    ) -> usize {
        if let Some((binding, slot)) = top_root {
            self.item_slots[id.index()] = slot;
            return binding;
        }
        self.declare(name, ty, id, BindingKind::Field);
        self.bindings.len() - 1
    }

    /// The binding and cell of the input's top-level field root `name`,
    /// declared as the input started, if it is one and not a `def`.
    fn top_root_cell(&self, name: Name) -> Option<(usize, Slot)> {
        let node = self.mentions.tree.root(name)?;
        let cell = self.field_cells.get(node as usize)?;
        Some((cell.binding?, cell.slot))
    }

    /// The root of the node `node` of the tree of top-level fields.
    fn root_of(&self, node: u32) -> u32 {
        let mut at = node;
        while let Some(parent) = self
            .mentions
            .tree
            .nodes
            .get(at as usize)
            .and_then(|entry| entry.parent)
        {
            at = parent;
        }
        at
    }

    /// The type `function`'s parameters and return type declare, as its
    /// signature is written, without inferring anything from its body:
    /// what a use of a top-level function sees before its body is checked.
    /// `Error` if the signature is erroneous, which this reports nothing
    /// of: the function's own check does.
    fn declared_type(&mut self, function: FnId) -> TyId {
        let (errors, warnings) = (self.errors.len(), self.warnings.len());
        let decl = *self.ast.fn_decl(function);
        let (params_mode, ret_mode) = bareness_of(decl.effect);
        let base = self.fn_params.len();
        for param in self.ast.params(decl.params) {
            let ty = self.resolve_type(param.ty, params_mode);
            self.fn_params.push((Some(param.name), ty));
        }
        let ret = decl
            .ret
            .map_or(TyId::ERROR, |ret| self.resolve_type(ret, ret_mode));
        let ty = self.intern_fn_type(base, ret, decl.effect);
        self.fn_params.truncate(base);
        let clean = self.errors.len() == errors && ret != TyId::ERROR;
        self.errors.truncate(errors);
        self.warnings.truncate(warnings);
        if clean { ty } else { TyId::ERROR }
    }

    /// When the walk of the outermost block reaches the item `id`: if it
    /// is a top-level function, which is declared already, records the
    /// bindings above it, which its body sees, gives the item its slot
    /// and, for a `def`, adds the field, and returns `true`; and so for a
    /// definition made with a def-type that is none, which it places in its
    /// namespace, or reports, as such ([`Checker::place_definition`]).
    fn reach_top_fn(&mut self, id: ItemId) -> bool {
        let Some(index) = self.top_fns.iter().position(|top| top.item == id) else {
            // A definition made with a def-type, or a definition's meta, that
            // is no top-level function: one whose `DT` is no def-type, or
            // that the rules of dotted fields reject in its namespace (see
            // [Def-types](#def-types)).
            if let ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } =
                self.ast.item(id).kind
            {
                self.place_definition(id, def, value);
                return true;
            }
            return false;
        };
        let Some(top) = self.top_fns.get_mut(index) else {
            return false;
        };
        top.cut = Some(self.bindings.len());
        let (name, binding) = (top.name, top.binding);
        let Some(name) = name else {
            self.reach_dotted_def(index, id);
            return true;
        };
        self.check_declared_twice(id, name);
        let Some(&Binding { slot, ty, .. }) = self.bindings.get(binding) else {
            return true;
        };
        self.item_slots[id.index()] = slot;
        if let ItemKind::Field { path, .. } = self.ast.item(id).kind {
            let name_span = self
                .ast
                .path(path)
                .first()
                .map_or(self.ast.item(id).span, |segment| segment.span);
            let soft = match self.spreads {
                BlockSpreads::None => None,
                BlockSpreads::Some | BlockSpreads::Rejected => self
                    .block_field(name)
                    .filter(|&index| self.fields[index].spread.is_some()),
            };
            match soft {
                // The `def` overrides the soft field: the field keeps its
                // place and takes the function's slot.
                Some(index) => {
                    let field = &mut self.fields[index];
                    (field.ty, field.slot, field.name_span) = (ty, slot, name_span);
                    field.spread = None;
                }
                None => self.fields.push(FieldDecl {
                    name,
                    name_span,
                    ty,
                    slot,
                    open: None,
                    spread: None,
                    rejected: false,
                }),
            }
        }
        true
    }

    /// When the walk of the outermost block reaches the item `id`, the
    /// top-level function `index`, a `def` named by a dotted path: adds its
    /// path to the record of the input as the dotted field it is, its leaf
    /// typed as its function is by now, which its group's check settles if
    /// it has not yet (`Checker::finish_group`), and gives the item its
    /// leaf's cell, which its closure is stored in.
    fn reach_dotted_def(&mut self, index: usize, id: ItemId) {
        let before = self.nodes.len();
        match self.ast.item(id).kind {
            ItemKind::Field { path, value, .. } => {
                let segments = self.ast.path(path);
                if let Some(first) = segments.first().and_then(PathSegment::name) {
                    self.check_name_clash(id, first);
                }
                self.check_dotted_field(id, segments, (None, value), None);
            }
            // A definition made with a def-type: the dotted field of its
            // path under its namespace's key, apart from the input's record;
            // a definition's meta likewise, under its meta key (see
            // [Def-types](#def-types)).
            ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } => {
                self.place_definition(id, def, value);
            }
            _ => return,
        }
        let Some(top) = self.top_fns.get_mut(index) else {
            return;
        };
        // The leaf is the last node the path added.
        if self.nodes.len() > before {
            top.node = u32::try_from(self.nodes.len() - 1).unwrap_or(NO_NODE);
        }
        let slot = top
            .leaf
            .and_then(|leaf| self.field_cells.get(leaf as usize))
            .map_or(Slot::NONE, |cell| cell.slot);
        self.item_slots[id.index()] = slot;
    }

    /// When the walk of the outermost block reaches the definition `id`,
    /// made with a def-type, `def`, of function `value`: places it in its
    /// def-type's namespace as the dotted field of its path under the
    /// namespace's key is placed in a record, among the namespaces' own
    /// fields ([`Checker::def_fields`]), never the input's record, so that
    /// what the rules of dotted fields reject is reported as they report it
    /// ("field `package x` is defined twice"), its value then checked on its
    /// own; or, if its `DT` is no def-type, says so (see
    /// [Def-types](#def-types)).
    fn place_definition(&mut self, id: ItemId, def: DefTypeId, value: ExprId) {
        let Some(segments) = self.definition_path(def) else {
            let dt = self.ast.def_type(def).dt;
            if self.check_expr(dt) != TyId::ERROR {
                self.not_a_def_type(def);
            }
            return;
        };
        let fields = std::mem::replace(&mut self.fields, std::mem::take(&mut self.def_fields));
        let start = std::mem::replace(&mut self.fields_start, 0);
        let spreads = std::mem::take(&mut self.spreads);
        self.check_dotted_field(id, &segments, (None, value), None);
        self.def_fields = std::mem::replace(&mut self.fields, fields);
        self.fields_start = start;
        self.spreads = spreads;
    }

    /// The path of the definition `def` made with a def-type under its
    /// def-type's namespace: the namespace's key, written where its `DT` is,
    /// then its path; for a definition's meta, its leaf, the meta key,
    /// written where its `DT` is, then its path as one name, written where
    /// its path is; `None` if its `DT` is no def-type.
    fn definition_path(&self, def: DefTypeId) -> Option<Vec<PathSegment>> {
        let use_ = self.ast.def_type(def);
        if use_.role == DefTypeRole::Meta {
            let (key, leaf) = use_.meta?;
            let path = path_span(self.ast.path(use_.path));
            return Some(vec![
                PathSegment {
                    kind: SegmentKind::Name(key),
                    span: self.ast.get(use_.dt).span,
                },
                PathSegment {
                    kind: SegmentKind::Name(leaf),
                    span: path,
                },
            ]);
        }
        let key = PathSegment {
            kind: SegmentKind::Name(use_.key?),
            span: self.ast.get(use_.dt).span,
        };
        Some(
            std::iter::once(key)
                .chain(self.ast.path(use_.path).iter().copied())
                .collect(),
        )
    }

    /// Reports the def-type definition or reference `def`, whose `DT` names
    /// something that is not a def-type: "`package` is not a def-type", at
    /// its `DT`.
    fn not_a_def_type(&mut self, def: DefTypeId) {
        let (shown, _) = self.ast.show_def_type(def);
        let use_ = self.ast.def_type(def);
        let at = self.ast.get(use_.dt).span;
        let mut error = Diagnostic::error(format!("`{shown}` is not a def-type"), at)
            .with_note("a def-type is a `def` marked `#[def_type]`");
        // A reference may be two names that meant something else.
        if use_.read.is_some() {
            error = error.with_note(JUXTAPOSED_NOTE);
        }
        self.errors.push(error);
    }

    /// Types the function expression `id`, of function `function`, the
    /// definition `def` made with a def-type, and checks its body (see
    /// [Def-types](#def-types)): its type is the function type of its
    /// def-type's second parameter, present, whose parameters' types its
    /// parameters, written by name alone, take in order, whose result its
    /// body is checked against, and whose effect it has; a count of
    /// parameters other than that type's is "`package` defines functions of
    /// no parameters" (or "of 2 parameters, `a` and `b`"), at its parameter
    /// list. `Error` if its def-type is not one, which its own check
    /// reports, or the count is wrong.
    fn check_definition(&mut self, id: ExprId, function: FnId, def: DefTypeId) -> TyId {
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        let use_ = *self.ast.def_type(def);
        let errors = self.errors.len();
        let dt_ty = self.check_expr(use_.dt);
        // A shadowed definition needs nothing, so its def-type, read where
        // its file does not otherwise need the def-type's, may not be had:
        // then it is not checked, its types being the def-type's to give
        // (see [Shadowed declarations](#shadowed-declarations)).
        if self.checking_shadowed && dt_ty == TyId::ERROR {
            self.errors.truncate(errors);
            return TyId::ERROR;
        }
        let signature = self.def_type_signature(dt_ty).and_then(|(ty, _)| {
            let sig = self.types.fn_sig(ty)?.clone();
            Some((ty, sig))
        });
        let Some((ty, sig)) = signature else {
            return TyId::ERROR;
        };
        let params = self.ast.params(decl.params);
        if params.len() != sig.params.len() {
            self.definition_arity(def, &sig);
            return TyId::ERROR;
        }
        let base = self.fn_params.len();
        for (param, &(_, param_ty)) in params.iter().zip(sig.params.iter()) {
            self.fn_params.push((Some(param.name), param_ty));
        }
        self.expr_types[id.index()] = ty;
        let span = self.ast.get(id).span;
        let dt_span = self.ast.get(use_.dt).span;
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: sig.effect,
            marker: dt_span,
            keyword: dt_span,
            demanded: vec![None; params.len()],
            result: Some((sig.ret, dt_span)),
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        self.check_fn_body(&decl, base, (None, ty, span), sig.ret);
        self.fn_params.truncate(base);
        self.inherit_attributes(function, use_.dt);
        // Its meta's cell, read as its body reads a top-level function.
        self.definition_meta(function, def, sig.meta);
        self.finish_function(decl.params.len(), None);
        ty
    }

    /// Gives the definition of function `function` the attributes of its
    /// def-type, the function `dt` names, as defaults (see
    /// [Attributes](#attributes)): the def-type's, in its order, each
    /// replaced whole by the definition's own of the same attribute, then
    /// the definition's others, in theirs. A def-type known by no tree of
    /// this input is known by what its cell carries ([`KnownParams`]).
    fn inherit_attributes(&mut self, function: FnId, dt: ExprId) {
        let inherited: Vec<Attribute> = match self.known_callee(dt) {
            Some(KnownCallee::Current(def_type)) => self
                .lowering
                .fn_attributes
                .get(&def_type)
                .map(|a| a.to_vec())
                .unwrap_or_default(),
            Some(KnownCallee::Earlier(index)) => self
                .scope_known
                .get(index)
                .and_then(Option::as_ref)
                .map(|known| known.attributes.to_vec())
                .unwrap_or_default(),
            Some(KnownCallee::Imported(node)) => self
                .workspace
                .and_then(|workspace| workspace.known(node))
                .map(|known| known.attributes.to_vec())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if inherited.is_empty() {
            return;
        }
        let own = self
            .lowering
            .fn_attributes
            .remove(&function)
            .unwrap_or_default();
        let mut merged: Vec<Attribute> = inherited
            .into_iter()
            .map(|default| {
                own.iter()
                    .find(|attribute| attribute.id == default.id)
                    .cloned()
                    .unwrap_or(default)
            })
            .collect();
        for attribute in &own {
            if !merged.iter().any(|kept| kept.id == attribute.id) {
                merged.push(attribute.clone());
            }
        }
        self.lowering
            .fn_attributes
            .insert(function, merged.into_boxed_slice());
    }

    /// Resolves the read of its meta of the definition `def` made with a
    /// def-type, of function `function`, whose def-type declares `meta` for
    /// its definitions, if it does (see [Metas](#metas)): the cell of the
    /// meta of its path that its directory declares, read as a reference to
    /// it is, whose slot the definition's code names, so that reading the
    /// meta of a closure of it calls the closure in it. A definition whose
    /// directory declares none is "`package dev.peios.x` has no `defmeta` in
    /// its directory", at its def-type and path, noted "`package` declares a
    /// meta, `META-> PackageMeta`, for each of its definitions". The read is
    /// typed `{}`, and nothing else is done, when the def-type declares no
    /// meta, when the directory declares none, and for a shadowed
    /// definition, which is never made.
    fn definition_meta(&mut self, function: FnId, def: DefTypeId, meta: Option<Meta>) {
        let use_ = *self.ast.def_type(def);
        let Some(read) = use_.meta_read else {
            return;
        };
        let unread = |checker: &mut Self| {
            let mut at = read;
            loop {
                checker.expr_types[at.index()] = TyId::EMPTY;
                match checker.ast.get(at).kind {
                    ExprKind::Field { expr, .. } => at = expr,
                    _ => break,
                }
            }
        };
        let Some(meta) = meta else {
            unread(self);
            return;
        };
        let unpaired = use_.meta.is_none()
            || self
                .workspace
                .is_some_and(|workspace| workspace.unpaired(def));
        if !unpaired && self.checking_shadowed {
            unread(self);
            return;
        }
        let read_ty = if unpaired {
            None
        } else {
            self.field_reference(read)
        };
        let slot = match self.expr_vars[read.index()] {
            VarRef::Local(slot) | VarRef::Fn(slot) => Some(slot),
            _ => None,
        };
        if let (Some(ty), Some(slot)) = (read_ty, slot) {
            self.expr_types[read.index()] = ty;
            self.metas.push((function, slot));
        } else {
            unread(self);
            self.no_defmeta(def, meta);
        }
    }

    /// Reports the definition `def` made with a def-type whose def-type
    /// declares `meta` for its definitions and whose directory declares none
    /// for it: "`package dev.peios.x` has no `defmeta` in its directory", at
    /// its def-type and path, noted "`package` declares a meta, `META->
    /// PackageMeta`, for each of its definitions", the meta as the def-type's
    /// declaration writes it if the workspace has it, else as its type shows
    /// (see [Metas](#metas)).
    fn no_defmeta(&mut self, def: DefTypeId, meta: Meta) {
        let use_ = *self.ast.def_type(def);
        let (shown, path) = self.ast.show_def_type(def);
        let at = Span::join(
            self.ast.get(use_.dt).span,
            path_span(self.ast.path(use_.path)),
        );
        let written = use_.key.and_then(|key| {
            self.workspace
                .and_then(|workspace| workspace.meta_written(key, self.ast.names()))
        });
        let written = written.unwrap_or_else(|| {
            // Bare as written, its either shapes those the marker makes.
            let ty = if meta.effect == Effect::Pure {
                meta.ty
            } else {
                self.types.present(meta.ty)
            };
            let ty = self.types.display(ty, self.ast.names()).to_string();
            format!("{} {ty}", meta.effect.meta_marker())
        });
        self.errors.push(
            Diagnostic::error(
                format!("`{shown} {path}` has no `defmeta` in its directory"),
                at,
            )
            .with_note(format!(
                "`{shown}` declares a meta, `{written}`, for each of its definitions"
            )),
        );
    }

    /// Types the function expression `id`, of function `function`, the
    /// definition's meta `def`, and checks its body (see [Metas](#metas)):
    /// its type is the function of no parameters, of the effect of its
    /// marker, returning its type, that the meta its def-type declares for
    /// its definitions makes, which its body is checked against, as the
    /// body of such a function. A def-type that declares none is
    /// "`package` declares no meta for its definitions", at the meta's
    /// `defmeta` through its path. `Error` if its def-type is not one, which
    /// its own check reports, or declares no meta.
    fn check_defmeta(&mut self, id: ExprId, function: FnId, def: DefTypeId) -> TyId {
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        let use_ = *self.ast.def_type(def);
        let dt_ty = self.check_expr(use_.dt);
        let Some((definition, _)) = self.def_type_signature(dt_ty) else {
            return TyId::ERROR;
        };
        let Some(meta) = self.types.fn_sig(definition).and_then(|sig| sig.meta) else {
            let (shown, _) = self.ast.show_def_type(def);
            self.errors.push(
                Diagnostic::error(
                    format!("`{shown}` declares no meta for its definitions"),
                    use_.span,
                )
                .with_note(
                    "a def-type declares one by its definition parameter's type, as in `func: \
                     fn(): PackageDef META: PackageMeta`",
                ),
            );
            return TyId::ERROR;
        };
        let none: [(Option<Name>, TyId); 0] = [];
        let ty = self.types.fn_type(none, meta.ty, meta.effect);
        self.expr_types[id.index()] = ty;
        let span = self.ast.get(id).span;
        let dt_span = self.ast.get(use_.dt).span;
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: meta.effect,
            marker: dt_span,
            keyword: dt_span,
            demanded: Vec::new(),
            result: Some((meta.ty, dt_span)),
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let base = self.fn_params.len();
        self.check_fn_body(&decl, base, (None, ty, span), meta.ty);
        self.finish_function(0, None);
        ty
    }

    /// Reports the definition `def` made with a def-type, whose parameters
    /// are not as many as those of `sig`, the function type its def-type
    /// gives it: "`package` defines functions of no parameters", "of 1
    /// parameter, `x`" or "of 2 parameters, `a` and `b`", naming them if the
    /// type names them all, at its parameter list.
    fn definition_arity(&mut self, def: DefTypeId, sig: &FnTy) {
        let (shown, _) = self.ast.show_def_type(def);
        let count = sig.params.len();
        let param_names: Option<Vec<String>> = sig
            .params
            .iter()
            .map(|&(name, _)| Some(format!("`{}`", self.ast.names().get(name?))))
            .collect();
        let counted = match count {
            0 => "no parameters".to_owned(),
            1 => "1 parameter".to_owned(),
            _ => format!("{count} parameters"),
        };
        let named = match param_names.as_deref() {
            Some([only]) => format!(", {only}"),
            Some([init @ .., last]) => format!(", {} and {last}", init.join(", ")),
            _ => String::new(),
        };
        let at = self.ast.def_type(def).span;
        self.errors.push(Diagnostic::error(
            format!("`{shown}` defines functions of {counted}{named}"),
            at,
        ));
    }

    /// Reports the `#[def_type]` `def` of function `function`, of type
    /// `ty`, if its parameters are not those of a def-type: "a def-type takes
    /// the name as a `String` and then the definition as a function, as in
    /// `def package(name: String, func: fn(): T)`", at its parameters (see
    /// [Def-types](#def-types)).
    fn check_def_type_signature(&mut self, function: FnId, ty: TyId) {
        if !self.def_type_fns.contains(&function)
            || ty == TyId::ERROR
            || self.def_type_signature(ty).is_some()
        {
            return;
        }
        let decl = *self.ast.fn_decl(function);
        let params = self.ast.params(decl.params);
        let at = match (params.first(), params.last()) {
            (Some(first), Some(last)) => Span::join(first.span, last.span),
            _ => self.ast.get(decl.body).span,
        };
        let name = self
            .ast
            .items(self.ast.root())
            .iter()
            .find_map(|&item| match self.ast.item(item).kind {
                ItemKind::Field { path, value, .. }
                    if self.ast.get(value).kind == ExprKind::Fn(function) =>
                {
                    Some(self.ast.show_path(self.ast.path(path)).to_string())
                }
                _ => None,
            })
            .unwrap_or_default();
        self.errors.push(Diagnostic::error(
            format!(
                "a def-type takes the name as a `String` and then the definition as a \
                 function, as in `def {name}(name: String, func: fn(): T)`"
            ),
            at,
        ));
    }

    /// Types the def-type reference `id`, `DT PATH`, of `def` (see
    /// [Def-types](#def-types)): the function that calls the def-type with
    /// the definition's path, the definition and its own arguments, of the
    /// type of the def-type's parameters after its second, its result and
    /// its effect. Its `DT` must name a def-type, "`package` is not a
    /// def-type" otherwise, and its path a definition of the def-type's
    /// namespace that the file sees, "`package` has no definition
    /// `dev.peios.nope`", with a note listing those it sees, or, for a path
    /// above some, "`dev.peios` is not a `package` definition", at the path.
    fn check_def_type_ref(&mut self, id: ExprId, def: DefTypeId) -> TyId {
        let use_ = *self.ast.def_type(def);
        let errors = self.errors.len();
        let dt_ty = self.check_expr(use_.dt);
        if dt_ty == TyId::ERROR {
            // Two names in a row, the first undefined, may have meant
            // something else.
            let at = self.ast.get(use_.dt).span;
            for error in self.errors.get_mut(errors..).unwrap_or_default() {
                if error.message.starts_with("undefined variable")
                    && error.span.start >= at.start
                    && error.span.end <= at.end
                {
                    error.notes.push(JUXTAPOSED_NOTE.to_owned());
                }
            }
            return TyId::ERROR;
        }
        let (Some(key), Some(read)) = (use_.key, use_.read) else {
            self.not_a_def_type(def);
            return TyId::ERROR;
        };
        if self.shadows_def_type(use_.dt) {
            self.not_a_def_type(def);
            return TyId::ERROR;
        }
        // What is wrong with a def-type's own parameters is reported where
        // it is declared.
        let Some((definition, sig)) = self.def_type_signature(dt_ty) else {
            return TyId::ERROR;
        };
        // The meta the def-type declares for its definitions, which a
        // reference carries, and which a path past its definition reads.
        let meta = self.types.fn_sig(definition).and_then(|f| f.meta);
        let (shown, path) = self.ast.show_def_type(def);
        let at = self.ast.get(read).span;
        let mut defined = read;
        match self.definition_at(key, read, meta.is_some()) {
            DefinitionAt::Leaf => {}
            DefinitionAt::Past(prefix) => defined = prefix,
            DefinitionAt::Interior => {
                self.errors.push(Diagnostic::error(
                    format!("`{path}` is not a `{shown}` definition"),
                    at,
                ));
                return TyId::ERROR;
            }
            DefinitionAt::Missing(seen) => {
                let note = match seen.as_slice() {
                    [] => format!("`{shown}` has no definitions here"),
                    [only] => format!("the only definition of `{shown}` here is `{only}`"),
                    [init @ .., last] => {
                        let init: Vec<String> = init.iter().map(|d| format!("`{d}`")).collect();
                        format!(
                            "the definitions of `{shown}` here are {} and `{last}`",
                            init.join(", ")
                        )
                    }
                };
                self.errors.push(
                    Diagnostic::error(format!("`{shown}` has no definition `{path}`"), at)
                        .with_note(note),
                );
                return TyId::ERROR;
            }
        }
        // The definition's cell, read as any dotted field's is.
        let read_ty = match self.field_reference(defined) {
            Some(ty) => ty,
            None => self.check_expr(defined),
        };
        self.expr_types[defined.index()] = read_ty;
        if read_ty == TyId::ERROR {
            self.unread_past(read, defined);
            return TyId::ERROR;
        }
        let ty = self.types.fn_type_with_meta(
            sig.params.get(2..).unwrap_or_default().iter().copied(),
            sig.ret,
            sig.effect,
            meta,
        );
        if defined == read {
            self.expr_types[id.index()] = ty;
            return ty;
        }
        self.references_past.push((id, defined, ty));
        let found = self.read_past(def, read, (defined, read_ty));
        self.expr_types[id.index()] = found;
        found
    }

    /// Types the field accesses of the read `read` of the def-type reference
    /// `def` past `defined`, its prefix that reads the definition, of type
    /// `defined_ty`, as the field accesses they are: the first reads the
    /// definition's meta, `package dev.peios.testpkg.versions` the meta's
    /// field `versions`, each further one a field of the one before (see
    /// [Metas](#metas)). The meta's errors name the reference, `package
    /// dev.peios.testpkg`, as reading its meta would (`Checker::meta_read`).
    /// Returns the type of the last.
    fn read_past(&mut self, def: DefTypeId, read: ExprId, defined: (ExprId, TyId)) -> TyId {
        // The accesses past the definition, innermost first.
        let mut accesses = Vec::new();
        let mut at = read;
        while at != defined.0 {
            let ExprKind::Field { expr, name } = self.ast.get(at).kind else {
                break;
            };
            accesses.push((at, name));
            at = expr;
        }
        accesses.reverse();
        let outer = self.reading_past.replace((defined.0, def));
        let (mut record, mut ty) = defined;
        for &(access, name) in &accesses {
            ty = if ty == TyId::ERROR {
                TyId::ERROR
            } else {
                self.field_access(access, (record, ty), name)
            };
            self.expr_types[access.index()] = ty;
            record = access;
        }
        self.reading_past = outer;
        ty
    }

    /// Types the field accesses of the read `read` of a def-type reference
    /// past `defined`, its prefix that reads the definition, as erroneous,
    /// when the definition cannot be read, so that none is left untyped.
    fn unread_past(&mut self, read: ExprId, defined: ExprId) {
        let mut at = read;
        while at != defined {
            self.expr_types[at.index()] = TyId::ERROR;
            let ExprKind::Field { expr, .. } = self.ast.get(at).kind else {
                break;
            };
            at = expr;
        }
    }

    /// Whether the def-type `dt` of a reference names a binding nearer than
    /// the input's top-level declarations, a parameter or a local, which is
    /// then what it names, whatever the workspace found its name names at
    /// the top level (see [Def-types](#def-types)).
    fn shadows_def_type(&self, dt: ExprId) -> bool {
        let mut at = dt;
        while let ExprKind::Field { expr, .. } = self.ast.get(at).kind {
            at = expr;
        }
        let ExprKind::Name(name) = self.ast.get(at).kind else {
            return false;
        };
        self.lookup(name)
            .is_some_and(|(index, _)| index >= self.top_end)
    }

    /// What the path `read` of a def-type reference, a namespace's key,
    /// `key`, and the reference's path, names in the file's view: a
    /// definition, a node above some, or nothing, with the paths of the
    /// definitions under the key it sees (see [Def-types](#def-types)). An
    /// earlier session input's namespace is a binding of the key's, whose
    /// record type says. With `past`, for a def-type that declares a meta,
    /// a definition at a proper prefix of the path is what it names, the
    /// rest of the path read from its meta, since no definition lies under
    /// another (see [Metas](#metas)).
    fn definition_at(&mut self, key: Name, read: ExprId, past: bool) -> DefinitionAt {
        // The path's names, each with the prefix of the read through it.
        let mut path = Vec::new();
        let mut at = read;
        while let ExprKind::Field { expr, name } = self.ast.get(at).kind {
            path.push((name, at));
            at = expr;
        }
        path.reverse();
        let last = path.len().saturating_sub(1);
        if let Some(workspace) = self.workspace
            && let Some(root) = workspace.scope.root(key)
        {
            let scope = &workspace.scope;
            let mut node = root;
            for (index, &(name, prefix)) in path.iter().enumerate() {
                match scope.child(node, name) {
                    Some(child) => node = child,
                    None => return DefinitionAt::Missing(self.definitions_under(root)),
                }
                if past && index < last && scope.is_leaf(node) {
                    return DefinitionAt::Past(prefix);
                }
            }
            return if scope.is_leaf(node) {
                DefinitionAt::Leaf
            } else {
                DefinitionAt::Interior
            };
        }
        let Some((_, binding)) = self.lookup(key) else {
            return DefinitionAt::Missing(Vec::new());
        };
        let mut ty = binding.ty;
        for (index, &(name, prefix)) in path.iter().enumerate() {
            match self.types.field(self.types.strip(ty), name) {
                Some(field) => ty = field,
                None => return DefinitionAt::Missing(self.definitions_of(binding.ty)),
            }
            if past && index < last && self.types.fn_sig(self.types.strip(ty)).is_some() {
                return DefinitionAt::Past(prefix);
            }
        }
        if self.types.fn_sig(self.types.strip(ty)).is_some() {
            DefinitionAt::Leaf
        } else {
            DefinitionAt::Interior
        }
    }

    /// The paths of the definitions at or under the scope's number `root`
    /// of the file's view, a namespace's root, in the view's order, as
    /// written after the def-type.
    fn definitions_under(&self, root: u32) -> Vec<String> {
        let Some(workspace) = self.workspace else {
            return Vec::new();
        };
        let names = self.ast.names();
        let mut found = Vec::new();
        let mut stack: Vec<(u32, String)> = vec![(root, String::new())];
        while let Some((node, path)) = stack.pop() {
            if workspace.scope.is_leaf(node) {
                found.push(path);
                continue;
            }
            for (name, child) in workspace.scope.children(node).into_iter().rev() {
                let text = names.get(name);
                let child_path = if path.is_empty() {
                    text.to_owned()
                } else {
                    format!("{path}.{text}")
                };
                stack.push((child, child_path));
            }
        }
        found
    }

    /// The paths of the definitions of a namespace of type `ty`, an earlier
    /// session input's: the paths of its function-typed leaves, in the
    /// order of its fields' names.
    fn definitions_of(&self, ty: TyId) -> Vec<String> {
        let names = self.ast.names();
        let mut found = Vec::new();
        let mut stack: Vec<(TyId, String)> = vec![(ty, String::new())];
        while let Some((ty, path)) = stack.pop() {
            let ty = self.types.strip(ty);
            let Some(fields) = self.types.record_fields(ty) else {
                found.push(path);
                continue;
            };
            let mut fields: Vec<(&str, TyId)> = fields
                .iter()
                .map(|&(name, field)| (names.get(name), field))
                .collect();
            fields.sort_unstable_by(|a, b| b.0.cmp(a.0));
            for (text, field) in fields {
                let child = if path.is_empty() {
                    text.to_owned()
                } else {
                    format!("{path}.{text}")
                };
                stack.push((field, child));
            }
        }
        found
    }

    /// Checks `value`, the value of a field item that the rules of dotted
    /// fields reject, on its own, unless it is a top-level function's,
    /// whose body its group checks, exactly once.
    fn check_rejected(&mut self, value: ExprId) {
        if !self.top_fns.iter().any(|top| top.expr == value) {
            self.check_alone(value);
        }
    }

    /// Where the item `id`'s name is written: the first segment of its
    /// path, or the item, if it has none.
    fn name_span(&self, id: ItemId) -> Span {
        let item = self.ast.item(id);
        match item.kind {
            ItemKind::Let { path, .. }
            | ItemKind::Field { path, .. }
            | ItemKind::Rebind { path, .. } => self
                .ast
                .path(path)
                .first()
                .map_or(item.span, |segment| segment.span),
            _ => item.span,
        }
    }

    /// When the walk reaches the top-level function `id`, named `name`:
    /// "function `f` is declared twice" if a top-level function above it
    /// has its name, unless both are `def`s, which are fields defined
    /// twice.
    fn check_declared_twice(&mut self, id: ItemId, name: Name) {
        let is_def =
            |ast: &Ast, item: ItemId| matches!(ast.item(item).kind, ItemKind::Field { .. });
        let Some(first) = self
            .top_fns
            .iter()
            .take_while(|top| top.item != id)
            .find(|top| top.name == Some(name))
            .map(|top| top.item)
        else {
            return;
        };
        if is_def(self.ast, first) && is_def(self.ast, id) {
            return;
        }
        let text = self.ast.names().get(name);
        self.errors.push(
            Diagnostic::error(
                format!("function `{text}` is declared twice"),
                self.name_span(id),
            )
            .with_context(self.name_span(first), "first declared here"),
        );
    }

    /// Before the walk checks the top-level item `id`, which is not a
    /// top-level function: "`f` is a function of this file" if it is a
    /// `let`, `let mut`, field or `:=` whose name is a top-level function's,
    /// unless it is a field and the function a `def`, which is a field
    /// defined twice.
    fn check_clash(&mut self, id: ItemId) {
        let Some(name) = root_declares(self.ast, id) else {
            return;
        };
        self.check_name_clash(id, name);
    }

    /// [`Checker::check_clash`] for the item `id`, declaring `name`: a
    /// top-level item that is no top-level function, or a `def` named by a
    /// dotted path, whose path's first segment `name` is, as a dotted
    /// field's is (a function named by a dotted path has no name to clash).
    fn check_name_clash(&mut self, id: ItemId, name: Name) {
        let Some(function) = self
            .top_fns
            .iter()
            .find(|top| top.name == Some(name))
            .map(|top| top.item)
        else {
            self.check_field_clash(id, name);
            return;
        };
        let field = |item: ItemId| matches!(self.ast.item(item).kind, ItemKind::Field { .. });
        if field(id) && field(function) {
            return;
        }
        // A `fn` shadows a shadowable field silently, in the whole input
        // (see [Shadowed declarations](#shadowed-declarations)).
        if field(id) && self.shadowable_root(name) {
            return;
        }
        let text = self.ast.names().get(name);
        self.errors.push(
            Diagnostic::error(
                format!("`{text}` is a function of this file"),
                self.name_span(id),
            )
            .with_context(self.name_span(function), "declared here")
            .with_note(
                "a function declared at the top level is visible in the whole file, so its \
                 name cannot also be a variable or a field",
            ),
        );
    }

    /// The names of the slots a later input of a session must not take, the
    /// [`Scope`]'s: every binding's slot and every slot below
    /// `top_slots_end`, each named by the last binding in it, or as the
    /// checker named it. A slot that no binding owns must have been named in
    /// `slot_names` for the scope to cover it.
    fn scope_slot_names(&self) -> Vec<Name> {
        let top_end = self
            .bindings
            .iter()
            .map(|binding| binding.slot.index().saturating_add(1))
            .chain(std::iter::once(self.top_slots_end as usize))
            .max()
            .unwrap_or_default();
        let mut names = self.slot_names.clone();
        names.truncate(top_end);
        for binding in &self.bindings {
            if let Some(entry) = names.get_mut(binding.slot.index()) {
                *entry = binding.name;
            }
        }
        names
    }

    /// Before the walk checks the top-level item `id`, a `let`, `let mut`,
    /// field or `:=` declaring or rebinding `name`, which no top-level
    /// function has: "`a` is a field of this file" if it is not a field item
    /// and `name` is the root of a top-level field of the input (see
    /// [Top-level fields](#top-level-fields)), labelled on the field's first
    /// item, wherever it is. Field items of one root are the dotted-field
    /// rules' to judge.
    fn check_field_clash(&mut self, id: ItemId, name: Name) {
        if matches!(self.ast.item(id).kind, ItemKind::Field { .. }) {
            return;
        }
        // A `let` shadows a shadowable field silently, from its item on
        // (see [Shadowed declarations](#shadowed-declarations)).
        if matches!(self.ast.item(id).kind, ItemKind::Let { .. }) && self.shadowable_root(name) {
            return;
        }
        let tree = &self.mentions.tree;
        let Some(first) = tree
            .root(name)
            .and_then(|root| tree.nodes.get(root as usize))
            .and_then(|root| self.ast.items(self.ast.root()).get(root.first))
            .copied()
        else {
            self.check_directory_clash(id, name);
            return;
        };
        let text = self.ast.names().get(name);
        self.field_clash = Some(id);
        self.errors.push(
            Diagnostic::error(
                format!("`{text}` is a field of this file"),
                self.name_span(id),
            )
            .with_context(self.name_span(first), "declared here")
            .with_note(
                "a field declared at the top level is visible in the whole file, so its name \
                 cannot also be a variable",
            ),
        );
    }

    /// Before the walk checks the top-level item at `position` in the
    /// outermost block, once the bodies it needs are checked: "`f` cannot
    /// be used here yet" for each top-level function or field it mentions
    /// that needs a function that does not exist, or a leaf that cannot be
    /// computed, before the item (see [When a function may be
    /// used](#when-a-function-may-be-used) and [Top-level
    /// fields](#top-level-fields)). A field item is held to it only when its
    /// leaf is computed at its own item: one computed earlier is the item's
    /// that needed it.
    fn check_uses(&mut self, position: usize) {
        if self.computed_early(position) {
            return;
        }
        let at = |list: &[Vec<u32>], spans: &[Vec<Span>]| {
            let list = list.get(position).map_or(&[][..], Vec::as_slice);
            let spans = spans.get(position).map_or(&[][..], Vec::as_slice);
            list.iter()
                .copied()
                .zip(spans.iter().copied())
                .collect::<Vec<_>>()
        };
        let mut mentioned: Vec<(Need, Span)> = at(&self.mentions.items, &self.mentions.spans)
            .into_iter()
            .map(|(function, span)| (Need::Function(function), span))
            .chain(
                at(&self.mentions.fields, &self.mentions.field_spans)
                    .into_iter()
                    .map(|(node, span)| (Need::Field(node), span)),
            )
            .collect();
        mentioned.sort_by_key(|&(_, span)| span.start);
        let mut errors = Vec::new();
        for (need, span) in mentioned {
            if self.closure_ready(need) > position
                && let Some(error) = self.not_ready(need, position, span)
            {
                errors.push(error);
            }
        }
        self.errors.extend(errors);
    }

    /// The latest item before which something `start` needs (itself, or
    /// what it mentions or has, and so on, not through a node on a cycle)
    /// cannot be had, 0 if nothing it needs is ever blocked; remembered in
    /// `closure_ready`. Everything `start` needs is checked by the time the
    /// walk reaches an item that mentions it, so the answer never changes.
    /// The search reuses what earlier searches found, so each item of a
    /// chain of fields adds one step; only a mention that may be an error
    /// is searched in full, by `Checker::not_ready`.
    fn closure_ready(&mut self, start: Need) -> usize {
        let index = self.need_index(start);
        if let Some(&Some(known)) = self.closure_ready.get(index) {
            return known;
        }
        let mut seen = HashSet::from([index]);
        let mut stack = vec![start];
        let mut latest = 0;
        while let Some(need) = stack.pop() {
            let at = self.need_index(need);
            if at != index
                && let Some(&Some(known)) = self.closure_ready.get(at)
            {
                latest = latest.max(known);
                continue;
            }
            let (ready, blocked) = match need {
                Need::Function(function) => self
                    .top_fns
                    .get(function as usize)
                    .map_or((0, false), |top| (top.ready, top.blocker.is_some())),
                Need::Field(node) => self
                    .field_cells
                    .get(node as usize)
                    .map_or((0, false), |cell| (cell.ready, cell.blocker.is_some())),
            };
            if self.on_cycle(need) {
                continue;
            }
            if blocked {
                latest = latest.max(ready);
            }
            for (next, _) in self.mentions.needs(need) {
                if seen.insert(self.need_index(next)) {
                    stack.push(next);
                }
            }
        }
        if let Some(entry) = self.closure_ready.get_mut(index) {
            *entry = Some(latest);
        }
        latest
    }

    /// Whether the item at `position` of the outermost block, of `kind`,
    /// does not run where it is written: a top-level field whose leaf is
    /// computed before its own item ([`Checker::computed_early`]), or a
    /// `type`, which never runs.
    fn runs_elsewhere(&self, position: usize, kind: ItemKind) -> bool {
        matches!(kind, ItemKind::Type { .. }) || self.computed_early(position)
    }

    /// Whether the item at `position` of the outermost block is a top-level
    /// field whose leaf is computed before its own item.
    fn computed_early(&self, position: usize) -> bool {
        self.mentions
            .tree
            .item_leaf
            .get(position)
            .copied()
            .flatten()
            .and_then(|leaf| self.schedule.point.get(leaf as usize))
            .is_some_and(|&point| point < position)
    }

    /// The index of `need` among the nodes of the graph of what needs what:
    /// the field nodes, then the top-level functions.
    fn need_index(&self, need: Need) -> usize {
        match need {
            Need::Field(node) => node as usize,
            Need::Function(function) => self.mentions.node_count() + function as usize,
        }
    }

    /// The name of `need` in a diagnostic: a function's name, or its path
    /// if it is a `def` named by a dotted path, or a field node's full path.
    fn need_text(&self, need: Need) -> String {
        let names = self.ast.names();
        match need {
            Need::Function(function) => {
                let Some(top) = self.top_fns.get(function as usize) else {
                    return String::new();
                };
                match (top.name, self.ast.item(top.item).kind) {
                    (Some(name), _) => names.get(name).to_string(),
                    (None, ItemKind::Field { path, .. }) => {
                        self.ast.show_path(self.ast.path(path)).to_string()
                    }
                    // A definition's meta, as `defmeta`, its def-type and its
                    // path.
                    (None, ItemKind::DefMeta { def, .. }) => self
                        .ast
                        .def_type(def)
                        .meta
                        .map(|(key, leaf)| crate::names::show_path(names, &[key, leaf]))
                        .unwrap_or_default(),
                    // A definition made with a def-type, as its def-type and
                    // its path are written.
                    (None, ItemKind::DefTypeDef { def, .. }) => {
                        let (shown, path) = self.ast.show_def_type(def);
                        format!("{shown} {path}")
                    }
                    (None, _) => String::new(),
                }
            }
            Need::Field(node) => {
                let path: Vec<&str> = self
                    .mentions
                    .path(node)
                    .into_iter()
                    .map(|name| names.get(name))
                    .collect();
                path.join(".")
            }
        }
    }

    /// Why `need` cannot be had before the item at `position`, if it
    /// cannot: a function whose closure is not made by then, or a leaf
    /// that cannot be computed by then. A node on a cycle, already
    /// reported, is not reported again.
    fn blocker_of(&self, need: Need, position: usize) -> Option<Blocker> {
        let (ready, blocker) = match need {
            Need::Function(function) => {
                let top = self.top_fns.get(function as usize)?;
                (top.ready, top.blocker)
            }
            Need::Field(node) => {
                let cell = self.field_cells.get(node as usize)?;
                (cell.ready, cell.blocker)
            }
        };
        blocker.filter(|_| ready > position && !self.on_cycle(need))
    }

    /// Whether `need` is on a cycle through fields, already reported.
    fn on_cycle(&self, need: Need) -> bool {
        match need {
            Need::Field(node) => self
                .field_cells
                .get(node as usize)
                .is_some_and(|cell| cell.cyclic),
            Need::Function(function) => self.cyclic_fns.contains(&function),
        }
    }

    /// The error for a mention at `span`, by the item at `position`, of the
    /// top-level function or field node `start`, if something it needs
    /// (itself, or what it mentions or has, and so on) does not exist
    /// before the item: the reason first in source order, with the chain of
    /// uses that leads to it.
    fn not_ready(&self, start: Need, position: usize, span: Span) -> Option<Diagnostic> {
        // Breadth first, so the chain to each node is a shortest one.
        let count = self.mentions.node_count() + self.top_fns.len();
        let mut parent: Vec<Option<(Need, bool)>> = vec![None; count];
        *parent.get_mut(self.need_index(start))? = Some((start, false));
        let (mut queue, mut head) = (vec![start], 0);
        let mut culprit: Option<(Need, Blocker)> = None;
        while let Some(&need) = queue.get(head) {
            head += 1;
            if let Some(blocker) = self.blocker_of(need, position)
                && culprit.is_none_or(|(_, first)| blocker.span.start < first.span.start)
            {
                culprit = Some((need, blocker));
            }
            if self.on_cycle(need) {
                continue;
            }
            for (next, at) in self.mentions.needs(need) {
                if let Some(entry) = parent.get_mut(self.need_index(next))
                    && entry.is_none()
                {
                    *entry = Some((need, at.is_none()));
                    queue.push(next);
                }
            }
        }
        let (culprit, blocker) = culprit?;
        // The chain from `start`, each step with whether it is a "has":
        // a run of those is said once, from its first node to its last.
        let mut chain = vec![(culprit, false)];
        while let Some(&(last, _)) = chain.last()
            && last != start
        {
            let (up, has) = (*parent.get(self.need_index(last))?)?;
            if let Some(entry) = chain.last_mut() {
                entry.1 = has;
            }
            chain.push((up, false));
        }
        chain.reverse();
        // Each step's verb now belongs to the node it leaves.
        let mut steps: Vec<(Need, bool)> = Vec::with_capacity(chain.len());
        for window in chain.windows(2) {
            let ((from, _), (_, has)) = (window[0], window[1]);
            steps.push((from, has));
        }
        let mut collapsed: Vec<(Need, bool)> = Vec::with_capacity(steps.len());
        for (from, has) in steps {
            if has && collapsed.last().is_some_and(|&(_, previous)| previous) {
                continue;
            }
            collapsed.push((from, has));
        }
        let text = |need: Need| self.need_text(need);
        let verb = |has: bool| if has { "has" } else { "uses" };
        let uses = match collapsed.as_slice() {
            [] => format!("`{}`", text(culprit)),
            [(first, has), (second, _), ..] if collapsed.len() > 2 => format!(
                "`{}` {} `{}`, ..., and `{}`",
                text(*first),
                verb(*has),
                text(*second),
                text(culprit)
            ),
            _ => {
                let mut said: Vec<String> = Vec::with_capacity(collapsed.len());
                for (index, &(from, has)) in collapsed.iter().enumerate() {
                    let to = collapsed.get(index + 1).map_or(culprit, |&(to, _)| to);
                    said.push(format!("`{}` {} `{}`", text(from), verb(has), text(to)));
                }
                format!("{}, and `{}`", said.join(", "), text(culprit))
            }
        };
        let read = self.ast.names().get(blocker.name);
        let field = matches!(culprit, Need::Field(_));
        let bound = if field {
            "a field can be computed once every variable it reads is bound"
        } else {
            "a function can be used once every variable it reads is bound"
        };
        let (label, note) = match blocker.kind {
            BlockerKind::Bound => (format!("{uses} reads `{read}`, which is bound here"), bound),
            BlockerKind::Spread => (
                format!("{uses} reads `{read}`, which this spread may define"),
                bound,
            ),
            BlockerKind::Mutable if field => (
                format!("{uses} reads the mutable variable `{read}`, so it is computed here"),
                "a field that reads a mutable variable is computed where it is written",
            ),
            BlockerKind::Mutable => (
                format!("{uses} reads the mutable variable `{read}`, so it exists from here"),
                "a function copies a mutable variable where the function is written",
            ),
        };
        Some(
            Diagnostic::error(format!("`{}` cannot be used here yet", text(start)), span)
                .with_context(blocker.span, label)
                .with_note(note),
        )
    }

    /// Before the walk checks the top-level item at `position` in the
    /// outermost block: checks the body of every top-level function the
    /// item mentions that is not checked yet, with the groups they mention
    /// first.
    fn check_mentioned(&mut self, position: usize) {
        let Some(mentioned) = self.mentions.items.get(position).cloned() else {
            return;
        };
        for function in mentioned {
            if let Some(&group) = self.mentions.group_of.get(function as usize) {
                self.check_group(group);
            }
        }
    }

    /// After the last item of the outermost block: checks the bodies not
    /// checked yet, groups callee first, and gives each `def`'s field its
    /// function's type.
    fn check_remaining_bodies(&mut self) {
        for group in 0..self.mentions.groups.len() {
            self.check_group(position(group));
        }
        for top in &self.top_fns {
            let Some(binding) = self.bindings.get(top.binding) else {
                continue;
            };
            if let Some(field) = self.fields[self.fields_start..]
                .iter_mut()
                .find(|field| field.slot == binding.slot)
            {
                field.ty = binding.ty;
            }
        }
    }

    /// Checks the bodies of the group `group` of top-level functions, if
    /// they are not checked yet: first those of the groups it mentions,
    /// then its members', in source order, each exactly once. A use of a
    /// member inside the group sees its declared type; once the group is
    /// checked, every member's binding has the type its body gives it and
    /// its function, as a function declared above its users' has.
    ///
    /// The walk keeps its own stack of the groups being entered, each with
    /// the callee groups still to visit, so that a long chain of functions
    /// does not recurse on the native stack.
    fn check_group(&mut self, group: u32) {
        let mut work: Vec<(u32, Vec<u32>, usize)> = Vec::new();
        if let Some(callees) = self.enter_group(group) {
            work.push((group, callees, 0));
        }
        while let Some((group, callees, next)) = work.last_mut() {
            if let Some(&callee) = callees.get(*next) {
                *next += 1;
                if let Some(callees) = self.enter_group(callee) {
                    work.push((callee, callees, 0));
                }
                continue;
            }
            let group = *group;
            work.pop();
            self.finish_group(group);
        }
    }

    /// Starts checking the group `group`, if none of its members' bodies is
    /// checked or being checked: marks them as being checked and returns
    /// the other groups its members mention, in the order they mention
    /// them, which must be checked first.
    fn enter_group(&mut self, group: u32) -> Option<Vec<u32>> {
        let members = self.mentions.groups.get(group as usize)?.clone();
        if members.iter().any(|&member| {
            self.top_fns
                .get(member as usize)
                .is_none_or(|top| top.state != BodyState::Unchecked)
        }) {
            return None;
        }
        let mut callee_groups = Vec::new();
        for &member in &members {
            if let Some(top) = self.top_fns.get_mut(member as usize) {
                top.state = BodyState::Checking;
            }
            let position = self
                .mentions
                .functions
                .get(member as usize)
                .map_or(usize::MAX, |top| top.position);
            for &callee in self
                .mentions
                .items
                .get(position)
                .map_or(&[][..], Vec::as_slice)
            {
                match self.mentions.group_of.get(callee as usize) {
                    Some(&callee_group) if callee_group != group => {
                        callee_groups.push(callee_group);
                    }
                    _ => {}
                }
            }
        }
        Some(callee_groups)
    }

    /// Checks the bodies of the group `group`, whose callee groups are
    /// checked, members in source order, and gives each member's binding
    /// the type and function its body gives it.
    fn finish_group(&mut self, group: u32) {
        let Some(members) = self.mentions.groups.get(group as usize).cloned() else {
            return;
        };
        for &member in &members {
            self.check_top_body(member as usize);
        }
        for &member in &members {
            let Some(top) = self.top_fns.get_mut(member as usize) else {
                continue;
            };
            top.state = BodyState::Done;
            // A function whose body read a name it cannot see yet is
            // erroneous for its users.
            let (ty, function) = match top.ready {
                usize::MAX => (TyId::ERROR, None),
                _ => (self.expr_types[top.expr.index()], Some(top.function)),
            };
            let (leaf, node) = (top.leaf, top.node);
            if let Some(binding) = self.bindings.get_mut(top.binding) {
                binding.ty = ty;
                binding.function = function;
            }
            // A `def` named by a dotted path: its cell, the nodes computed
            // above it and its node of the input's record take the type.
            if let Some(leaf) = leaf
                && let Some(cell) = self.field_cells.get_mut(leaf as usize)
                && cell.ty != Some(ty)
            {
                cell.ty = Some(ty);
                self.retype_above(leaf);
            }
            if let Some(entry) = self.nodes.get_mut(node as usize) {
                entry.ty = ty;
            }
        }
    }

    /// Checks the body of the top-level function `index` in the view of
    /// the top level it sees: the bindings above its own declaration, or,
    /// before the walk has reached it, above the item being checked. The
    /// bindings below are set aside while it is checked, which is cheap,
    /// since they are a suffix of `bindings`.
    fn check_top_body(&mut self, index: usize) {
        let Some(top) = self.top_fns.get(index).copied() else {
            return;
        };
        let cut = top
            .cut
            .unwrap_or(self.bindings.len())
            .max(self.top_end)
            .min(self.bindings.len());
        let own = self
            .mentions
            .functions
            .get(index)
            .map_or(usize::MAX, |function| function.position);
        self.early = top.cut.is_none().then_some((self.root_position, own));
        self.checking_top = Some(index);
        let below = self.bindings.split_off(cut);
        // A `def` named by a dotted path sees itself by its path only.
        self.callee_name = top.name;
        let found = self.check_expr(top.expr);
        self.settle(top.expr, found);
        self.callee_name = None;
        self.bindings.extend(below);
        (self.early, self.checking_top) = (None, None);
        self.settle_top(index, own);
    }

    /// Once the body of the top-level function `index`, declared at the
    /// position `own` among the outermost block's items, is checked: works
    /// out from what it copies where its closure is made, and so before
    /// which item it exists (see [When a function may be
    /// used](#when-a-function-may-be-used)). What it copies from an
    /// earlier input does not count.
    fn settle_top(&mut self, index: usize, own: usize) {
        let copies = self
            .top_copies
            .get(index)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let bindings = copies
            .iter()
            .filter(|&&binding| binding >= self.top_start)
            .filter_map(|&binding| self.bindings.get(binding).copied());
        let blocked = self
            .top_fns
            .get(index)
            .is_some_and(|top| top.blocker.is_some());
        let (mut ready, mut why, mut made) = (0, None, MadeAt::Start);
        if blocked {
            ready = usize::MAX;
            made = MadeAt::Own;
        } else if let Some(mutable) = bindings
            .clone()
            .find(|binding| binding.kind == BindingKind::Mutable)
        {
            let span = self
                .top_fns
                .get(index)
                .map(|top| self.ast.item(top.item).span);
            ready = own.saturating_add(1);
            made = MadeAt::Own;
            why = span.map(|span| Blocker {
                name: mutable.name,
                span,
                kind: BlockerKind::Mutable,
            });
        } else {
            let root = self.ast.items(self.ast.root());
            for binding in bindings {
                let Some(declared) = binding.declared else {
                    continue;
                };
                let Some((position, &item)) = root.iter().enumerate().find(|&(_, &id)| {
                    let span = self.ast.item(id).span;
                    (span.start..span.end).contains(&declared.start)
                }) else {
                    continue;
                };
                if position + 1 > ready {
                    ready = position + 1;
                    made = MadeAt::After(item);
                    why = Some(Blocker {
                        name: binding.name,
                        span: declared,
                        kind: BlockerKind::Bound,
                    });
                }
            }
        }
        if let Some(top) = self.top_fns.get_mut(index) {
            top.ready = ready;
            top.made = made;
            top.blocker = top.blocker.or(why);
        }
    }

    /// The top-level function whose binding, or for a `def` named by a
    /// dotted path whose leaf's cell, is in `slot`, if one of this input's
    /// is.
    fn top_fn_in(&self, slot: Slot) -> Option<&TopFn> {
        self.top_fns.iter().find(|top| {
            let cell = top
                .leaf
                .and_then(|leaf| self.field_cells.get(leaf as usize))
                .map(|cell| cell.slot);
            cell.or_else(|| self.bindings.get(top.binding).map(|binding| binding.slot))
                == Some(slot)
        })
    }

    /// Declares a variable or field, as `kind` says, for the item `id`, in
    /// the innermost block, in the next free slot, which the item writes.
    fn declare(&mut self, name: Name, ty: TyId, id: ItemId, kind: BindingKind) {
        self.declare_initialised(name, ty, id, (kind, None));
    }

    /// Declares a variable or field, as `kind` says, for the item `id`, as
    /// [`Checker::declare`] does, initialised directly with the function
    /// expression `function`, if it is `Some`.
    fn declare_initialised(
        &mut self,
        name: Name,
        ty: TyId,
        id: ItemId,
        (kind, function): (BindingKind, Option<FnId>),
    ) {
        let span = self.ast.item(id).span;
        let slot = self.declare_binding(name, ty, Some(span), kind);
        if let Some(binding) = self.bindings.last_mut() {
            binding.function = function;
        }
        self.item_slots[id.index()] = slot;
    }

    /// Declares a binding of `kind` in the innermost block, in the next free
    /// slot of the current frame, and returns that slot. `declared` is where
    /// its declaration is written. It is initialised with no known function
    /// expression.
    fn declare_binding(
        &mut self,
        name: Name,
        ty: TyId,
        declared: Option<Span>,
        kind: BindingKind,
    ) -> Slot {
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        self.bind(Binding {
            name,
            slot,
            ty,
            declared,
            kind,
            function: None,
            callback: None,
        });
        slot
    }

    /// Adds `binding` to the innermost block, in its slot, which is taken
    /// already: the next free slot [`Checker::declare_binding`] has just
    /// taken, or the one slot of the field it defines again.
    fn bind(&mut self, binding: Binding) {
        match self.slot_names.get_mut(binding.slot.index()) {
            Some(entry) => *entry = binding.name,
            // Slots are taken in order, so a slot beyond the table is the
            // next one.
            None => self.slot_names.push(binding.name),
        }
        self.record_decl(binding);
        self.bindings.push(binding);
    }

    /// As the check ends, the input's value being of type `ty`: what it
    /// recorded, if it records (see [Recording](#recording)), with the
    /// tables as they are.
    fn finish_record(&mut self, ty: TyId) -> Option<Box<Record>> {
        let mut record = self.record.take()?;
        record.types.clone_from(&self.expr_types);
        record.declared_types.clone_from(&self.declared_types);
        record.cells = self.cell_types();
        record.value = Some(ty);
        Some(record)
    }

    /// Records that the `:=` item `id` rebinds the binding declared at
    /// `replaced`, if the check records (see [Recording](#recording)) and
    /// that was declared in this input.
    fn record_rebind(&mut self, id: ItemId, replaced: Option<Span>) {
        if let Some(record) = &mut self.record
            && let Some(replaced) = replaced
        {
            record.rebinds.push((self.ast.item(id).span, replaced));
        }
    }

    /// Records `binding` as declared, if the check records (see
    /// [Recording](#recording)) and it was declared in this input.
    fn record_decl(&mut self, binding: Binding) {
        if let Some(record) = &mut self.record
            && let Some(declared) = binding.declared
        {
            record.decls.push(RecordedDecl {
                declared,
                name: binding.name,
                ty: binding.ty,
                kind: binding.kind,
            });
        }
    }

    /// The innermost, latest binding of `name`, with its index in
    /// `bindings`.
    fn lookup(&self, name: Name) -> Option<(usize, Binding)> {
        let (index, binding) = self
            .bindings
            .iter()
            .enumerate()
            .rev()
            .find(|(_, binding)| binding.name == name)
            .map(|(index, &binding)| (index, binding))?;
        // A top-level function or field shadows a soft field of its name
        // that a spread of the top level defines, wherever it is read.
        let top_end = self.top_end;
        let root_end = self
            .functions
            .first()
            .map(|function| function.bindings_start)
            .into_iter()
            .chain(self.root_end)
            .min()
            .unwrap_or(self.bindings.len());
        if binding.kind == BindingKind::Field
            && (top_end..root_end).contains(&index)
            && let Some(top) = (self.top_start..top_end)
                .rev()
                .find(|&top| self.bindings.get(top).is_some_and(|b| b.name == name))
        {
            return self.bindings.get(top).map(|&binding| (top, binding));
        }
        Some((index, binding))
    }

    /// Where a use of `binding`, at `index` in `bindings`, reads its value
    /// from in the current function: its slot, or the callee, if it is the
    /// current function's or the top level's own; and otherwise a capture,
    /// which this adds to every function between the binding's home and
    /// the current one that lacks it (see the module documentation).
    fn resolve(&mut self, index: usize, binding: Binding) -> VarRef {
        // A top-level function or field of this input is read through the
        // top level's frame from any function, never captured.
        // So is a declaration of another file of a workspace, from its
        // directory chain: the scope a file is checked from holds nothing
        // else (see [Directory scope](#directory-scope)). A session's input
        // is checked beside the standard library as a file is, but its
        // scope is the session's, read as a session's input reads it.
        let committed = self.workspace.is_some_and(|w| !w.session()) && index < self.top_start;
        if committed || (self.top_start..self.top_end).contains(&index) {
            return if self.functions.is_empty() {
                VarRef::Local(binding.slot)
            } else {
                VarRef::Fn(binding.slot)
            };
        }
        let home = match binding.kind {
            BindingKind::Callee => VarRef::Callee,
            BindingKind::Variable
            | BindingKind::Mutable
            | BindingKind::Header
            | BindingKind::Field
            | BindingKind::Narrowed => VarRef::Local(binding.slot),
        };
        // The outermost function the binding is not in: every function from
        // there inwards must capture it.
        let Some(first) = self
            .functions
            .iter()
            .position(|function| function.bindings_start > index)
        else {
            return home;
        };
        let mut source = home;
        for function in &mut self.functions[first..] {
            let existing = function
                .captures
                .iter()
                .position(|capture| capture.binding == index);
            let capture = existing.unwrap_or_else(|| {
                function.captures.push(Capture {
                    binding: index,
                    source,
                    ty: binding.ty,
                });
                function.captures.len() - 1
            });
            source = VarRef::Capture(position(capture));
        }
        source
    }

    /// The type a type expression denotes, with a bare type meaning what
    /// `mode` says (see the module documentation): in `Either` mode, a type
    /// written as a name, a record type or a list type (`List<T>` or `T[]`)
    /// is the either shape, a record type's fields and a list type's
    /// element type being resolved in `Either` mode too. A function type
    /// is never made the either shape at its own position, whatever the
    /// mode: its interior follows its own purity marker, and a function
    /// value that may be a future is written `Future<fn...>` or `fn... |
    /// Future<fn...>`. Nor are an applied type or a union, which say
    /// themselves what they are. `Error` (reported) if the type names no
    /// type, or is a record type with a field that names none or a name
    /// that appears twice, or a function type with a parameter or return
    /// type that is erroneous, or an applied type or a union that is not
    /// one of those accepted, or contains one that is erroneous.
    ///
    /// A type the expansion of the file's types made (see [Type
    /// declarations](#type-declarations)) is resolved as it was made: one
    /// that names nothing it can is `Error`, its problem reported; a copy of
    /// a declared type's definition is resolved with nothing reported, its
    /// own file's check having reported what is wrong in it, and is `Error`
    /// if anything is; a record type with spreads is the record type of its
    /// merged fields, or `Error`, its spreads' problems reported.
    fn resolve_type(&mut self, id: TypeExprId, mode: Bareness) -> TyId {
        if self.ast.type_expr(id).kind == TypeExprKind::Invalid {
            return self.invalid_type(id);
        }
        // A copy of a definition is resolved once in each mode, however
        // many uses share it, so that a type built from others costs what
        // its definition does, not what its expansion would, written out.
        if self.ast.is_copied(id) {
            if let Some(&resolved) = self.copies.get(&(id, mode)) {
                self.quiet.failed |= resolved == TyId::ERROR;
                return resolved;
            }
            let resolved = self.resolve_quietly(id, mode);
            self.copies.insert((id, mode), resolved);
            return resolved;
        }
        if !self.quiet.types && self.ast.is_definition_copy(id) {
            return self.resolve_quietly(id, mode);
        }
        self.resolve_written(id, mode)
    }

    /// The type the type expression `id`, a copy of a declared type's
    /// definition or a name made one, denotes in `mode`, resolved with
    /// nothing reported (see [`Checker::resolve_type`]): `Error` if
    /// anything would be, the check then failing quietly.
    fn resolve_quietly(&mut self, id: TypeExprId, mode: Bareness) -> TyId {
        let outer = std::mem::replace(&mut self.quiet.types, true);
        let errors = self.errors.len();
        let resolved = self.resolve_written(id, mode);
        self.quiet.types = outer;
        if self.errors.len() > errors {
            self.errors.truncate(errors);
            self.quiet.failed = true;
            return TyId::ERROR;
        }
        resolved
    }

    /// The type `Error` of the type expression `id`, which names nothing it
    /// can ([`TypeExprKind::Invalid`]): its problem, if it is this file's to
    /// report, reported; a check with nothing to report for it fails
    /// quietly (see [Type declarations](#type-declarations)).
    fn invalid_type(&mut self, id: TypeExprId) -> TyId {
        let ast = self.ast;
        let mut reported = false;
        if !self.quiet.types && !self.sees_broken() {
            for problem in ast.type_problems(id) {
                if let TypeProblem::Unresolved(error) = problem {
                    self.errors.push(error.clone());
                    reported = true;
                }
            }
        }
        self.quiet.failed |= !reported;
        TyId::ERROR
    }

    /// Reports the problems the expansion of the file's types found with
    /// the spreads of the record type `id` (see [Type
    /// declarations](#type-declarations)), unless resolving quietly, and
    /// returns whether it has any, which make it erroneous: a spread of a
    /// type that is not a record type, "cannot spread type T: it is not a
    /// record type", at the type spread, which is resolved here, being in
    /// none of the record's fields, so that its own errors are reported
    /// instead if it has any; a spread of a field of a field written before
    /// it, "field `x` is defined twice", as for a value's spread; and a
    /// spread of a record type written in place with a name twice, "field
    /// `x` appears twice in this type", at the later, as anywhere else.
    fn spread_problems(&mut self, id: TypeExprId) -> bool {
        let ast = self.ast;
        let mut any = false;
        for problem in ast.type_problems(id) {
            any = true;
            let reported = self.errors.len();
            match *problem {
                TypeProblem::NotRecord(operand) => {
                    let ty = self.resolve_type(operand, Bareness::Present);
                    if ty != TyId::ERROR && !self.quiet.types {
                        let shown = self.in_message(ty);
                        self.errors.push(Diagnostic::error(
                            format!("cannot spread type {shown}: it is not a record type"),
                            ast.type_expr(operand).span,
                        ));
                    }
                }
                TypeProblem::Replaces {
                    operand,
                    name,
                    ty,
                    first,
                    others,
                } => {
                    let ty = self.resolve_type(ty, Bareness::Present);
                    if !self.quiet.types {
                        let note = spread_first_note(others as usize);
                        let (message, label) = match name {
                            Some(name) => {
                                let text = FieldName(ast.names().get(name)).to_string();
                                (
                                    format!("field `{text}` is defined twice"),
                                    format!(
                                        "`{text}` is a field of this, of type {}",
                                        self.in_label(ty)
                                    ),
                                )
                            }
                            None => (
                                "the rest-field is defined twice".to_owned(),
                                format!("this has a rest-field, of type {}", self.in_label(ty)),
                            ),
                        };
                        self.errors.push(
                            Diagnostic::error(message, ast.type_expr(operand).span)
                                .with_label(label)
                                .with_context(first, "first defined here")
                                .with_note(note),
                        );
                    }
                }
                TypeProblem::Repeated { name, at } => {
                    if !self.quiet.types {
                        self.errors
                            .push(Diagnostic::error(appears_twice(ast.names(), name), at));
                    }
                }
                TypeProblem::Unresolved(_) | TypeProblem::Erroneous => {}
            }
            self.quiet.failed |= self.errors.len() == reported;
        }
        any
    }

    /// The type a type expression denotes, as [`Checker::resolve_type`]
    /// gives it, once it is known to be neither one that names nothing nor
    /// a copy of a declared type's definition to resolve quietly.
    fn resolve_written(&mut self, id: TypeExprId, mode: Bareness) -> TyId {
        let ty = *self.ast.type_expr(id);
        match ty.kind {
            TypeExprKind::Named(name) => {
                if let Some(&(_, builtin)) = self
                    .builtin_types
                    .iter()
                    .find(|(builtin, _)| *builtin == Some(name))
                {
                    return self.bare(builtin, mode);
                }
                let text = self.ast.names().get(name);
                if self.is_type_constructor(name) {
                    let message =
                        format!("type `{text}` needs a type argument, as in `{text}<Int>`");
                    self.errors.push(Diagnostic::error(message, ty.span));
                } else {
                    let message = format!("unknown type `{text}`");
                    let suggestion = self.type_suggestion(name, ty.span, false);
                    self.unresolved_name(
                        name,
                        Diagnostic::error(message, ty.span).with_suggestion(suggestion),
                    );
                }
                TyId::ERROR
            }
            TypeExprKind::List(element) => self.resolve_list(element, mode),
            TypeExprKind::Applied { name, span, arg } => {
                self.resolve_applied((name, span), arg, mode)
            }
            TypeExprKind::Union { bar, .. } => self.resolve_union(id, bar, mode),
            TypeExprKind::Record(range) => self.resolve_record(id, range, mode),
            TypeExprKind::Fn {
                params,
                ret,
                effect,
                meta,
            } => {
                // A function type has its own rule, whatever surrounds it.
                let (params_mode, ret_mode) = bareness_of(effect);
                let base = self.fn_params.len();
                for param in self.ast.param_tys(params) {
                    // Resolving a nested function type uses `fn_params`
                    // above this one's entries and truncates back.
                    let param_ty = self.resolve_type(param.ty, params_mode);
                    self.fn_params.push((param.name, param_ty));
                }
                let ret = self.resolve_type(ret, ret_mode);
                let meta = meta.map(|meta| self.resolve_meta(meta));
                // An erroneous meta makes the type erroneous.
                let ret = if matches!(meta, Some(None)) {
                    TyId::ERROR
                } else {
                    ret
                };
                let resolved = self.intern_fn_type_with_meta(base, ret, effect, meta.flatten());
                self.fn_params.truncate(base);
                resolved
            }
            // A qualified name is made what it names by the expansion of
            // the file's types; checked alone, a file names no module.
            TypeExprKind::Path(path) => {
                let segments = self.ast.path(path);
                let shown = self.ast.show_path(segments).to_string();
                let error = Diagnostic::error(format!("unknown type `{shown}`"), ty.span);
                match segments.first().and_then(PathSegment::name) {
                    Some(first) => self.unresolved_name(first, error),
                    None => self.unresolved(error),
                }
                TyId::ERROR
            }
            TypeExprKind::Ref(_) => {
                self.errors.push(
                    InternalError {
                        message: "a reference to a declared type is in a tree",
                        span: ty.span,
                    }
                    .into(),
                );
                TyId::ERROR
            }
            TypeExprKind::Invalid => self.invalid_type(id),
        }
    }

    /// The type the record type `id`, whose fields are `range`, denotes in
    /// `mode` (see [`Checker::resolve_type`]): the record of its fields'
    /// types and its rest-field's, if it has one, each resolved in `mode`
    /// too, written bare in `mode`; or `Error` if a field's type is
    /// erroneous, a name appears twice, "field `x` appears twice in this
    /// type", reported at each appearance after its first, the rest-field
    /// appears twice, "the rest-field appears twice in this type", at the
    /// later, or a spread of it had a problem ([`Checker::spread_problems`]).
    fn resolve_record(&mut self, id: TypeExprId, range: FieldTyRange, mode: Bareness) -> TyId {
        let fields = self.ast.field_tys(range);
        let base = self.sorted_fields.len();
        let mut erroneous = false;
        let mut defaults: Vec<(Name, Constant)> = Vec::new();
        let mut rest: Option<TyId> = None;
        for (index, field) in fields.iter().enumerate() {
            let name = match field.kind {
                FieldTyKind::Field(name) => name,
                FieldTyKind::Rest => {
                    let rest_ty = self.resolve_type(field.ty, mode);
                    erroneous |= rest_ty == TyId::ERROR;
                    if rest.replace(rest_ty).is_some() {
                        self.errors.push(Diagnostic::error(
                            appears_twice(self.ast.names(), None),
                            field.span,
                        ));
                        erroneous = true;
                    }
                    continue;
                }
                // A spread is merged into the fields by the expansion of the
                // file's types, which a workspace does before checking a file.
                FieldTyKind::Spread => {
                    self.errors.push(
                        InternalError {
                            message: "a spread in a record type was not expanded",
                            span: field.span,
                        }
                        .into(),
                    );
                    erroneous = true;
                    continue;
                }
            };
            // Resolving a nested record type uses `sorted_fields` above this
            // one's entries and truncates back.
            let field_ty = self.resolve_type(field.ty, mode);
            erroneous |= field_ty == TyId::ERROR;
            if let Some(default) = field.default {
                match self.field_default(default, (field_ty, field.ty)) {
                    Some(constant) => defaults.push((name, constant)),
                    None => erroneous = true,
                }
            }
            self.sorted_fields.push((name, position(index), field_ty));
        }
        erroneous |= self.spread_problems(id);
        let sorted = self.sorted_fields.get_mut(base..).unwrap_or_default();
        sorted.sort_unstable_by_key(|&(name, index, _)| (name, index));
        // Every appearance of a name after its first is reported.
        let mut previous = None;
        for &(name, index, _) in &*sorted {
            if previous.replace(name) != Some(name) {
                continue;
            }
            let Some(repeat) = fields.get(index as usize) else {
                continue;
            };
            self.errors.push(Diagnostic::error(
                appears_twice(self.ast.names(), Some(name)),
                repeat.span,
            ));
            erroneous = true;
        }
        let resolved = if erroneous {
            TyId::ERROR
        } else {
            let sorted = self.sorted_fields.get(base..).unwrap_or_default();
            let record = self.types.record_with_rest(
                sorted.iter().map(|&(name, _, ty)| (name, ty)),
                rest,
                defaults,
            );
            self.bare(record, mode)
        };
        self.sorted_fields.truncate(base);
        resolved
    }

    /// The constant that `default`, the default of a field of a record type
    /// whose type is `ty`, written as `written`, is (see [Field
    /// defaults](#field-defaults)), or `None` if it is in error, which makes
    /// the record type erroneous. One the parser made is checked where its
    /// type is resolved, as a parameter's default is (see
    /// [`Checker::check_defaults`]), and as the rest of the type is, so
    /// that a type resolved in silence first, as a top-level function's
    /// signature is ([`Checker::declared_type`]), reports it once, where it
    /// is resolved again: it must be a constant, "a field's
    /// default must be a constant" at it, noted "a literal, or a list or
    /// record of literals", and is otherwise checked against `ty` as an
    /// annotated initialiser is against its annotation, the usual mismatch
    /// at it if it does not fit. One a copy of a declared type's definition
    /// holds, its own file's check having reported what is wrong in it, is
    /// fitted to `ty` ([`Checker::fit_constant`]), with nothing reported:
    /// one that is no constant, or does not fit, fails the check quietly.
    fn field_default(
        &mut self,
        default: FieldDefault,
        (ty, written): (TyId, TypeExprId),
    ) -> Option<Constant> {
        match default {
            FieldDefault::Copied(at) => {
                let fitted = self
                    .ast
                    .copied_default(at)
                    .and_then(|constant| self.fit_constant(constant, ty));
                self.quiet.failed |= fitted.is_none();
                fitted
            }
            FieldDefault::Expr(expr) => self.check_field_default(expr, (ty, written)),
        }
    }

    /// Checks the expression `default`, a field's default that the parser
    /// made, against `ty`, the field's type, written as `written`, as
    /// [`Checker::field_default`] says, and returns the constant it is, or
    /// `None` if it is in error.
    fn check_field_default(
        &mut self,
        default: ExprId,
        (ty, written): (TyId, TypeExprId),
    ) -> Option<Constant> {
        if !self.is_constant(default) {
            self.errors.push(
                Diagnostic::error(FIELD_NOT_CONSTANT, self.ast.get(default).span)
                    .with_note(CONSTANT_NOTE),
            );
            return None;
        }
        if ty == TyId::ERROR {
            return None;
        }
        let errors = self.errors.len();
        let context = self.ast.type_expr(written).span;
        let found =
            self.check_expr_expecting(default, ty, context, "expected because of this annotation");
        if found == TyId::ERROR || self.errors.len() != errors {
            return None;
        }
        self.constant(default)
    }

    /// The type the union written as `id`, whose last `|` is at `bar`,
    /// denotes in `mode` (see the module documentation): its members are
    /// the types the `|`s separate, at any depth of the written tree. If one
    /// of them is written `Future<...>`, each is resolved with every bare
    /// type present, and the union must be the either shape: exactly one
    /// member a future, of the union of the others, which it is the either
    /// shape of; anything else is "unions other than `T | Future<T>` are not
    /// supported yet", at `bar`. Otherwise the union is bare: each member is
    /// resolved in `mode`, the either shape the bare type rule gave a bare
    /// member at its own position taken off it, and the union of the
    /// members is what [`Checker::bare`] makes of it in `mode`. `Error` if a
    /// member is erroneous.
    fn resolve_union(&mut self, id: TypeExprId, bar: Span, mode: Bareness) -> TyId {
        let mut written = Vec::new();
        let mut pending = vec![id];
        while let Some(member) = pending.pop() {
            match self.ast.type_expr(member).kind {
                TypeExprKind::Union { lhs, rhs, .. } => pending.extend([rhs, lhs]),
                _ => written.push(member),
            }
        }
        let explicit = written.iter().any(|&member| self.is_future(member));
        let member_mode = if explicit { Bareness::Present } else { mode };
        let mut members = Vec::with_capacity(written.len());
        let mut erroneous = false;
        for &member in &written {
            let mut ty = self.resolve_type(member, member_mode);
            if member_mode == Bareness::Either && self.is_bare(member) {
                ty = self.types.strip(ty);
            }
            erroneous |= ty == TyId::ERROR;
            members.push(ty);
        }
        if erroneous {
            return TyId::ERROR;
        }
        let (lazy, present): (Vec<TyId>, Vec<TyId>) = members
            .into_iter()
            .partition(|&member| self.types.laziness(member) != Laziness::Present);
        match *lazy.as_slice() {
            [] => {
                let union = self.types.union(present);
                self.bare(union, mode)
            }
            [future] if !present.is_empty() => {
                let union = self.types.union(present);
                if self.types.kind(future) == &TyKind::Future(union) {
                    self.types.deferrable(union)
                } else {
                    self.union_future_error(bar)
                }
            }
            _ => self.union_future_error(bar),
        }
    }

    /// Reports a union with a `Future` member that is not the either shape,
    /// at its last `|`, `bar`, and returns `Error`.
    fn union_future_error(&mut self, bar: Span) -> TyId {
        self.errors.push(Diagnostic::error(
            "a union with a `Future` member must be `T | Future<T>`",
            bar,
        ));
        TyId::ERROR
    }

    /// Whether `ty` is written `Future<...>`.
    fn is_future(&self, ty: TypeExprId) -> bool {
        matches!(
            self.ast.type_expr(ty).kind,
            TypeExprKind::Applied { name, .. } if Some(name) == self.future_name
        )
    }

    /// Whether the union written as `id` has a member written
    /// `Future<...>`, at any depth of the written tree, which makes it say
    /// what it is.
    fn has_future_member(&self, id: TypeExprId) -> bool {
        let mut pending = vec![id];
        while let Some(member) = pending.pop() {
            match self.ast.type_expr(member).kind {
                TypeExprKind::Union { lhs, rhs, .. } => pending.extend([rhs, lhs]),
                _ if self.is_future(member) => return true,
                _ => {}
            }
        }
        false
    }

    /// What the type `ty`, written bare, means in `mode`: `ty` itself in
    /// `Present` mode, and its either shape in `Either` mode.
    fn bare(&mut self, ty: TyId, mode: Bareness) -> TyId {
        match mode {
            Bareness::Present => ty,
            Bareness::Either => self.types.deferrable(ty),
        }
    }

    /// The list type whose element type is written as `element`, written
    /// bare in `mode`, as a record type is: the element type is a position
    /// inside it, resolved in `mode` too. `Error` if the element type is
    /// erroneous.
    fn resolve_list(&mut self, element: TypeExprId, mode: Bareness) -> TyId {
        let element = self.resolve_type(element, mode);
        if element == TyId::ERROR {
            return TyId::ERROR;
        }
        let list = self.types.list(element);
        self.bare(list, mode)
    }

    /// Whether `name` is `Future`, `Present` or `List`, which take a type
    /// argument.
    fn is_type_constructor(&self, name: Name) -> bool {
        Some(name) == self.future_name
            || Some(name) == self.present_name
            || Some(name) == self.list_name
    }

    /// Whether a type written as `ty` is *bare*, so that the bare type rule
    /// applies to it: a name, a record type, a list type, written `List<T>`
    /// or `T[]`, or a union with no member written `Future<...>`. Not
    /// `Present<...>`, `Future<...>`, a union with a `Future` member or a
    /// function type, which say what they are.
    fn is_bare(&self, ty: TypeExprId) -> bool {
        match self.ast.type_expr(ty).kind {
            TypeExprKind::Named(_)
            | TypeExprKind::Path(_)
            | TypeExprKind::Record(_)
            | TypeExprKind::List(_) => true,
            TypeExprKind::Applied { name, .. } => Some(name) == self.list_name,
            TypeExprKind::Union { .. } => !self.has_future_member(ty),
            TypeExprKind::Fn { .. } | TypeExprKind::Ref(_) | TypeExprKind::Invalid => false,
        }
    }

    /// The type the applied type `NAME<ARG>` denotes, the name `name`
    /// written at `span`, in `mode`: `Future<T>` for `Future` (`T` stripped,
    /// as [`TypeTable::future`] does, so `Future<Future<Int>>` is
    /// `Future<Int>`), and `T` for `Present`, where `T` is the argument
    /// resolved with every bare type present; the list type of the argument
    /// for `List`, written bare in `mode` (see [`Checker::resolve_list`]);
    /// or `Error` (reported) for any other name, or an erroneous argument.
    /// The argument is resolved, and its errors reported, whatever the name.
    fn resolve_applied(
        &mut self,
        (name, span): (Name, Span),
        arg: TypeExprId,
        mode: Bareness,
    ) -> TyId {
        if Some(name) == self.list_name {
            return self.resolve_list(arg, mode);
        }
        let arg = self.resolve_type(arg, Bareness::Present);
        let text = self.ast.names().get(name);
        if Some(name) == self.future_name {
            return if arg == TyId::ERROR {
                TyId::ERROR
            } else {
                self.types.future(arg)
            };
        }
        if Some(name) == self.present_name {
            return arg;
        }
        let builtin = self
            .builtin_types
            .iter()
            .any(|(builtin, _)| *builtin == Some(name));
        if builtin {
            let message = format!("type `{text}` takes no type argument");
            self.errors.push(Diagnostic::error(message, span));
        } else {
            let suggestion = self.type_suggestion(name, span, true);
            self.unresolved_name(
                name,
                Diagnostic::error(format!("unknown type `{text}`"), span)
                    .with_suggestion(suggestion),
            );
        }
        TyId::ERROR
    }

    /// The suggestion for the unknown type `name`, written at `span`, with a
    /// type argument if `applied` (see [Suggestions](#suggestions)): the
    /// nearest of the types of the file's view and the built-in types that
    /// take no argument, or, applied, of `List`, `Future` and `Present`.
    /// None where the error is not reported: resolving quietly, or in a
    /// file whose view sees one with syntax errors.
    fn type_suggestion(&self, name: Name, span: Span, applied: bool) -> Option<(Span, String)> {
        if self.quiet.types || self.sees_broken() {
            return None;
        }
        let names = self.ast.names();
        let mut candidates: Vec<(&str, u8)> = Vec::new();
        if applied {
            candidates.extend(["List", "Future", "Present"].map(|text| (text, 1)));
        } else {
            candidates.extend(
                ["Int", "Float", "Bool", "String", "Duration", "Size"].map(|text| (text, 1)),
            );
            if let Some(workspace) = self.workspace {
                candidates.extend(
                    workspace
                        .type_names()
                        .map(|declared| (names.get(declared), 0))
                        .filter(|&(text, _)| crate::lexer::is_identifier(text)),
                );
            }
        }
        let best = suggest::best(names.get(name), candidates)?;
        Some((span, best.to_owned()))
    }

    /// Reports `error`, of a name that names nothing, a variable's or a
    /// type's, unless the input's view sees a file with syntax errors, in
    /// whose source that could not be parsed the name may be declared: the
    /// check then fails quietly, its error reported where the source is
    /// (see [Syntax errors](crate::workspace#syntax-errors)).
    fn unresolved(&mut self, error: Diagnostic) {
        if self.sees_broken() {
            self.quiet.failed = true;
        } else {
            self.errors.push(error);
        }
    }

    /// [`Checker::unresolved`] of the error `error` of the name `name`,
    /// which is no error, the check failing quietly, if the file reads the
    /// name as `Error` (see [`crate::workspace`], "Scan errors").
    fn unresolved_name(&mut self, name: Name, error: Diagnostic) {
        if self
            .workspace
            .is_some_and(|workspace| workspace.poisoned(name))
        {
            self.quiet.failed = true;
        } else {
            self.unresolved(error);
        }
    }

    /// Whether a read of the name `name`, whose binding where it is read is
    /// `found`, is a read of what the file reads as `Error`, silently, the
    /// scan having recovered from an error about it (see
    /// [`crate::workspace`], "Scan errors"): a name of the file's view, or
    /// one that names nothing, not a binding of a function or an inner block
    /// of it. The check then fails quietly.
    fn reads_poisoned(&mut self, name: Name, found: Option<(usize, Binding)>) -> bool {
        let poisoned = self
            .workspace
            .is_some_and(|workspace| workspace.has_poison() && workspace.poisoned(name))
            && found.is_none_or(|(index, _)| index < self.top_end);
        self.quiet.failed |= poisoned;
        poisoned
    }

    /// Whether the input's view sees a file with syntax errors, its own
    /// tree among them: a name that names nothing may be declared in source
    /// its parser could not parse, so it is no error, and the check fails
    /// quietly (see [`Checker::unresolved`]).
    fn sees_broken(&self) -> bool {
        self.quiet.sees_broken || self.ast.syntax() != Syntax::Clean
    }

    /// Interns the function type whose parameters are `fn_params[base..]`
    /// and whose return type is `ret`, of effect `effect`, or returns
    /// `Error` if any of them is erroneous, having been reported.
    fn intern_fn_type(&mut self, base: usize, ret: TyId, effect: Effect) -> TyId {
        self.intern_fn_type_with_meta(base, ret, effect, None)
    }

    /// [`Checker::intern_fn_type`], the function type having the meta
    /// `meta`, if it has one (see [Metas](#metas)).
    fn intern_fn_type_with_meta(
        &mut self,
        base: usize,
        ret: TyId,
        effect: Effect,
        meta: Option<Meta>,
    ) -> TyId {
        let params = self.fn_params.get(base..).unwrap_or_default();
        if ret == TyId::ERROR || params.iter().any(|&(_, ty)| ty == TyId::ERROR) {
            return TyId::ERROR;
        }
        self.types
            .fn_type_with_meta(params.iter().copied(), ret, effect, meta)
    }

    /// The meta the function type's meta written as `meta` denotes (see
    /// [Metas](#metas)): its type resolved as a return type of its marker's
    /// effect is, either-shaped under `META->` and `META->>` and present under
    /// `META:` where it is bare, which must be a record type once every
    /// `Future` is taken off it, "a meta must be a record type: its fields
    /// are read from the function" otherwise, at the type. `None` if it is
    /// erroneous, reported.
    fn resolve_meta(&mut self, meta: MetaTy) -> Option<Meta> {
        let (_, mode) = bareness_of(meta.effect);
        let ty = self.resolve_type(meta.ty, mode);
        if ty == TyId::ERROR {
            return None;
        }
        if !self.types.is_record(self.types.strip(ty)) {
            self.errors.push(Diagnostic::error(
                "a meta must be a record type: its fields are read from the function",
                self.ast.type_expr(meta.ty).span,
            ));
            return None;
        }
        Some(Meta {
            effect: meta.effect,
            ty,
        })
    }

    /// Checks that `expr`, of type `found`, can have type `expected`: an
    /// untyped integer is coerced to `Int` or `Float`, the type of the
    /// `Int` or `Float` expected or of the either shape of one, and any
    /// other type must be a subtype of `expected`, or become one once
    /// awaited, in which case `expr` is marked to be awaited (see
    /// [`TypeTable::needs_await`]), subject to the purity rule. A mismatch
    /// is reported at `expr` with a context label on `context`, the source
    /// of the expectation.
    ///
    /// Where `found` has a provisional type of the innermost function's at a
    /// position where `expected` is present, that parameter is measured
    /// here, and the test is retried with every provisional type of the
    /// function in `found` made present.
    fn expect(&mut self, expr: ExprId, found: TyId, expected: TyId, context: Span, why: &str) {
        let site = (AwaitSite::Implicit, None);
        self.expect_at(expr, found, expected, (context, why), site);
    }

    /// As [`Checker::expect`], with `site` saying how an await at the top is
    /// reported in a pure function (see [`Checker::await_here`]), and
    /// `filling`, for an argument of a known callee of this input, the
    /// callee's function and the parameter the argument fills, for the
    /// label on a mismatch (see [`Checker::narrowing_label`]).
    fn expect_at(
        &mut self,
        expr: ExprId,
        found: TyId,
        expected: TyId,
        (context, why): (Span, &str),
        (site, filling): (AwaitSite, Option<(FnId, usize)>),
    ) {
        match (found, expected) {
            (TyId::ERROR, _) => {}
            // The expected type is unknown and has been reported.
            (_, TyId::ERROR) => {
                self.settle(expr, found);
            }
            (TyId::UNTYPED_INT, TyId::INT | TyId::FLOAT) => self.coerce(expr, expected),
            // A literal is present, so it satisfies the either shape of its
            // type, `Int | Future<Int>`, as it would `Int`, and a provisional
            // one, which stands for its present type as a supertype; and a
            // union with a numeric member, as that member.
            (TyId::UNTYPED_INT, _)
                if self.types.laziness(expected) != Laziness::Future
                    && self.numeric_member(self.types.strip(expected)).is_some() =>
            {
                let present = self.numeric_target(expected);
                self.coerce(expr, present);
            }
            _ if self.types.is_subtype(found, expected) => {}
            _ => {
                let found = match self.claim_against(expr, found, expected) {
                    Some(claimed) if self.types.is_subtype(claimed, expected) => return,
                    Some(claimed) => claimed,
                    None => found,
                };
                match self.types.needs_await(found, expected) {
                    Some(kind) => {
                        // Only an await at the top is the parameter's doing.
                        let site = match (kind, site) {
                            (AwaitKind::Top, site) => site,
                            (AwaitKind::Deep, _) => AwaitSite::Implicit,
                        };
                        self.await_here(expr, found, kind, site);
                    }
                    None => self.mismatch(expr, (found, expected), (context, why), filling),
                }
            }
        }
    }

    /// The functions, as indices into the stack of functions being checked,
    /// whose parameters a use in the innermost body may measure: the
    /// innermost function, and each function enclosing it that is reached
    /// through pure functions only (see the module documentation), a
    /// memoised function counting as the impure function it is. Empty at
    /// the top level.
    fn claimable(&self) -> std::ops::Range<usize> {
        let end = self.functions.len();
        let mut start = end.saturating_sub(1);
        while start > 0
            && self
                .functions
                .get(start)
                .is_some_and(|function| function.effect == Effect::Pure)
        {
            start -= 1;
        }
        start..end
    }

    /// Where `expr`, of type `found`, is expected to have type `expected`,
    /// and does not as it is: measures each parameter of a claimable
    /// function (see [`Checker::claimable`]) whose provisional type sits in
    /// `found` where `expected` is present (see
    /// [`TypeTable::provisionals_at_present`]), and returns `found` with
    /// every provisional type of those functions made present; or `None` if
    /// there is no such parameter.
    fn claim_against(&mut self, expr: ExprId, found: TyId, expected: TyId) -> Option<TyId> {
        self.claim_against_at(self.ast.get(expr).span, found, expected)
    }

    /// As [`Checker::claim_against`], for a value at `span`, which is where
    /// each parameter is measured: a spread's operand, for a soft field,
    /// which is not an expression of its own.
    fn claim_against_at(&mut self, span: Span, found: TyId, expected: TyId) -> Option<TyId> {
        let mut found = found;
        let mut claimed = false;
        for index in self.claimable().rev() {
            let Some(function) = self.functions.get(index).map(|function| function.fn_id) else {
                continue;
            };
            if !self.types.contains_provisional(found, Some(function)) {
                continue;
            }
            let mut params = std::mem::take(&mut self.claimed);
            params.clear();
            self.types
                .provisionals_at_present(found, expected, function, &mut params);
            if !params.is_empty() {
                found = self.make_present(index, span, &params, found);
                claimed = true;
            }
            self.claimed = params;
        }
        claimed.then_some(found)
    }

    /// Where `expr`, of type `ty`, is used as its bits, or must agree with
    /// another type: measures every parameter of a claimable function (see
    /// [`Checker::claimable`]) whose provisional type is in `ty`, at any
    /// depth, and returns `ty` with all of them present. `ty` itself if it
    /// has none.
    fn claim(&mut self, expr: ExprId, ty: TyId) -> TyId {
        let mut ty = ty;
        for index in self.claimable().rev() {
            let Some(function) = self.functions.get(index).map(|function| function.fn_id) else {
                continue;
            };
            if !self.types.contains_provisional(ty, Some(function)) {
                continue;
            }
            let mut params = std::mem::take(&mut self.claimed);
            params.clear();
            self.types.provisional_params(ty, function, &mut params);
            let span = self.ast.get(expr).span;
            ty = self.make_present(index, span, &params, ty);
            self.claimed = params;
        }
        ty
    }

    /// Where the value at `span`, of type `ty`, is used as its bits at the
    /// top, as a record is by an update: if `ty` is a provisional type of a
    /// claimable function (see [`Checker::claimable`]), measures its
    /// parameter and returns its present type; otherwise returns `ty`.
    fn claim_top(&mut self, span: Span, ty: TyId) -> TyId {
        let TyKind::Provisional {
            function, param, ..
        } = *self.types.kind(ty)
        else {
            return ty;
        };
        let index = self
            .claimable()
            .find(|&index| self.functions.get(index).map(|f| f.fn_id) == Some(function));
        match index {
            Some(index) => self.make_present(index, span, &[param], ty),
            None => ty,
        }
    }

    /// Records that each of `params`, parameters of the function at `index`
    /// of the stack of functions being checked, is measured at `span`, and
    /// returns `ty` with every provisional type of that function in it made
    /// present.
    fn make_present(&mut self, index: usize, span: Span, params: &[u32], ty: TyId) -> TyId {
        let Some(function) = self.functions.get_mut(index) else {
            return ty;
        };
        for &param in params {
            if let Some(entry) = function.demanded.get_mut(param as usize) {
                // The first measuring use in source order.
                if entry.is_none_or(|first| span.start < first.start) {
                    *entry = Some(span);
                }
            }
        }
        let (fn_id, count) = (function.fn_id, function.demanded.len());
        self.types.settle(ty, fn_id, &vec![true; count])
    }

    /// Reports that `expr`, of type `found`, does not fit the type
    /// `expected` stated at `context`, labelled `why`; for an argument of a
    /// known callee of this input, `filling` being the callee's function
    /// and the parameter the argument fills, with a label at what kept the
    /// parameter from being narrowed, if that is the cause (see
    /// [`Checker::narrowing_label`]); with a note when a future is expected
    /// and what is found is not one; with the note saying to tell a
    /// union's members apart when what is found is one (see
    /// [`Checker::union_note`]); and with the note saying that a default
    /// fills only a record literal when what is found lacks a defaulted
    /// field (see [`Checker::default_note`]).
    fn mismatch(
        &mut self,
        expr: ExprId,
        (found, expected): (TyId, TyId),
        (context, why): (Span, &str),
        filling: Option<(FnId, usize)>,
    ) {
        let narrowing = filling.and_then(|filling| self.narrowing_label(filling, found, expected));
        let found = self.settle(expr, found);
        let label = format!(
            "expected {}, found {}",
            self.in_label(expected),
            self.in_label(found)
        );
        let mut error = Diagnostic::error("mismatched types", self.ast.get(expr).span)
            .with_label(label)
            .with_context(context, why);
        if let Some((span, label)) = narrowing {
            error = error.with_context(span, label);
        }
        if let Some(note) = self.union_note(expr, found) {
            error = error.with_note(note);
        }
        if let Some(note) = self.default_note(found, expected) {
            error = error.with_note(note);
        }
        if self.types.laziness(expected) == Laziness::Future {
            let shown = self.in_message(expected);
            let found = self.shown(found);
            match self.types.laziness(found) {
                Laziness::Present => {
                    error = error.with_note(format!(
                        "a present value does not satisfy {shown}; write `lazy` before the \
                         expression to defer it"
                    ));
                }
                // A provisional type is shown as what it may be, but none is
                // left once shown.
                Laziness::Either | Laziness::Provisional => {
                    error = error.with_note(format!(
                        "this value may already be present; a {shown} must be deferred"
                    ));
                }
                Laziness::Future => {}
            }
        }
        self.errors.push(error);
    }

    /// The note on a mismatch of a value of type `found` with `expected`
    /// when `found` lacks a defaulted field of a record type of `expected`
    /// (see [Field defaults](#field-defaults)), which no default fills, the
    /// value being no record literal written where the record type is
    /// expected: "a default fills only a record literal written where `T`
    /// is expected", `T` that record type ([`Checker::lacked_default`]).
    /// `None` otherwise.
    fn default_note(&mut self, found: TyId, expected: TyId) -> Option<String> {
        let record = self.lacked_default(found, expected)?;
        let shown = self.in_message(record);
        Some(format!(
            "a default fills only a record literal written where {shown} is expected"
        ))
    }

    /// The record type, of `expected` or of a type in it, that has a
    /// defaulted field that `found` lacks where the two types meet: each
    /// stripped of its future layers, at the top, and then through the
    /// element types of two list types and through the fields that two
    /// record types share, outermost first. `None` if there is none, and
    /// where `expected` is a `Future`.
    ///
    /// Recurses once per level of list and record nesting shared by the two
    /// types.
    fn lacked_default(&self, found: TyId, expected: TyId) -> Option<TyId> {
        if self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let (found, expected) = (self.types.strip(found), self.types.strip(expected));
        if let (Some(found), Some(expected)) = (
            self.types.list_element(found),
            self.types.list_element(expected),
        ) {
            return self.lacked_default(found, expected);
        }
        let (Some(_), Some(fields)) = (
            self.types.record_fields(found),
            self.types.record_fields(expected),
        ) else {
            return None;
        };
        let lacked = self
            .types
            .defaults(expected)
            .iter()
            .any(|&(name, _)| self.types.field(found, name).is_none());
        if lacked {
            return Some(expected);
        }
        fields
            .iter()
            .find_map(|&(name, ty)| self.lacked_default(self.types.field(found, name)?, ty))
    }

    /// The context label on the mismatch of an argument of type `found`
    /// with `expected`, the type of the parameter number `param` of the
    /// function expression `function` of this input, which the argument
    /// fills, if that parameter is a callback parameter that was not
    /// narrowed at some position and the argument would have fitted had
    /// every narrowable position been narrowed, as it is or once awaited:
    /// the label is at what first kept one of the positions the argument
    /// needs narrowed as written, in source order, a position being needed
    /// if the argument's function type does not take what the position is
    /// written as (see the module documentation). `None` otherwise, when
    /// the mismatch has another cause.
    fn narrowing_label(
        &mut self,
        (function, param): (FnId, usize),
        found: TyId,
        expected: TyId,
    ) -> Option<(Span, String)> {
        let unnarrowed = self
            .fn_resolutions
            .get(function.index())?
            .unnarrowed
            .get(param)?
            .clone()?;
        if !self.types.is_subtype(found, unnarrowed.narrowed)
            && self.types.needs_await(found, unnarrowed.narrowed).is_none()
        {
            return None;
        }
        let offered = self.types.fn_sig(self.types.strip(found))?;
        let written = self.types.fn_sig(expected)?;
        let block = unnarrowed
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(at, &block)| {
                let (&(_, written), &(_, accepted)) =
                    (written.params.get(at)?, offered.params.get(at)?);
                block.filter(|_| !self.types.is_subtype(written, accepted))
            })
            .min_by_key(|block| block.span().start)?;
        let decl = self.ast.fn_decl(function);
        let name = self
            .ast
            .names()
            .get(self.ast.params(decl.params).get(param)?.name);
        let label = match block {
            NarrowingBlock::Given(_) => {
                format!("`{name}` is given a value that may be a future here")
            }
            NarrowingBlock::Used(_) => {
                format!("`{name}` is used as a value here, so it may be given a future")
            }
        };
        Some((block.span(), label))
    }

    /// Where `expr`, of type `ty`, is used as its bits at the top, by a
    /// measuring operator, a call of it or a field access into it: if it
    /// is a parameter of the innermost function whose type is provisional,
    /// the parameter is measured, which makes it present, and its present
    /// type returned; if it may be a future, it is marked to be awaited at
    /// the top, subject to the purity rule, and the type it has once
    /// awaited returned; otherwise, or if it is erroneous, `ty` is.
    fn measure(&mut self, expr: ExprId, ty: TyId) -> TyId {
        if ty == TyId::ERROR || self.types.laziness(ty) == Laziness::Present {
            return ty;
        }
        let claimed = self.claim_top(self.ast.get(expr).span, ty);
        if claimed != ty {
            return claimed;
        }
        // A future, an either, or a provisional type of an enclosing
        // function, which may be a future for all this body knows.
        self.await_here(expr, ty, AwaitKind::Top, AwaitSite::Implicit);
        self.types.strip(ty)
    }

    /// Where `expr`, of type `ty`, is used as its bits all the way down, as
    /// an interpolated expression or an operand of an equality, or is
    /// awaited as a statement unit-like once awaited deeply: every
    /// parameter of the innermost function whose provisional type is in it,
    /// at the top or in a field, is measured; then if it, or a field of the
    /// record it is at any depth, may be a future, it is marked to be
    /// awaited (deeply, if the fields need it), subject to the purity rule,
    /// and the type it has once awaited returned; otherwise, or if it is
    /// erroneous, its type with the measured parameters present is.
    fn measure_deep(&mut self, expr: ExprId, ty: TyId) -> TyId {
        if ty == TyId::ERROR {
            return ty;
        }
        let ty = self.claim(expr, ty);
        let present = self.types.present(ty);
        if present == ty {
            return ty;
        }
        let kind = if present == self.types.strip(ty) {
            AwaitKind::Top
        } else {
            AwaitKind::Deep
        };
        self.await_here(expr, ty, kind, AwaitSite::Implicit);
        present
    }

    /// Marks `expr`, of type `ty`, to be awaited as `kind` says, and applies
    /// the purity rule: awaiting is impure, so inside a pure function it is
    /// an error, "cannot await in a pure function" at the `await` keyword
    /// for an explicit await ([`AwaitSite::Explicit`]), and "cannot use a
    /// value that may be a future in a pure function" at `expr` for an
    /// implicit one (see [`Checker::implicit_await_error`]), or its
    /// call-site form for an argument that fills a parameter the callee's
    /// body measures ([`AwaitSite::Argument`]; see
    /// [`Checker::argument_await_error`]). The mark is recorded regardless,
    /// so that the error does not cascade. The body of a `lazy` is impure,
    /// wherever it is written.
    fn await_here(&mut self, expr: ExprId, ty: TyId, kind: AwaitKind, site: AwaitSite) {
        self.awaits[expr.index()] = kind.into();
        let Some(marker) = self.pure_marker() else {
            return;
        };
        let error = match site {
            AwaitSite::Explicit(keyword) => explicit_await_error(keyword, marker),
            AwaitSite::Argument(call_site) => {
                self.argument_await_error(expr, ty, marker, call_site)
            }
            AwaitSite::Implicit => {
                let capture = self.awaitable_capture(expr);
                self.implicit_await_error(self.ast.get(expr).span, ty, marker, capture)
            }
        };
        self.errors.push(error);
    }

    /// The implicit-await error, "cannot use a value that may be a future
    /// in a pure function", at `span`, where a value of type `ty` is used
    /// as its bits in a pure function whose marker is at `marker`: labelled
    /// with the type and the marker. Its note says how to await `capture`
    /// before the function is created, if it is a variable the function
    /// captures that can be awaited there (see
    /// [`Checker::awaitable_capture`]), and otherwise that the function
    /// could be made impure. For a use of a value by an expression and for
    /// a record read by a dotted `let` or `:=` alike.
    fn implicit_await_error(
        &mut self,
        span: Span,
        ty: TyId,
        marker: Span,
        capture: Option<Name>,
    ) -> Diagnostic {
        let note = match capture {
            Some(name) => {
                let text = self.ast.names().get(name);
                format!(
                    "a pure function cannot await, and `{text}` may be a future; await it before \
                     the function is created, in the enclosing impure code: `let {text} = await \
                     {text}`"
                )
            }
            None => "using the value here would await it, which may run effects; a pure \
                     function may hold and pass on a future but not await it. Declare the \
                     function with `->` to make it impure"
                .to_owned(),
        };
        Diagnostic::error(
            "cannot use a value that may be a future in a pure function",
            span,
        )
        .with_label(format!("this is {}", self.in_label(ty)))
        .with_context(marker, PURE_CONTEXT)
        .with_note(note)
    }

    /// The name of the variable that `expr` is, through any parentheses and
    /// as the result of any block, if the innermost function captures it and
    /// it can be awaited before that function is created (see
    /// [`Checker::awaitable_outside`]). `None` for any other expression,
    /// including a capture whose home and every function between are pure,
    /// which can await it no more than the innermost function can.
    fn awaitable_capture(&self, expr: ExprId) -> Option<Name> {
        let mut node = expr;
        let name = loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                // A block is awaited for its result, as a function's body
                // `{ n }` is against the result type.
                ExprKind::Block { items, .. } => {
                    let last = self.ast.items(items).last()?;
                    let (ItemKind::Result(result) | ItemKind::Expr(result)) =
                        self.ast.item(*last).kind
                    else {
                        return None;
                    };
                    node = result;
                }
                ExprKind::Name(name) => break name,
                _ => return None,
            }
        };
        let VarRef::Capture(capture) = self.expr_vars[node.index()] else {
            return None;
        };
        let binding = self
            .functions
            .last()?
            .captures
            .get(capture as usize)?
            .binding;
        self.awaitable_outside(binding).then_some(name)
    }

    /// Whether the binding at `binding` in `bindings` is one the innermost
    /// function captures, lying below its first binding, that can be awaited
    /// before that function is created: one of the top level, which is
    /// impure, or whose home function (the function whose body declares it)
    /// or a function between that one and the innermost is impure, as a
    /// `lazy` and a memoised function are. Not a binding of the innermost
    /// function itself, nor a capture whose home and every function between
    /// are pure. The condition of [`Checker::awaitable_capture`], and of the
    /// same hint for the variable a dotted `let` or `:=` updates.
    fn awaitable_outside(&self, binding: usize) -> bool {
        let Some((_, enclosing)) = self.functions.split_last() else {
            return false;
        };
        // The outermost function the binding is not in; its home is the
        // function before it, or the top level if there is none.
        let Some(outside) = self
            .functions
            .iter()
            .position(|function| function.bindings_start > binding)
        else {
            return false;
        };
        match outside.checked_sub(1) {
            None => true,
            Some(home) => enclosing.get(home..).is_some_and(|between| {
                // A memoised function may await, as an impure one may.
                between
                    .iter()
                    .any(|function| function.effect != Effect::Pure)
            }),
        }
    }

    /// The marker of the innermost function, if it is pure (a memoised
    /// function is not): where the purity rule's errors say the function is
    /// pure.
    fn pure_marker(&self) -> Option<Span> {
        self.functions
            .last()
            .filter(|function| function.effect == Effect::Pure)
            .map(|function| function.marker)
    }

    /// The call-site form of the implicit-await error, in a pure function
    /// whose marker is at `marker`, for the argument `arg`, of type `ty`,
    /// which fills a parameter that is present because the callee's body
    /// measures it, as `site` describes: "argument `x` would be awaited
    /// inside `f`, which is pure", the argument and the callee named by
    /// their source text if each is a plain name or a dotted path ("this
    /// argument" and "the function" otherwise); labelled with the
    /// argument's type, with where the callee's body measures the
    /// parameter if its function is known, and with the pure function's
    /// marker; and a note saying how to await the argument first, with the
    /// call written so if every argument is a name or a dotted path.
    fn argument_await_error(
        &mut self,
        arg: ExprId,
        ty: TyId,
        marker: Span,
        site: CallSite,
    ) -> Diagnostic {
        let inside = self
            .path_text(site.callee)
            .map_or_else(|| "the function".to_owned(), |text| format!("`{text}`"));
        let message = match self.path_text(arg) {
            Some(text) => {
                format!("argument `{text}` would be awaited inside {inside}, which is pure")
            }
            None => format!("this argument would be awaited inside {inside}, which is pure"),
        };
        let label = format!("this is {}", self.in_label(ty));
        let mut error = Diagnostic::error(message, self.ast.get(arg).span).with_label(label);
        if let Some((span, param)) = site.measured {
            let text = self.ast.names().get(param);
            error = error.with_context(span, format!("`{text}` is measured here"));
        }
        let mut note = "a pure function cannot await; await the argument before the call, in an \
                        impure function"
            .to_owned();
        if let Some(call) = self.awaited_call_text(site.call, arg) {
            note = format!("{note}: `{call}`");
        }
        error.with_context(marker, PURE_CONTEXT).with_note(note)
    }

    /// The source text of `expr` if it is a plain name or a dotted path, a
    /// chain of field accesses on a name, as in `server.port`; `None`
    /// otherwise.
    fn path_text(&self, expr: ExprId) -> Option<String> {
        let names = self.ast.names();
        let mut fields = Vec::new();
        let mut node = expr;
        let root = loop {
            match self.ast.get(node).kind {
                ExprKind::Name(name) => break name,
                ExprKind::Field { expr, name } => {
                    fields.push(name);
                    node = expr;
                }
                _ => return None,
            }
        };
        let mut text = names.get(root).to_owned();
        for &field in fields.iter().rev() {
            text.push('.');
            text.push_str(&FieldName(names.get(field)).to_string());
        }
        Some(text)
    }

    /// The call `call` written with `await` before its argument `awaited`,
    /// as in `f(await x)`, if its callee and every argument, named ones by
    /// their values, is a plain name or a dotted path, so that it can be
    /// written from them; `None` otherwise.
    fn awaited_call_text(&self, call: ExprId, awaited: ExprId) -> Option<String> {
        let ExprKind::Call { callee, args } = self.ast.get(call).kind else {
            return None;
        };
        let mut text = self.path_text(callee)?;
        text.push('(');
        for (index, arg) in self.ast.args(args).iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            if let ArgKind::Named { name, .. } = arg.kind {
                text.push_str(self.ast.names().get(name));
                text.push_str(" = ");
            }
            if arg.value == awaited {
                text.push_str("await ");
            }
            text.push_str(&self.path_text(arg.value)?);
        }
        text.push(')');
        Some(text)
    }

    /// Checks `id` where a value of type `expected` is expected, as stated
    /// at `context`, labelled `why`, and returns the type it has.
    ///
    /// If `expected` is a record type, or the either shape of one, and `id`
    /// is, through any parentheses, a record block, the block is checked
    /// against the record type field by field (see the module
    /// documentation) and its type, and that of each parenthesis around it,
    /// is the record of its fields, a subtype of `expected`, or `Error` if a
    /// field is missing. If `expected` is a list type, or the either shape
    /// of one, and `id` is, through any parentheses, a list literal, a block
    /// with a result or an `if` with `else`, it is checked against the list
    /// type as [`Checker::check_list_expected`] says. Otherwise `id` is
    /// checked bottom-up and then must be a subtype of `expected`, as
    /// [`Checker::expect`] says; but if `expected` is a record type with
    /// defaults, or the either shape of one, and `id` is, through any
    /// parentheses, a block with a result or an `if` with `else`, a record
    /// literal it results in is checked against the record type first, for
    /// the defaults alone, as [`Checker::fill_result`] says.
    fn check_expr_expecting(
        &mut self,
        id: ExprId,
        expected: TyId,
        context: Span,
        why: &str,
    ) -> TyId {
        self.check_expr_expecting_at(id, expected, (context, why), (AwaitSite::Implicit, None))
    }

    /// As [`Checker::check_expr_expecting`], with `site` saying how an
    /// await of `id` at the top is reported in a pure function (see
    /// [`Checker::await_here`]), and `filling`, for an argument of a known
    /// callee of this input, the callee's function and the parameter the
    /// argument fills, for the label on a mismatch (see
    /// [`Checker::narrowing_label`]).
    fn check_expr_expecting_at(
        &mut self,
        id: ExprId,
        expected: TyId,
        stated: (Span, &str),
        how: (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        // An erroneous expected type, its error reported elsewhere, types
        // nothing in the value: what it would have typed is no error.
        if expected == TyId::ERROR {
            self.quiet.expecting_error = self.quiet.expecting_error.saturating_add(1);
            let found = self.check_expected(id, expected, stated, how);
            self.quiet.expecting_error = self.quiet.expecting_error.saturating_sub(1);
            return found;
        }
        self.check_expected(id, expected, stated, how)
    }

    /// [`Checker::check_expr_expecting_at`], whatever `expected` is.
    fn check_expected(
        &mut self,
        id: ExprId,
        expected: TyId,
        (context, why): (Span, &str),
        (site, filling): (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        if let Some(found) = self.loop_within(id) {
            return self.check_loop_expecting(id, found, expected, (context, why), (site, filling));
        }
        // A record block is present, so it is checked against the present
        // member of an either shape (or of a provisional type), or the one
        // record member of a union.
        if let Some(record) = self.expected_member(expected, ValueKind::Record)
            && let Some((block, body)) = self.record_block_within(id)
        {
            let span = self.ast.get(block).span;
            // A union of record types fills nothing.
            let itself = record == self.types.strip(expected);
            let expecting = Expecting {
                ty: record,
                context,
                why,
                close: Span::new(span.end.saturating_sub(1), span.end),
                fill: itself.then_some(block),
                reach: Reach::Whole,
            };
            let ty = self.check_block(body, Some(expecting));
            let mut node = id;
            loop {
                self.expr_types[node.index()] = ty;
                match self.ast.get(node).kind {
                    ExprKind::Paren(inner) => node = inner,
                    _ => break,
                }
            }
            return ty;
        }
        // `{}`, the empty record's literal, is given the defaults of a
        // record type with defaults expected itself.
        if let Some(block) = self.empty_block_within(id)
            && let Some(record) = self.defaulted_record(expected)
        {
            let span = self.ast.get(block).span;
            let expecting = Expecting {
                ty: record,
                context,
                why,
                close: Span::new(span.end.saturating_sub(1), span.end),
                fill: Some(block),
                reach: Reach::Whole,
            };
            let ty = self.empty_record(expecting);
            self.set_through_parens(id, ty);
            return ty;
        }
        if self.expects_list(expected)
            && let Some(ty) = self.check_list_expected(id, expected, (context, why))
        {
            return ty;
        }
        // A record type with defaults expected itself reaches a record
        // literal that a block results in, or an `if`'s branch does, to give
        // it the defaults it lacks; what the block or `if` is then must fit
        // as ever.
        let found = match self.defaulted_record(expected) {
            Some(record) if self.reaches_result(id) => self.fill_result(id, record, (context, why)),
            _ => self.check_expr(id),
        };
        self.expect_at(id, found, expected, (context, why), (site, filling));
        found
    }

    /// Whether `id` is, through any parentheses, a block with a result or an
    /// `if` with `else`: what a record type with defaults expected of it
    /// reaches, for a record literal it results in (see
    /// [`Checker::fill_result`]).
    fn reaches_result(&self, id: ExprId) -> bool {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items, .. } => return self.has_result(items),
                ExprKind::If { has_else, .. } => return has_else,
                _ => return false,
            }
        }
    }

    /// Checks `id`, of which the record type with defaults `record` is
    /// expected, as stated at `context`, labelled `why`, for the defaults
    /// alone (see [Field defaults](#field-defaults)), and returns its type,
    /// recorded for `id` and every parenthesis around it. A record literal,
    /// a record block or `{}`, through any parentheses, is checked against
    /// `record` as any literal is where it is expected
    /// ([`Checker::check_expr_expecting`]), which gives it the defaults it
    /// lacks. A block with a result has its result checked so in turn
    /// ([`Expecting::literal_only`]), and a labelled one's `return`s as
    /// ever; an `if` with `else` has each branch checked so, and is their
    /// join, as ever ([`Checker::check_if`]). Anything else is checked as it
    /// is with nothing expected, and nothing else of the block or `if`
    /// changes: what it is must then fit what is expected where it stands,
    /// as ever.
    ///
    /// Recurses once per level of blocks and branches nested in each other.
    fn fill_result(&mut self, id: ExprId, record: TyId, (context, why): (Span, &str)) -> TyId {
        let mut node = id;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let expr = *self.ast.get(node);
        let ty = match expr.kind {
            ExprKind::Block { items, label }
                if self.is_record_body(items)
                    || (label.is_none() && self.ast.items(items).is_empty()) =>
            {
                return if self.lacks_default(items, record) {
                    self.check_expr_expecting(id, record, context, why)
                } else {
                    self.check_expr(id)
                };
            }
            ExprKind::Block { items, label } if self.has_result(items) => {
                let expecting = Expecting::literal_only(record, (context, why), expr.span);
                self.reach_block(node, (items, label), expecting)
            }
            ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                let fills = Branch::Fills(record, (context, why));
                self.check_if(keyword, cond, (then, else_), (true, fills))
            }
            _ => return self.check_expr(id),
        };
        self.set_through_parens(id, ty);
        ty
    }

    /// Checks `id`, a list whose elements the record type with defaults
    /// `record` is expected to be of, as stated at `context`, labelled
    /// `why`, for the defaults alone (see [Field defaults](#field-defaults)),
    /// and returns its type, recorded for `id` and every parenthesis around
    /// it. A list literal, through any parentheses, is checked as it is
    /// with nothing expected but for its elements: each that is no spread
    /// as [`Checker::fill_result`] says, so that a record literal lacking a
    /// defaulted field is given it, and each spread's operand as
    /// [`Checker::check_spread`] says with `record` ([`Checker::check_list`]).
    /// A `for` has the record literal its body results in given the
    /// defaults it lacks, as a spread `for` has ([`Checker::check_for`]). A
    /// block with a result has its result checked so in turn
    /// ([`Expecting::elements_only`]), and a labelled one's `return`s as
    /// ever; an `if` with `else` has each branch checked so, and is their
    /// join, as ever ([`Checker::check_if`]). Anything else is checked as it
    /// is with nothing expected, and nothing else of the list, the loop, the
    /// block or the `if` changes: what it is must then fit what is expected
    /// where it stands, as ever.
    ///
    /// Recurses once per level of blocks, branches, list literals and
    /// spreads nested in each other.
    fn fill_elements(&mut self, id: ExprId, record: TyId, (context, why): (Span, &str)) -> TyId {
        let mut node = id;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let expr = *self.ast.get(node);
        let fill = (record, (context, why));
        let ty = match expr.kind {
            ExprKind::List(elements) => self.check_list(node, elements, Some(fill)),
            ExprKind::For(for_id) => self.check_for(node, for_id, (None, Some(fill))),
            ExprKind::Block { items, label } if self.has_result(items) => {
                let expecting = Expecting::elements_only(record, (context, why), expr.span);
                self.reach_block(node, (items, label), expecting)
            }
            ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                let fills = Branch::FillsElements(record, (context, why));
                self.check_if(keyword, cond, (then, else_), (true, fills))
            }
            _ => return self.check_expr(id),
        };
        self.set_through_parens(id, ty);
        ty
    }

    /// Checks the block `node`, with a result, whose items are `items`,
    /// labelled `label` if it is, with `expecting`, which reaches only part
    /// of its result ([`Expecting::literal_only`],
    /// [`Expecting::elements_only`]), and returns its type: a labelled one
    /// has its `return`s checked as ever, and joined with its result.
    fn reach_block(
        &mut self,
        node: ExprId,
        (items, label): (ItemRange, Option<LabelId>),
        expecting: Expecting<'_>,
    ) -> TyId {
        match label {
            Some(label) => {
                self.enter_block(node, self.ast.label(label), None);
                let ty = self.check_block(items, Some(expecting));
                self.leave_block(node, ty, None)
            }
            None => self.check_block(items, Some(expecting)),
        }
    }

    /// Whether the record literal whose items are `items`, a record block's
    /// or `{}`'s, has no spread and lacks a defaulted field of the record
    /// type `record`, giving it by none of its field items, or `record` has
    /// a rest-field: whether, where `record` reaches it only for its
    /// defaults and its rest-field ([`Checker::fill_result`]), it is given
    /// one, and so checked against `record`. One that gives every defaulted
    /// field, or that has a spread, which may give it one, is checked there
    /// as it is with nothing expected, unless `record` has a rest-field,
    /// which the literal has only once checked against it.
    fn lacks_default(&self, items: ItemRange, record: TyId) -> bool {
        if self.types.record_rest(record).is_some() {
            return true;
        }
        let items = self.ast.items(items);
        let ast = self.ast;
        if items
            .iter()
            .any(|&item| matches!(ast.item(item).kind, ItemKind::Spread(_)))
        {
            return false;
        }
        self.types.defaults(record).iter().any(|&(name, _)| {
            !items.iter().any(|&item| match ast.item(item).kind {
                ItemKind::Field { path, .. } => {
                    ast.path(path).first().and_then(PathSegment::name) == Some(name)
                }
                _ => false,
            })
        })
    }

    /// Whether a value where `expected` is expected is checked against it
    /// as [`Checker::check_list_expected`] says: whether `expected` is a
    /// list type or a union, or the either shape of one (or a provisional
    /// one), but not a future, which no list literal, block or `if` is.
    fn expects_list(&self, expected: TyId) -> bool {
        if self.types.laziness(expected) == Laziness::Future {
            return false;
        }
        let stripped = self.types.strip(expected);
        self.types.list_element(stripped).is_some() || self.types.union_members(stripped).is_some()
    }

    /// The type a record block, for `kind` [`ValueKind::Record`], or a list
    /// literal or a `for`, for [`ValueKind::List`], is checked against
    /// where `expected` is expected: `expected` once stripped, if it is of
    /// that kind, or the one member of that kind of the union it is once
    /// stripped, if it has exactly one; `None` otherwise, and where
    /// `expected` is a future, which no such value is.
    fn expected_member(&self, expected: TyId, kind: ValueKind) -> Option<TyId> {
        if self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let stripped = self.types.strip(expected);
        let of_kind = |ty: TyId| match kind {
            ValueKind::Record => self.types.is_record(ty),
            ValueKind::List => self.types.list_element(ty).is_some(),
            _ => false,
        };
        if of_kind(stripped) {
            return Some(stripped);
        }
        let mut members = self
            .types
            .union_members(stripped)?
            .iter()
            .copied()
            .filter(|&member| of_kind(member));
        match (members.next(), members.next()) {
            (Some(member), None) => Some(member),
            _ => None,
        }
    }

    /// Whether `body`, a block's, has a result: whether it is not a record
    /// block and its last item is a `= EXPR` or an expression statement.
    fn has_result(&self, body: ItemRange) -> bool {
        !self.is_record_body(body)
            && self.ast.items(body).last().is_some_and(|&last| {
                matches!(
                    self.ast.item(last).kind,
                    ItemKind::Result(_) | ItemKind::Expr(_)
                )
            })
    }

    /// Where a value of type `expected`, a list type or a union, or the
    /// either shape of one, is expected, as stated at `context`, labelled
    /// `why`: checks `id` against it and returns the type it has, if `id`
    /// is, through any parentheses, a list literal and `expected` has a
    /// list type to check it against (see [`Checker::expected_member`]), a
    /// block with a result or an `if` with `else`, and `None`, having
    /// checked nothing, otherwise. (A loop is checked against what is
    /// expected of it by [`Checker::check_loop_expecting`].) The type of a
    /// list literal is found as [`Checker::check_list_expecting`] says, and
    /// that of a block or an `if` is what its result, or each branch, checked against
    /// `expected` in turn, is once any await marked on it has run (see
    /// [`Checker::fit`]); it is recorded for `id` and every parenthesis
    /// around it. So the expected type reaches a list literal that is a
    /// function's body's result, a branch's, or either nested in the other,
    /// and each branch of an `if` that a union is expected of, so that
    /// branches of different types make no join, and no warning.
    fn check_list_expected(
        &mut self,
        id: ExprId,
        expected: TyId,
        (context, why): (Span, &str),
    ) -> Option<TyId> {
        let mut node = id;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let expr = *self.ast.get(node);
        let ty = match expr.kind {
            ExprKind::List(elements) => {
                // `[]` is a list of every type, so any of a union's lists.
                let list = self
                    .expected_member(expected, ValueKind::List)
                    .or_else(|| {
                        let members = self.types.union_members(self.types.strip(expected))?;
                        let empty = self.ast.elements(elements).is_empty();
                        members
                            .iter()
                            .copied()
                            .find(|&member| empty && self.types.list_element(member).is_some())
                    })?;
                self.check_list_expecting(elements, list, (context, why))
            }
            ExprKind::Block { items, label } if self.has_result(items) => {
                let expecting = Expecting::new(expected, (context, why), expr.span);
                match label {
                    Some(label) => self.labelled_block(node, label, items, Some(expecting)),
                    None => self.check_block(items, Some(expecting)),
                }
            }
            ExprKind::If {
                cond,
                then,
                else_,
                has_else: true,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                let narrowing = self.if_condition(keyword, cond);
                let expecting = Branch::Fits(expected, (context, why));
                let then_ty = self.check_branch(then, narrowing.then, expecting);
                let else_ty = self.check_branch(else_, narrowing.else_, expecting);
                if let Some(next_slot) = narrowing.next_slot {
                    self.next_slot = next_slot;
                }
                if then_ty == TyId::ERROR || else_ty == TyId::ERROR {
                    TyId::ERROR
                } else if self.expr_types[cond.index()] == self.never {
                    // A condition of type `Never` never picks a branch.
                    self.never
                } else {
                    // What the branches hold has a join that fits, or the
                    // expected type describes both. Each branch fits what
                    // is expected, so a widened join is no warning.
                    let (joined, _) = self.types.join(then_ty, else_ty);
                    if self.types.is_subtype(joined, expected) {
                        joined
                    } else {
                        expected
                    }
                }
            }
            // A loop is checked against whatever is expected of it, by
            // `check_loop_expecting`, before this is asked.
            _ => return None,
        };
        self.set_through_parens(id, ty);
        Some(ty)
    }

    /// Records `ty` as the type of `id` and of every parenthesis inside it,
    /// down to what they hold.
    fn set_through_parens(&mut self, id: ExprId, ty: TyId) {
        let mut node = id;
        loop {
            self.expr_types[node.index()] = ty;
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                _ => break,
            }
        }
    }

    /// The loop that `id` is, through any parentheses, with its details,
    /// or `None` if it is not one.
    fn loop_within(&self, id: ExprId) -> Option<(ExprId, ForId)> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::For(for_id) => return Some((node, for_id)),
                _ => return None,
            }
        }
    }

    /// Checks `id`, which is, through any parentheses, the loop `node`,
    /// whose details are `for_id`, where a value of type `expected` is
    /// expected, as stated at `context`, labelled `why`, with `site` and
    /// `filling` as for [`Checker::check_expr_expecting_at`] (see
    /// [`Checker::check_for`]): every `return` for the loop is checked
    /// against `expected`, and its body against the element type of
    /// `expected`'s list type, if it has one, as a list literal is. A loop
    /// that no `return` leaves, and whose list type is not checked so, must
    /// then fit `expected` as any value must (see [`Checker::expect`]).
    /// Returns its type, recorded for `id` and every parenthesis inside it.
    fn check_loop_expecting(
        &mut self,
        id: ExprId,
        (node, for_id): (ExprId, ForId),
        expected: TyId,
        (context, why): (Span, &str),
        (site, filling): (AwaitSite, Option<(FnId, usize)>),
    ) -> TyId {
        let ty = self.check_for(node, for_id, (Some((expected, (context, why))), None));
        self.set_through_parens(id, ty);
        let listed = self.expected_member(expected, ValueKind::List).is_some();
        if !listed && !self.loop_resolutions[for_id.index()].returned {
            self.expect_at(id, ty, expected, (context, why), (site, filling));
        }
        ty
    }

    /// Checks `id` where a value of type `expected` is expected, as stated
    /// at `context`, labelled `why` (see [`Checker::check_expr_expecting`]),
    /// and returns what it is once any await marked on it has run (see
    /// [`Checker::value_of`]), or `Error` if checking it reported an error,
    /// a mismatch included. A record block checked field by field against a
    /// record type is the record of what its fields are (see
    /// [`Checker::held_record`]).
    fn fit(&mut self, id: ExprId, expected: TyId, (context, why): (Span, &str)) -> TyId {
        let errors = self.errors.len();
        let found =
            self.check_expr_expecting_at(id, expected, (context, why), (AwaitSite::Implicit, None));
        if found == TyId::ERROR || self.errors.len() > errors {
            return TyId::ERROR;
        }
        match self.value_of(id, expected) {
            BodyValue::Typed(ty) => ty,
            BodyValue::Fields(items) => {
                let record = self.types.strip(expected);
                self.held_record(items, record, found)
            }
            BodyValue::Open(_) => found,
        }
    }

    /// What the record block of `items`, checked without an error field by
    /// field against the record type `record`, which gave it the type
    /// `found`, holds once the awaits marked on its fields' values have run:
    /// the record of what each field's value is, found as
    /// [`Checker::value_of`] finds it, a record block in turn as this finds
    /// it, a field that `record` lacks as its type says, and a defaulted
    /// field of `record` that the block lacks, which it was given (see
    /// [Field defaults](#field-defaults)), as `record` has it. A block with a
    /// dotted field or a spread keeps `found`, the record of the expected
    /// fields' types, and of what the soft fields the expected type lacks
    /// hold, a supertype of what it holds.
    ///
    /// Recurses once per level of record blocks nested in each other.
    fn held_record(&mut self, items: ItemRange, record: TyId, found: TyId) -> TyId {
        let ast = self.ast;
        let base = self.sorted_fields.len();
        for &item in ast.items(items) {
            let (path, value) = match ast.item(item).kind {
                ItemKind::Field { path, value, .. } => (path, value),
                ItemKind::Spread(_) => {
                    self.sorted_fields.truncate(base);
                    return found;
                }
                _ => continue,
            };
            let Some(name) = (match ast.path(path) {
                [field] => field.name(),
                _ => None,
            }) else {
                self.sorted_fields.truncate(base);
                return found;
            };
            let ty = match self.types.field(record, name) {
                Some(expected) => match self.value_of(value, expected) {
                    BodyValue::Typed(ty) => ty,
                    BodyValue::Fields(inner) => {
                        let (inner_record, inner_found) =
                            (self.types.strip(expected), self.expr_types[value.index()]);
                        self.held_record(inner, inner_record, inner_found)
                    }
                    BodyValue::Open(_) => self.expr_types[value.index()],
                },
                None => self.expr_types[value.index()],
            };
            self.sorted_fields.push((name, 0, ty));
        }
        // The defaulted fields the block lacks, which it was given (see
        // [Field defaults](#field-defaults)), as the type has them.
        for &(name, _) in self.types.defaults(record) {
            let written = self
                .sorted_fields
                .get(base..)
                .unwrap_or_default()
                .iter()
                .any(|&(field, _, _)| field == name);
            if !written && let Some(ty) = self.types.field(record, name) {
                self.sorted_fields.push((name, 0, ty));
            }
        }
        let fields = self.sorted_fields.get(base..).unwrap_or_default();
        let held = self
            .types
            .record(fields.iter().map(|&(name, _, ty)| (name, ty)));
        self.sorted_fields.truncate(base);
        held
    }

    /// The empty block `{}`, unlabelled, that `id` is, through any
    /// parentheses: the empty record's literal; `None` if it is not one.
    fn empty_block_within(&self, id: ExprId) -> Option<ExprId> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items, label: None } if self.ast.items(items).is_empty() => {
                    return Some(node);
                }
                _ => return None,
            }
        }
    }

    /// The record type that `expected` is once stripped, if it is not a
    /// future and the record type has defaults or a rest-field: what `{}`
    /// is given the defaults of where `expected` is expected (see [Field
    /// defaults](#field-defaults)), and its rest-field, which a record
    /// literal written there has too (see [Records](#records)). `None`
    /// otherwise, a union of record types included, where `{}` is checked
    /// as ever.
    fn defaulted_record(&self, expected: TyId) -> Option<TyId> {
        if self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let record = self.types.strip(expected);
        (self.types.is_record(record)
            && (!self.types.defaults(record).is_empty()
                || self.types.record_rest(record).is_some()))
        .then_some(record)
    }

    /// The type of `{}`, the empty record's literal, checked against
    /// `expecting`'s record type, which has defaults or a rest-field, as a
    /// record block is ([`Checker::report_missing_fields`]): each defaulted
    /// field is given it, and the record of those fields, with the
    /// rest-field, is its type; each other field is missing, reported, and
    /// its type `Error`.
    fn empty_record(&mut self, expecting: Expecting<'_>) -> TyId {
        let base = self.sorted_fields.len();
        let missing = self.report_missing_fields(base, expecting);
        let fields: Vec<(Name, TyId)> = self
            .sorted_fields
            .get(base..)
            .unwrap_or_default()
            .iter()
            .map(|&(name, _, ty)| (name, ty))
            .collect();
        self.sorted_fields.truncate(base);
        if missing {
            TyId::ERROR
        } else {
            let rest = self.types.record_rest(expecting.ty);
            self.types.record_with_rest(fields, rest, Vec::new())
        }
    }

    /// The type of the function body `body`, if it is `{}` and the return
    /// type `ret`, stated at `context`, labelled `why`, is a record type
    /// with defaults: `{}` given them, as any `{}` is where such a type is
    /// expected ([`Checker::empty_record`]). `None` otherwise, the body
    /// being checked as ever.
    fn empty_body(
        &mut self,
        body: ExprId,
        ret: TyId,
        (context, why): (Span, &str),
    ) -> Option<TyId> {
        let block = self.empty_block_within(body)?;
        let record = self.defaulted_record(ret)?;
        let span = self.ast.get(block).span;
        Some(self.empty_record(Expecting {
            ty: record,
            context,
            why,
            close: Span::new(span.end.saturating_sub(1), span.end),
            fill: Some(block),
            reach: Reach::Whole,
        }))
    }

    /// The record block that `id` is, through any parentheses, with its
    /// body, or `None` if it is not one.
    fn record_block_within(&self, id: ExprId) -> Option<(ExprId, ItemRange)> {
        let mut node = id;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items, .. } if self.is_record_body(items) => {
                    return Some((node, items));
                }
                _ => return None,
            }
        }
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

    /// Computes, records and returns the type of `id`, bottom-up.
    fn check_expr(&mut self, id: ExprId) -> TyId {
        let expr = self.ast.get(id);
        let ty = match expr.kind {
            ExprKind::Int(_) => TyId::UNTYPED_INT,
            ExprKind::Float(_) => TyId::FLOAT,
            ExprKind::Duration(_) => TyId::DURATION,
            ExprKind::Size(_) => TyId::SIZE,
            ExprKind::Bool(_) => TyId::BOOL,
            ExprKind::Str(_) => TyId::STRING,
            ExprKind::Interp(parts) => self.check_interpolation(parts),
            ExprKind::If {
                cond,
                then,
                else_,
                has_else,
            } => {
                let keyword = expr.keyword_span().unwrap_or(expr.span);
                self.check_if(keyword, cond, (then, else_), (has_else, Branch::Free))
            }
            ExprKind::Paren(inner) => self.check_expr(inner),
            ExprKind::Unary { op, operand } => {
                let operand_ty = self.check_expr(operand);
                let op_span = expr.operator_span().unwrap_or(expr.span);
                self.unary(op, op_span, (operand, operand_ty))
            }
            ExprKind::Binary { op, lhs, rhs, .. } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                self.binary(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
            }
            ExprKind::Name(name) => self.check_name(id, name, expr.span),
            ExprKind::Block { items, label } => match label {
                Some(label) => self.labelled_block(id, label, items, None),
                None => self.check_block(items, None),
            },
            ExprKind::Field { expr: record, name } => {
                // A reference to a node of the tree of the input's top-level
                // fields below its root reads that node's cell.
                let imports = self.workspace.is_some_and(Context::has_view);
                let reference = if self.mentions.tree.nodes.is_empty() && !imports {
                    None
                } else {
                    self.field_reference(id)
                };
                if let Some(ty) = reference {
                    self.builtin_use(id, ty, expr.span)
                } else if let Some(ty) = self.fileroot_lacks(id, record, name) {
                    ty
                } else {
                    let record_ty = self.check_expr(record);
                    self.field_access(id, (record, record_ty), name)
                }
            }
            ExprKind::Ascribe { expr: inner, ty } => {
                let expected = self.resolve_type(ty, Bareness::Present);
                let context = self.ast.type_expr(ty).span;
                let found = self.check_expr_expecting(inner, expected, context, "ascribed here");
                if found == TyId::ERROR {
                    TyId::ERROR
                } else {
                    expected
                }
            }
            ExprKind::Is { expr: operand, ty } => match self.check_test(id, operand, ty, "is") {
                Some(tested) if tested.unreached => self.never,
                Some(_) => TyId::BOOL,
                None => TyId::ERROR,
            },
            ExprKind::As { expr: operand, ty } => self
                .check_test(id, operand, ty, "as")
                .map_or(TyId::ERROR, |tested| tested.narrowed),
            ExprKind::Fn(function) => self.check_fn(id, function),
            ExprKind::Call { callee, args } => self.check_call(id, callee, args),
            ExprKind::Lazy(function) => {
                self.check_lazy(function, symbol_span("lazy", expr.span.start))
            }
            ExprKind::List(elements) => self.check_list(id, elements, None),
            ExprKind::Index { expr: list, index } => self.check_index(id, list, index),
            ExprKind::For(for_id) => self.check_for(id, for_id, (None, None)),
            ExprKind::DefTypeRef(def) => self.check_def_type_ref(id, def),
            // What the parser could not parse, reported already.
            ExprKind::Error => TyId::ERROR,
            // What the parser kept of what it could not finish, reported
            // already: checked for what it is, silently.
            ExprKind::NoField(record) => {
                self.check_unfinished(&[record]);
                TyId::ERROR
            }
            ExprKind::Unclosed { callee, args } => {
                let mut parts = vec![callee];
                parts.extend(self.ast.args(args).iter().map(|arg| arg.value));
                self.check_unfinished(&parts);
                TyId::ERROR
            }
        };
        self.expr_types[id.index()] = ty;
        ty
    }

    /// Checks `parts`, what the parser kept of an expression it could not
    /// finish (a field access with no name's record, an unclosed call's
    /// callee and arguments), each as any expression is but with no
    /// diagnostic: the expression's error is the parser's, and what each is
    /// alone may be what it may not be alone, a module or a built-in read
    /// whole, a callback whose parameters only the call types (see
    /// [Recording](#recording)).
    fn check_unfinished(&mut self, parts: &[ExprId]) {
        let (errors, warnings) = (self.errors.len(), self.warnings.len());
        for &part in parts {
            self.check_expr(part);
        }
        self.errors.truncate(errors);
        self.warnings.truncate(warnings);
    }

    /// Types the binary operator `op` of expression `id` applied to `lhs`
    /// and `rhs`, of types `lhs_ty` and `rhs_ty`: each operand is awaited
    /// at the top if it may be a future, or deeply for an equality, and the
    /// operator typed on what they are once awaited.
    fn binary(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if op.is_equality() {
            return self.equality(id, op, (lhs, lhs_ty), (rhs, rhs_ty));
        }
        let lhs_ty = self.measure(lhs, lhs_ty);
        let rhs_ty = self.measure(rhs, rhs_ty);
        // The right operand of `and` and `or` may not run, so only the left
        // makes them `Never`.
        let logic = matches!(op, BinaryOp::And | BinaryOp::Or);
        if lhs_ty == self.never || (rhs_ty == self.never && !logic) {
            return self.unreached_operands(lhs, rhs);
        }
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                self.arithmetic(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
            }
            BinaryOp::Div => self.division(id, (lhs, lhs_ty), (rhs, rhs_ty)),
            BinaryOp::FloorDiv | BinaryOp::Rem => {
                self.integer_only(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
            }
            BinaryOp::And | BinaryOp::Or => {
                let expr = self.ast.get(id);
                let op_span = expr.operator_span().unwrap_or(expr.span);
                self.logic_operand(op.verb(), op_span, lhs, lhs_ty);
                self.logic_operand(op.verb(), op_span, rhs, rhs_ty);
                if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
                    TyId::ERROR
                } else {
                    TyId::BOOL
                }
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Same
            | BinaryOp::NotSame
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => self.comparison(id, op, (lhs, lhs_ty), (rhs, rhs_ty)),
            BinaryOp::Range | BinaryOp::RangeInclusive => {
                let fits =
                    [(lhs, lhs_ty), (rhs, rhs_ty)].map(|(bound, ty)| self.range_bound(bound, ty));
                if fits.contains(&false) {
                    TyId::ERROR
                } else {
                    self.types.list(TyId::INT)
                }
            }
        }
    }

    /// Checks `bound`, of type `ty` once measured, a bound of a range, which
    /// must be an `Int`, an untyped integer being one: "a range's bounds
    /// must be Int, found T" otherwise. Returns whether it is one.
    fn range_bound(&mut self, bound: ExprId, ty: TyId) -> bool {
        match ty {
            TyId::UNTYPED_INT => {
                self.coerce(bound, TyId::INT);
                true
            }
            TyId::INT => true,
            TyId::ERROR => false,
            _ => {
                let shown = self.in_message(ty);
                let label = format!("this is {}", self.in_label(ty));
                let mut error = Diagnostic::error(
                    format!("a range's bounds must be Int, found {shown}"),
                    self.ast.get(bound).span,
                )
                .with_label(label);
                if let Some(note) = self.union_note(bound, ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
                false
            }
        }
    }

    /// Types the equality `op` of expression `id`: futures have no equality,
    /// so a type with a `Future` anywhere below its top is an error; and
    /// otherwise each operand is awaited deeply if it needs to be, and the
    /// comparison typed on what they are once awaited.
    fn equality(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        // A future at the top is awaited as for any operator; one in a field
        // is not, since it is part of the value compared.
        let with_future = [lhs_ty, rhs_ty]
            .into_iter()
            .map(|ty| self.types.strip(ty))
            .find(|&ty| ty != TyId::ERROR && self.types.contains_future(ty));
        if let Some(ty) = with_future {
            self.settle(lhs, lhs_ty);
            self.settle(rhs, rhs_ty);
            let expr = self.ast.get(id);
            let span = expr.operator_span().unwrap_or(expr.span);
            let lhs_label = format!("this is {}", self.in_label(self.types.strip(lhs_ty)));
            let rhs_label = format!("this is {}", self.in_label(self.types.strip(rhs_ty)));
            let message = format!(
                "cannot compare {}: it contains a future",
                self.in_message(ty)
            );
            self.errors.push(
                Diagnostic::error(message, span)
                    .with_context(self.ast.get(lhs).span, lhs_label)
                    .with_context(self.ast.get(rhs).span, rhs_label)
                    .with_note("futures have no equality; await the future and compare its value"),
            );
            return TyId::ERROR;
        }
        let lhs_ty = self.measure_deep(lhs, lhs_ty);
        let rhs_ty = self.measure_deep(rhs, rhs_ty);
        if lhs_ty == self.never || rhs_ty == self.never {
            return self.unreached_operands(lhs, rhs);
        }
        self.comparison(id, op, (lhs, lhs_ty), (rhs, rhs_ty))
    }

    /// The type of a binary operator one of whose operands, `lhs` and
    /// `rhs`, which always runs, is of type `Never`: `Never`, the operator
    /// never being applied, so asking nothing of either operand; an untyped
    /// integer among them settles to `Int`.
    fn unreached_operands(&mut self, lhs: ExprId, rhs: ExprId) -> TyId {
        for operand in [lhs, rhs] {
            let ty = self.expr_types[operand.index()];
            self.settle(operand, ty);
        }
        self.never
    }

    /// Types a `lazy` expression, whose body is function `function`:
    /// the body is checked as that of an anonymous, parameterless, impure
    /// function, in a frame of its own, capturing what it uses of the
    /// enclosing scopes as a closure does, and the expression's type is
    /// `Future<T>`, `T` being the body's type with an untyped integer
    /// settled to `Int`, stripped (see [`TypeTable::future`]: `lazy lazy 1`
    /// is `Future<Int>`); or `Error` if the body is erroneous. The `lazy`
    /// keyword is at `keyword`.
    fn check_lazy(&mut self, function: FnId, keyword: Span) -> TyId {
        // A function in the body must not see a name meant for another.
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: Effect::Impure,
            marker: decl.marker,
            keyword,
            demanded: Vec::new(),
            result: None,
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let found = self.check_expr(decl.body);
        let returns = self
            .functions
            .last_mut()
            .map(|function| std::mem::take(&mut function.returns))
            .unwrap_or_default();
        let body_ty = if returns.is_empty() {
            self.settle(decl.body, found)
        } else {
            self.returned_lazy(keyword, (decl.body, found), &returns)
        };
        self.finish_function(0, None);
        if body_ty == TyId::ERROR {
            TyId::ERROR
        } else {
            self.types.future(body_ty)
        }
    }

    /// What the `lazy` whose keyword is at `keyword`, whose operand is
    /// `body`, of type `found`, and which the `return`s `returns` leave,
    /// yields: the join of its operand's type and the values returned, as
    /// an `if`'s branches are joined, untyped integers adapting to each
    /// other and then settling to `Int`. A join that widens is a warning,
    /// "the values this `lazy` returns have different types", at the
    /// keyword, labelled at each. (Nothing reaches a `lazy` from a type
    /// expected of it: the `Future` it is must fit that as a whole.)
    fn returned_lazy(
        &mut self,
        keyword: Span,
        (body, found): (ExprId, TyId),
        returns: &[Site],
    ) -> TyId {
        let mut sites = Vec::with_capacity(returns.len() + 1);
        sites.push(Site {
            value: Some(body),
            at: self.ast.get(self.branch_value(body)).span,
            ty: found,
        });
        sites.extend_from_slice(returns);
        let (joined, widened) = self.join_sites(body, &sites, true);
        if widened {
            let future = self.types.future(joined);
            self.sites_widened(
                (
                    "the values this `lazy` returns have different types",
                    keyword,
                ),
                &sites,
                ("`lazy`", future),
            );
        }
        joined
    }

    /// Pops the innermost function off the stack of functions being
    /// checked, recording what was found for it, with `param_count`
    /// parameters and the callee name `callee`, and restores the enclosing
    /// frame's slot allocation. Returns which of its parameters its body
    /// measures: where it first does, for each.
    fn finish_function(&mut self, param_count: u32, callee: Option<Name>) -> Vec<Option<Span>> {
        let Some(finished) = self.functions.pop() else {
            return Vec::new();
        };
        let slot_names = std::mem::replace(&mut self.slot_names, finished.outer_slot_names);
        self.fn_resolutions[finished.fn_id.index()] = FnResolution {
            frame_size: self.frame_size,
            param_count,
            captures: finished
                .captures
                .iter()
                .map(|capture| (capture.source, capture.ty))
                .collect(),
            callee,
            slot_names,
            param_demands: finished.demanded.clone(),
            // Recorded by `check_fn`, for a function that has any.
            unnarrowed: Vec::new(),
            defaults: Vec::new(),
        };
        // A top-level function's copies decide where its closure is made.
        if self.functions.is_empty()
            && let Some(copies) = self
                .checking_top
                .and_then(|top| self.top_copies.get_mut(top))
        {
            *copies = finished
                .captures
                .iter()
                .map(|capture| capture.binding)
                .collect();
        }
        (self.next_slot, self.frame_size) = (finished.outer_next_slot, finished.outer_frame_size);
        finished.demanded
    }

    /// Types the function expression `id`, whose details are `function`,
    /// and checks its body in a frame of its own (see the module
    /// documentation). Its type is known, and recorded, before the body is
    /// checked: the function type of its declared parameter and return
    /// types, or `Error` if any of them is. A bare type in them is present
    /// for a pure function, the either shape for an impure one, and for a
    /// memoised one present in the parameters and the either shape in the
    /// result (see `bareness_of`), except that a pure function's parameter
    /// whose type is written as a name or a record type has a provisional
    /// type while the body is checked, and is present if the body measures
    /// it and the either shape otherwise: once the body is checked, the type
    /// of every expression in the function is settled accordingly, and so
    /// is the function's type. An impure or memoised function whose body
    /// was checked without an error then has its result inferred from what
    /// the body returns (see [`Checker::inferred`]) and its callback
    /// parameters narrowed by what the body gives them (see
    /// [`Checker::narrow_callbacks`]), and its type, recorded again, is the
    /// function type of those parameters and that result. Its parameters'
    /// defaults are checked before its body, in its frame (see
    /// [`Checker::check_defaults`]).
    fn check_fn(&mut self, id: ExprId, function: FnId) -> TyId {
        if let Some(&def) = self.definitions.get(&function) {
            return self.check_definition(id, function, def);
        }
        if let Some(&def) = self.defmetas.get(&function) {
            return self.check_defmeta(id, function, def);
        }
        let callee = self.callee_name.take();
        let decl = *self.ast.fn_decl(function);
        let pure = decl.effect == Effect::Pure;
        let (params_mode, ret_mode) = bareness_of(decl.effect);
        let params = self.ast.params(decl.params);
        let base = self.fn_params.len();
        let mut provisional = false;
        // Each default and the type it is checked against: its parameter's,
        // before any inference.
        let mut defaults = Vec::new();
        // Each parameter's type before any inference, which its default
        // functions' copies of it are given.
        let mut resolved = Vec::with_capacity(params.len());
        for (index, param) in params.iter().enumerate() {
            let mut ty = self.resolve_type(param.ty, params_mode);
            resolved.push(ty);
            if let Some(default) = param.default {
                defaults.push((index, default, (ty, param.ty)));
            }
            if pure && ty != TyId::ERROR && self.is_inferred(param.ty) {
                ty = self.types.provisional(function, position(index), ty);
                provisional = true;
            }
            self.fn_params.push((Some(param.name), ty));
        }
        let ret = if let Some(ret) = decl.ret {
            self.resolve_type(ret, ret_mode)
        } else {
            // The parser gives every function expression a return type.
            self.errors.push(
                InternalError {
                    message: "a function expression has no return type",
                    span: self.ast.get(id).span,
                }
                .into(),
            );
            TyId::ERROR
        };
        let mut ty = self.intern_fn_type(base, ret, decl.effect);
        self.expr_types[id.index()] = ty;
        self.check_def_type_signature(function, ty);
        let ret_span = decl.ret.map_or(self.ast.get(decl.body).span, |ret| {
            self.ast.type_expr(ret).span
        });
        let (default_fns, measured) = self.check_default_fns((params, &resolved), &mut defaults);
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: decl.effect,
            marker: decl.marker,
            keyword: symbol_span("fn", self.ast.get(id).span.start),
            // What its defaults measure, which come before its body.
            demanded: measured,
            result: Some((ret, ret_span)),
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let defaults = self.check_defaults(&defaults, params.len());
        let span = self.ast.get(id).span;
        let errors = self.errors.len();
        let callbacks = (self.callbacks.len(), self.callback_positions.len());
        self.check_fn_body(&decl, base, (callee, ty, span), ret);
        let returns = self
            .functions
            .last_mut()
            .map(|function| std::mem::take(&mut function.returns))
            .unwrap_or_default();
        // An impure or memoised function's bare result takes its laziness
        // from the body and its `return`s, and its callback parameters are
        // narrowed, unless something is wrong with the function.
        let mut unnarrowed = Vec::new();
        if !pure && ty != TyId::ERROR && self.errors.len() == errors {
            let mut result = ret;
            if let Some(ret_expr) = decl.ret {
                result = self.inferred_result((ret_expr, ret), decl.body, &returns);
            }
            let narrowed = self.narrow_callbacks(base, callbacks.0, &mut unnarrowed);
            if result != ret || narrowed {
                ty = self.intern_fn_type(base, result, decl.effect);
                self.expr_types[id.index()] = ty;
            }
        }
        self.callbacks.truncate(callbacks.0);
        self.callback_positions.truncate(callbacks.1);
        self.fn_params.truncate(base);
        let demanded = self.finish_function(decl.params.len(), callee);
        self.add_default_copies();
        if let Some(resolution) = self.fn_resolutions.get_mut(function.index()) {
            resolution.unnarrowed = unnarrowed;
            resolution.defaults =
                merged_defaults(defaults, &default_fns, resolution.captures.len());
        }
        if !provisional {
            return ty;
        }
        let decisions: Vec<bool> = demanded.iter().map(Option::is_some).collect();
        self.settle_function(id, function, &decisions);
        self.expr_types[id.index()]
    }

    /// Checks each default of `defaults`, the defaults of a function whose
    /// parameters are `params`, of the types `resolved` before any
    /// inference, each with its parameter's number, that is no
    /// constant, as its default function (`Checker::check_default_fn`), in
    /// the scope the function is written in, before the function's frame is
    /// entered (see [Calls](#calls)), and takes it out of `defaults`, which
    /// keeps the constants. Returns each parameter with a default function,
    /// in order, with whether it checked without an error, and, for each
    /// parameter, where a default first measures it, if one does.
    fn check_default_fns(
        &mut self,
        (params, resolved): (&[Param], &[TyId]),
        defaults: &mut Vec<(usize, ExprId, (TyId, TypeExprId))>,
    ) -> (Vec<(usize, bool)>, Vec<Option<Span>>) {
        let mut default_fns = Vec::new();
        let mut measured = vec![None; params.len()];
        for &(index, _, expected) in defaults.iter() {
            let Some(default_fn) = params.get(index).and_then(|param| param.default_fn) else {
                continue;
            };
            // A default in a default sees neither's.
            let outer = self.defaults_checked.unseen.len();
            let unseen = params.get(index..).unwrap_or_default();
            let unseen = unseen.iter().map(|param| (param.name, param.span));
            self.defaults_checked.unseen.extend(unseen);
            let earlier = resolved.get(..index).unwrap_or_default();
            let (checked, demanded) = self.check_default_fn(default_fn, (earlier, expected));
            self.defaults_checked.unseen.truncate(outer);
            default_fns.push((index, checked));
            for (first, found) in measured.iter_mut().zip(demanded) {
                *first = first.or(found);
            }
        }
        defaults.retain(|&(index, _, _)| {
            params
                .get(index)
                .is_some_and(|param| param.default_fn.is_none())
        });
        (default_fns, measured)
    }

    /// Once a function's body is checked, if it is the top-level function
    /// whose body is being checked, adds the copies of its defaults to its
    /// own (see [Calls](#calls)).
    fn add_default_copies(&mut self) {
        let copies = &mut self.defaults_checked.copies;
        if copies.is_empty() || !self.functions.is_empty() {
            return;
        }
        if let Some(own) = self
            .checking_top
            .and_then(|top| self.top_copies.get_mut(top))
        {
            own.append(copies);
            own.sort_unstable();
            own.dedup();
        }
        copies.clear();
    }

    /// Checks the default function `function` of a parameter whose type, as
    /// resolved for its function before any inference and as written, is
    /// `expected` (see [Calls](#calls)): in a frame of its own, as a function
    /// written where its parameter's function is, whose parameters are the
    /// ones before the parameter, declared with the types `earlier` their
    /// function resolved them to before any inference, not resolved again,
    /// so that their errors are reported once and a memoised function's
    /// default sees them as its body does, and whose body, the default, is checked against
    /// `expected` as an annotated initialiser is against its annotation. Its
    /// effect is its function's, so a pure function's default is checked as
    /// pure code, its bare parameters inferred as any pure function's are
    /// (see [Inferred parameters](#inferred-parameters)) and settled when it
    /// ends. Returns whether it checked without an error, and, for each of
    /// its parameters, where it first measures it: for a pure function, what
    /// its default measures it measures, since the default runs on its
    /// arguments before its body does.
    fn check_default_fn(
        &mut self,
        function: FnId,
        (earlier, (expected, written)): (&[TyId], (TyId, TypeExprId)),
    ) -> (bool, Vec<Option<Span>>) {
        // A function in the body must not see a name meant for another.
        self.callee_name = None;
        let decl = *self.ast.fn_decl(function);
        let pure = decl.effect == Effect::Pure;
        let params = self.ast.params(decl.params);
        let base = self.fn_params.len();
        let mut provisional = false;
        for (index, param) in params.iter().enumerate() {
            let mut ty = earlier.get(index).copied().unwrap_or(TyId::ERROR);
            if pure && ty != TyId::ERROR && self.is_inferred(param.ty) {
                ty = self.types.provisional(function, position(index), ty);
                provisional = true;
            }
            self.fn_params.push((Some(param.name), ty));
        }
        let context = self.ast.type_expr(written).span;
        self.functions.push(Function {
            fn_id: function,
            bindings_start: self.bindings.len(),
            outer_next_slot: self.next_slot,
            outer_frame_size: self.frame_size,
            outer_slot_names: std::mem::take(&mut self.slot_names),
            captures: Vec::new(),
            effect: decl.effect,
            marker: decl.marker,
            keyword: decl.marker,
            demanded: vec![None; params.len()],
            result: Some((expected, context)),
            returns: Vec::new(),
        });
        (self.next_slot, self.frame_size) = (0, 0);
        let outer = (self.block_start, self.fields_start);
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        for (index, param) in params.iter().enumerate() {
            let ty = self
                .fn_params
                .get(base + index)
                .map_or(TyId::ERROR, |&(_, ty)| ty);
            self.declare_binding(param.name, ty, Some(param.span), BindingKind::Variable);
        }
        let errors = self.errors.len();
        let found = if expected == TyId::ERROR {
            self.check_expr(decl.body);
            TyId::ERROR
        } else {
            self.check_expr_expecting(
                decl.body,
                expected,
                context,
                "expected because of this annotation",
            )
        };
        self.bindings.truncate(self.block_start);
        (self.block_start, self.fields_start) = outer;
        self.fn_params.truncate(base);
        let top = self.functions.len() == 1;
        let demanded = self.finish_function(decl.params.len(), None);
        if provisional {
            let decisions: Vec<bool> = demanded.iter().map(Option::is_some).collect();
            self.settle_function(decl.body, function, &decisions);
        }
        // A top-level function's default function's copies are the
        // function's own, which `finish_function` took for its alone.
        if top
            && let Some(copies) = self
                .checking_top
                .and_then(|top| self.top_copies.get_mut(top))
        {
            self.defaults_checked.copies.append(copies);
        }
        (
            found != TyId::ERROR && self.errors.len() == errors,
            demanded,
        )
    }

    /// Checks the defaults of a function of `count` parameters, `defaults`,
    /// each with its parameter's number and the type it is checked against,
    /// its parameter's as resolved before any inference, and as written
    /// (see [Calls](#calls)), each a constant, in the function's own frame
    /// before its parameters are declared, so that the slots a record
    /// default's check takes are the function's and are free again after
    /// it: each is checked against the type as an annotated initialiser is
    /// against its annotation. Returns the constant each parameter's
    /// default is, for [`FnResolution::defaults`]: empty if none has one,
    /// and `None` for a parameter without one or whose default is in error.
    fn check_defaults(
        &mut self,
        defaults: &[(usize, ExprId, (TyId, TypeExprId))],
        count: usize,
    ) -> Vec<Option<Constant>> {
        if defaults.is_empty() {
            return Vec::new();
        }
        let mut constants = vec![None; count];
        for &(param, default, (ty, written)) in defaults {
            if ty == TyId::ERROR {
                continue;
            }
            let errors = self.errors.len();
            let context = self.ast.type_expr(written).span;
            let found = self.check_expr_expecting(
                default,
                ty,
                context,
                "expected because of this annotation",
            );
            if found == TyId::ERROR || self.errors.len() != errors {
                continue;
            }
            if let Some(entry) = constants.get_mut(param) {
                *entry = self.constant(default);
            }
        }
        constants
    }

    /// Checks what the file declares before its body is checked: its `type`
    /// items, quietly in a workspace, which has checked them first
    /// ([`check_types`]), and the uses of attributes on its functions.
    fn check_declarations(&mut self) {
        let quiet = std::mem::replace(&mut self.quiet.types, self.workspace.is_some());
        self.type_items();
        self.quiet.types = quiet;
        self.check_attribute_uses();
    }

    /// Whether `ty` holds only constants, as an attribute's arguments must
    /// (see [Attributes](#attributes)): an `Int`, `Float`, `Bool`, `String`,
    /// `Duration` or `Size`, a list of such, or a record of such fields with
    /// no rest-field; at the `top`, only such a record.
    ///
    /// Recurses once per level of list and record nesting in the type.
    fn holds_constants(&self, ty: TyId, top: bool) -> bool {
        match self.types.kind(ty) {
            TyKind::Record(record) => {
                record.rest().is_none()
                    && record
                        .fields
                        .iter()
                        .all(|&(_, field)| self.holds_constants(field, false))
            }
            _ if top => false,
            TyKind::Int | TyKind::Float | TyKind::Bool | TyKind::String | TyKind::Quantity(_) => {
                true
            }
            &TyKind::List(element) => self.holds_constants(element, false),
            _ => false,
        }
    }

    /// Checks every use of an attribute that is none of the parser's, on the
    /// function items they apply to (see [Attributes](#attributes)), and
    /// records each function's, in source order, as
    /// [`Resolution::attributes`] gives them. Its arguments' constants take
    /// no slot of the frame: what their checks allocate is given back.
    fn check_attribute_uses(&mut self) {
        let ast = self.ast;
        let uses = ast.all_attribute_uses();
        if uses.is_empty() {
            return;
        }
        let outer = (self.next_slot, self.frame_size, self.slot_names.len());
        let mut found: HashMap<FnId, Vec<Attribute>> = HashMap::new();
        for used in uses {
            let attribute = self.check_attribute_use(used);
            if let (Some(attribute), Some(function)) = (attribute, item_function(ast, used.item)) {
                found.entry(function).or_default().push(attribute);
            }
        }
        (self.next_slot, self.frame_size) = (outer.0, outer.1);
        self.slot_names.truncate(outer.2);
        self.lowering.fn_attributes = found
            .into_iter()
            .map(|(function, attributes)| (function, attributes.into_boxed_slice()))
            .collect();
    }

    /// The attribute `used` names, with its arguments, once checked (see
    /// [Attributes](#attributes)): `retry` and `timeout` the checker's own,
    /// any other name a type of the file's view marked `#[attribute]`,
    /// whose record type its arguments are checked against. `None`, the
    /// error reported, if it names none, or its arguments do not fit.
    fn check_attribute_use(&mut self, used: &AttributeUse) -> Option<Attribute> {
        let names = self.ast.names();
        let text = names.get(used.name);
        let (id, fields) = match text {
            RETRY => (
                AttributeId::Builtin(RETRY),
                vec![(Box::from("times"), TyId::INT, None)],
            ),
            TIMEOUT => (
                AttributeId::Builtin(TIMEOUT),
                vec![(Box::from("limit"), TyId::DURATION, None)],
            ),
            _ => self.declared_attribute(used)?,
        };
        let given = self.attribute_args(used, &fields)?;
        let mut args = Vec::with_capacity(fields.len());
        let mut failed = false;
        for ((field, ty, default), value) in fields.into_iter().zip(given) {
            let constant = match value {
                Some(value) => self.attribute_arg(value, ty, used.name_span),
                None if default.is_some() => default,
                None => {
                    let message = format!("`#[{text}]` needs `{field}`");
                    self.errors.push(Diagnostic::error(message, used.span));
                    None
                }
            };
            match constant {
                Some(constant) => args.push((field, constant)),
                None => failed = true,
            }
        }
        (!failed).then(|| Attribute {
            id,
            args: args.into_boxed_slice(),
        })
    }

    /// The attribute of the file's view of types that `used` names, and its
    /// record type's fields, by name, in its order, each with its type and
    /// default (see [Attributes](#attributes)). `None` for a name that
    /// names none: "unknown attribute `x`", with a suggestion among the
    /// attributes in view and the checker's, or "`T` is not an attribute"
    /// for a type that is not marked `#[attribute]`; and, quietly, for one
    /// whose type is erroneous, which its declaration reports.
    fn declared_attribute(
        &mut self,
        used: &AttributeUse,
    ) -> Option<(AttributeId, AttributeFields)> {
        let names = self.ast.names();
        let text = names.get(used.name);
        let lookup = self
            .workspace
            .map_or(AttributeLookup::Nothing, |workspace| {
                workspace.attribute(used.name)
            });
        let id = match lookup {
            AttributeLookup::Attribute(id) => id.clone(),
            AttributeLookup::NotAttribute => {
                self.errors.push(
                    Diagnostic::error(format!("`{text}` is not an attribute"), used.name_span)
                        .with_note("an attribute is a `type` marked `#[attribute]`"),
                );
                return None;
            }
            AttributeLookup::Nothing => {
                let declared = self
                    .workspace
                    .map(Context::attribute_names)
                    .unwrap_or_default();
                let candidates = declared
                    .iter()
                    .map(|&name| (names.get(name), 0))
                    .chain([(RETRY, 1), (TIMEOUT, 1)]);
                let suggestion =
                    suggest::best(text, candidates).map(|best| (used.name_span, best.to_owned()));
                self.errors.push(
                    Diagnostic::error(format!("unknown attribute `{text}`"), used.name_span)
                        .with_suggestion(suggestion),
                );
                return None;
            }
        };
        let ty = self.resolve_type(used.ty, Bareness::Present);
        let TyKind::Record(record) = self.types.kind(ty) else {
            return None;
        };
        let fields = record
            .fields
            .iter()
            .map(|&(name, field)| {
                let default = record
                    .defaults()
                    .iter()
                    .find(|&&(defaulted, _)| defaulted == name)
                    .map(|(_, constant)| constant.clone());
                (Box::from(names.get(name)), field, default)
            })
            .collect();
        Some((id, fields))
    }

    /// The argument `used` gives each of `fields`, the fields of its
    /// attribute's record, by name, in their order: one positional argument
    /// for an attribute of one field, and named arguments for any. `None`,
    /// the error reported, for a positional argument of an attribute of
    /// another number of fields, a second positional one, a named one that
    /// names no field or one named twice, or a trailing block.
    fn attribute_args(
        &mut self,
        used: &AttributeUse,
        fields: &AttributeFields,
    ) -> Option<Vec<Option<ExprId>>> {
        let ast = self.ast;
        let text = ast.names().get(used.name);
        let mut given: Vec<Option<ExprId>> = vec![None; fields.len()];
        let mut failed = false;
        for arg in used.args.map_or(&[][..], |args| ast.args(args)) {
            let error = match arg.kind {
                ArgKind::Positional if fields.len() != 1 => Some(format!(
                    "`#[{text}]` takes its arguments by name, as in `#[{text}({} = ...)]`",
                    fields.first().map_or("x", |(name, _, _)| name)
                )),
                ArgKind::Positional => match given.first_mut() {
                    Some(slot @ None) => {
                        *slot = Some(arg.value);
                        None
                    }
                    _ => Some(format!("`#[{text}]` takes one argument")),
                },
                ArgKind::Named { name, .. } => {
                    let field_name = ast.names().get(name);
                    match fields
                        .iter()
                        .position(|(field, _, _)| &**field == field_name)
                    {
                        None => Some(format!("`#[{text}]` has no argument `{field_name}`")),
                        Some(at) => match given.get_mut(at) {
                            Some(slot @ None) => {
                                *slot = Some(arg.value);
                                None
                            }
                            _ => Some(format!("`{field_name}` is given twice")),
                        },
                    }
                }
                ArgKind::Trailing => Some(format!("`#[{text}]` takes no block")),
            };
            if let Some(message) = error {
                self.errors.push(Diagnostic::error(message, arg.span(ast)));
                failed = true;
            }
        }
        (!failed).then_some(given)
    }

    /// The constant that `value`, an argument of an attribute named at
    /// `name`, is, checked against its field's type `ty` as a parameter's
    /// constant default is (see [Attributes](#attributes)): "an attribute's
    /// argument must be a constant", noted with what a constant is, for one
    /// that is not. `None`, the error reported, if it does not fit.
    fn attribute_arg(&mut self, value: ExprId, ty: TyId, name: Span) -> Option<Constant> {
        if !self.is_constant(value) {
            self.errors.push(
                Diagnostic::error(
                    "an attribute's argument must be a constant",
                    self.ast.get(value).span,
                )
                .with_note(CONSTANT_NOTE),
            );
            return None;
        }
        let errors = self.errors.len();
        let found = self.check_expr_expecting(value, ty, name, "expected by this attribute");
        if found == TyId::ERROR || self.errors.len() != errors {
            return None;
        }
        self.constant(value)
    }

    /// Whether the expression `id`, a parameter's or a field's default, is
    /// a constant (see [Calls](#calls)): [`is_constant`].
    fn is_constant(&self, id: ExprId) -> bool {
        crate::ast::is_constant(self.ast, id)
    }

    /// The constant that `id`, a parameter's or a field's default that
    /// [`Checker::is_constant`] accepts, is once checked without an error
    /// ([`constant_of`]): an integer literal that is a `Float` there a
    /// `Float`, and a record block given fields' defaults with those fields
    /// (see [Field defaults](#field-defaults)). `None` if it is not one
    /// after all.
    fn constant(&self, id: ExprId) -> Option<Constant> {
        constant_of(
            self.ast,
            id,
            (&self.expr_types, &self.lowering.record_fills),
        )
    }

    /// The constant that `constant`, a field's default of a declared type as
    /// a copy of its definition holds it, written with no type known (see
    /// [`written_constant`]), is where the field is of type
    /// `expected`, as [`Checker::check_expr_expecting`] would check the
    /// default it is written as (see [Field defaults](#field-defaults)):
    /// `None` if it does not fit. An `Int` is the `Int` or `Float` that an
    /// integer literal adapts to; a `Float`, `Bool` or `String` must be a
    /// subtype; a list is fitted against the list type expected, or the one
    /// list member of the union expected, or, for one with several, the
    /// first of them it fits, as a list literal checked bottom up would
    /// be; a record likewise against a record type, a field the type lacks
    /// kept as it is, and, where the record type is expected itself and
    /// not a union of it, each defaulted field it lacks given its default,
    /// as a record block is; and `{}` is given every default of a record
    /// type with defaults expected itself, as a record block is, and must
    /// be a subtype otherwise. Nothing fits a `Future`.
    ///
    /// Recurses once per level of list and record nesting.
    fn fit_constant(&self, constant: &Constant, expected: TyId) -> Option<Constant> {
        if expected == TyId::ERROR || self.types.laziness(expected) == Laziness::Future {
            return None;
        }
        let stripped = self.types.strip(expected);
        // The members a list or record is tried against, in turn.
        let members: &[TyId] = match self.types.union_members(stripped) {
            Some(members) => members,
            None => std::slice::from_ref(&stripped),
        };
        match constant {
            Constant::Int(value) => match self.numeric_member(stripped)? {
                TyId::FLOAT => Some(Constant::Float(int_to_float(*value).to_bits())),
                _ => Some(Constant::Int(*value)),
            },
            Constant::Float(_) => self
                .types
                .is_subtype(TyId::FLOAT, expected)
                .then(|| constant.clone()),
            Constant::Bool(_) => self
                .types
                .is_subtype(TyId::BOOL, expected)
                .then(|| constant.clone()),
            &Constant::Quantity(quantity, _) => self
                .types
                .is_subtype(TyId::of_quantity(quantity), expected)
                .then(|| constant.clone()),
            Constant::Str(_) => self
                .types
                .is_subtype(TyId::STRING, expected)
                .then(|| constant.clone()),
            // `{}` is given the defaults of a record type with defaults
            // expected itself, as a record block is, and is checked as ever
            // otherwise.
            Constant::Empty => match self.defaulted_record(expected) {
                Some(record) => self.fit_record(&[], record, true),
                None => self
                    .types
                    .is_subtype(TyId::EMPTY, expected)
                    .then_some(Constant::Empty),
            },
            Constant::List(elements) => members.iter().find_map(|&member| {
                let element = self.types.list_element(member)?;
                elements
                    .iter()
                    .map(|value| self.fit_constant(value, element))
                    .collect::<Option<_>>()
                    .map(Constant::List)
            }),
            Constant::Record(fields) => {
                // Only a record type expected itself fills.
                let fills = members.len() == 1;
                members
                    .iter()
                    .find_map(|&member| self.fit_record(fields, member, fills))
            }
        }
    }

    /// The record constant of `fields` fitted to the record type `record`,
    /// as [`Checker::fit_constant`] says, a field `record` does not name
    /// fitted to its rest-field if it has one, each defaulted field it lacks
    /// given its default if `fills`, after the fields written, in the order
    /// of their names; `None` if `record` is no record type, a field does
    /// not fit, or one it lacks has no default.
    ///
    /// Recurses once per level of list and record nesting.
    fn fit_record(
        &self,
        fields: &[(Name, Constant)],
        record: TyId,
        fills: bool,
    ) -> Option<Constant> {
        let expected = self.types.record_fields(record)?;
        let mut fitted = Vec::with_capacity(expected.len().max(fields.len()));
        for (name, value) in fields {
            let value = match self.types.field_or_rest(record, *name) {
                Some(ty) => self.fit_constant(value, ty)?,
                // Checked on its own, an integer settling to `Int`.
                None => value.clone(),
            };
            fitted.push((*name, value));
        }
        let mut filled: Vec<(Name, Constant)> = Vec::new();
        for &(name, _) in expected {
            if fields.iter().any(|&(written, _)| written == name) {
                continue;
            }
            let default = self.types.field_default(record, name).filter(|_| fills)?;
            filled.push((name, default.clone()));
        }
        let names = self.ast.names();
        filled.sort_unstable_by(|(a, _), (b, _)| names.get(*a).cmp(names.get(*b)));
        fitted.extend(filled);
        // `{}` where a record type with a rest-field but no defaulted field
        // is expected is given nothing.
        if fitted.is_empty() {
            return Some(Constant::Empty);
        }
        Some(Constant::Record(fitted.into_boxed_slice()))
    }

    /// Narrows the callback parameters `callbacks[start..]` of the impure or
    /// memoised function whose parameters' types are `fn_params[base..]`,
    /// whose body has just been checked without an error (see the module
    /// documentation): each narrowable position of a callback parameter
    /// that the body never uses as a value, and to which every call of it
    /// gives an argument that fits the position's present type, becomes
    /// that present type, in `fn_params`. Returns whether any did. For each
    /// callback parameter left unnarrowed at some narrowable position, what
    /// kept it so is recorded in `unnarrowed`, at the parameter's index,
    /// the vector being filled with `None` first.
    fn narrow_callbacks(
        &mut self,
        base: usize,
        start: usize,
        unnarrowed: &mut Vec<Option<Unnarrowed>>,
    ) -> bool {
        let count = self.fn_params.len().saturating_sub(base);
        let mut narrowed = false;
        for index in start..self.callbacks.len() {
            let Some(callback) = self.callbacks.get(index).copied() else {
                continue;
            };
            let Some(&(_, declared)) = self.fn_params.get(base + callback.param) else {
                continue;
            };
            let Some(sig) = self.types.fn_sig(declared).cloned() else {
                continue;
            };
            let positions = self
                .callback_positions
                .get(callback.positions..callback.positions + callback.count)
                .unwrap_or_default();
            // The parameters as narrowed, and as they would be had every
            // narrowable position been narrowed.
            let mut params = Vec::with_capacity(sig.params.len());
            let mut every = Vec::with_capacity(sig.params.len());
            let mut blocks = Vec::with_capacity(sig.params.len());
            for (at, &(name, written)) in sig.params.iter().enumerate() {
                let Some((present, given)) = positions.get(at).and_then(|position| {
                    position
                        .narrowable
                        .map(|(_, present)| (present, position.given))
                }) else {
                    params.push((name, written));
                    every.push((name, written));
                    blocks.push(None);
                    continue;
                };
                // The first in source order of what keeps it as written.
                let block = [
                    given.map(NarrowingBlock::Given),
                    callback.used.map(NarrowingBlock::Used),
                ]
                .into_iter()
                .flatten()
                .min_by_key(|block| block.span().start);
                params.push((name, if block.is_none() { present } else { written }));
                every.push((name, present));
                blocks.push(block);
            }
            // A meta is kept as declared: only parameters are narrowed.
            let ty = self
                .types
                .fn_type_with_meta(params, sig.ret, sig.effect, sig.meta);
            if ty != declared {
                if let Some(entry) = self.fn_params.get_mut(base + callback.param) {
                    entry.1 = ty;
                }
                narrowed = true;
            }
            if blocks.iter().any(Option::is_some) {
                let every = self
                    .types
                    .fn_type_with_meta(every, sig.ret, sig.effect, sig.meta);
                if unnarrowed.is_empty() {
                    unnarrowed.resize(count, None);
                }
                if let Some(entry) = unnarrowed.get_mut(callback.param) {
                    *entry = Some(Unnarrowed {
                        narrowed: every,
                        blocks: blocks.into(),
                    });
                }
            }
        }
        narrowed
    }

    /// Records that the argument `arg`, checked against `param_ty`, fills
    /// the parameter number `param` of the callback parameter
    /// `callbacks[callback]` in a call of it: if the position is
    /// narrowable and what the argument is once every await marked on it
    /// has run (see [`Checker::value_of`], [`Checker::inferred`]) does not
    /// fit its present type, the argument keeps the position as written.
    fn record_argument(&mut self, callback: usize, param: usize, arg: ExprId, param_ty: TyId) {
        let Some(at) = self
            .callbacks
            .get(callback)
            .filter(|callback| param < callback.count)
            .map(|callback| callback.positions + param)
        else {
            return;
        };
        let Some(&CallbackPosition {
            narrowable: Some((written, present)),
            given,
        }) = self.callback_positions.get(at)
        else {
            return;
        };
        let span = self.ast.get(arg).span;
        // An earlier argument keeps it as written already.
        if given.is_some_and(|given| given.start <= span.start) {
            return;
        }
        let value = self.value_of(arg, param_ty);
        let found = self.inferred(written, param_ty, value);
        if !self.types.is_subtype(found, present)
            && let Some(position) = self.callback_positions.get_mut(at)
        {
            position.given = Some(span);
        }
    }

    /// The inferred result of an impure or memoised function whose result
    /// type is written as `ty`, declared `declared`, whose body is `body`
    /// and whose `return`s are `returns` (see [Inferred
    /// results](#inferred-results)): decided from all its result sites, the
    /// body's own result and every value returned, as if they were the
    /// branches of one `if`. Each site is inferred against the declared
    /// type as the body alone would be (see [`Checker::inferred`]), a site
    /// of type `Never` playing no part, and the result is the join of what
    /// they give; the declared type if that is not a subtype of it. A
    /// function every site of which is `Never` is inferred from `Never`.
    fn inferred_result(
        &mut self,
        (ty, declared): (TypeExprId, TyId),
        body: ExprId,
        returns: &[Site],
    ) -> TyId {
        if returns.is_empty() {
            let value = self.value_of(body, declared);
            return self.inferred(ty, declared, value);
        }
        let mut joined = None;
        let sites = std::iter::once(Some(body)).chain(returns.iter().map(|site| site.value));
        for value in sites {
            let found = match value {
                Some(value) if self.awaited(value) == self.never => continue,
                Some(value) => {
                    let value = self.value_of(value, declared);
                    self.inferred(ty, declared, value)
                }
                // A `return` without a value returns `{}`.
                None => self.inferred(ty, declared, BodyValue::Typed(TyId::EMPTY)),
            };
            joined = Some(match joined {
                None => found,
                Some(so_far) => self.types.join(so_far, found).0,
            });
        }
        match joined {
            None => {
                let never = self.never;
                self.inferred(ty, declared, BodyValue::Typed(never))
            }
            Some(joined) if self.types.is_subtype(joined, declared) => joined,
            Some(_) => declared,
        }
    }

    /// Whether a pure function's parameter whose type is written as `ty` is
    /// inferred: present if the body measures it, the either shape if not.
    /// So is one written bare, as a name, a record type, a list type or a
    /// union with no `Future` member (see [`Checker::is_bare`]); not one
    /// written as `Present<...>` or `Future<...>` or a union with a
    /// `Future` member, which says what it is, nor as a function type,
    /// which is never made the either shape at its own position (see the
    /// module documentation).
    fn is_inferred(&self, ty: TypeExprId) -> bool {
        self.is_bare(ty)
    }

    /// The inferred result of an impure function at the position of its
    /// result type written as `ty`, whose declared type is `declared`, where
    /// the body returns `value` (see the module documentation). A position
    /// written bare, as a name, a record type, a list type or a union with
    /// no `Future` member, whose declared type is therefore the either
    /// shape of a `T`, is `T` if the value is
    /// present, `Future<T>` if it is a future, and the either shape, as
    /// declared, if it may be either or is a provisional type; for a record
    /// type, `T` is the record of its declared fields, each inferred in turn
    /// against the field of the same name of the value once stripped, and
    /// for a list type, the list of its declared element type inferred in
    /// turn against the element type of the value once stripped. Every other
    /// position keeps its declared type, and so does a bare one whose value
    /// is not known well enough: a record block, or a record its dotted
    /// paths build, where a name or a list type is declared, or a value
    /// whose type is not a subtype of what would be inferred.
    ///
    /// Recurses once per level of record or list type nesting in `ty`.
    fn inferred(&mut self, ty: TypeExprId, declared: TyId, value: BodyValue) -> TyId {
        // The fields of a record type, or the element type of a list type.
        let (fields, element) = match self.ast.type_expr(ty).kind {
            // A bare union is inferred at its top only, as a name is.
            TypeExprKind::Named(_) => (None, None),
            TypeExprKind::Union { .. } if self.is_bare(ty) => (None, None),
            TypeExprKind::Record(fields) => (Some(fields), None),
            TypeExprKind::List(element) => (None, Some(element)),
            TypeExprKind::Applied { name, arg, .. } if Some(name) == self.list_name => {
                (None, Some(arg))
            }
            // They say what they are, a function type at its own position
            // included; and a type that names nothing is erroneous.
            TypeExprKind::Applied { .. }
            | TypeExprKind::Union { .. }
            | TypeExprKind::Fn { .. }
            | TypeExprKind::Path(_)
            | TypeExprKind::Ref(_)
            | TypeExprKind::Invalid => {
                return declared;
            }
        };
        if self.types.laziness(declared) != Laziness::Either {
            return declared;
        }
        let present = self.types.strip(declared);
        let (laziness, stripped) = match value {
            // A record block is present, and so is a record its dotted
            // paths build.
            BodyValue::Fields(_) | BodyValue::Open(_) => (Laziness::Present, value),
            BodyValue::Typed(found) => (
                self.types.laziness(found),
                BodyValue::Typed(self.types.strip(found)),
            ),
        };
        let shape = match (fields, element, stripped) {
            (Some(fields), _, stripped) => match self.inferred_record(fields, present, stripped) {
                Some(record) => record,
                None => return declared,
            },
            (None, Some(element), BodyValue::Typed(found)) => {
                let (Some(declared_element), Some(found_element)) = (
                    self.types.list_element(present),
                    self.types.list_element(found),
                ) else {
                    return declared;
                };
                let element =
                    self.inferred(element, declared_element, BodyValue::Typed(found_element));
                self.types.list(element)
            }
            (None, None, BodyValue::Typed(_)) => present,
            (None, _, BodyValue::Fields(_) | BodyValue::Open(_)) => return declared,
        };
        let inferred = match laziness {
            Laziness::Present => shape,
            Laziness::Future => self.types.future(shape),
            Laziness::Either | Laziness::Provisional => self.types.deferrable(shape),
        };
        match value {
            BodyValue::Typed(found) if !self.types.is_subtype(found, inferred) => declared,
            _ => inferred,
        }
    }

    /// The present member of the inferred result at a record type written
    /// with the fields `fields`, whose declared present member is the record
    /// type `present`, where the body returns `value`, stripped: the record
    /// of the declared fields, each inferred against the value's field of
    /// the same name (see [`Checker::inferred`]), or keeping its declared
    /// type if the value has no such field. A field of a record block, or
    /// of a record its dotted paths build, that dotted paths build is
    /// inferred as such a record in turn. The value's other fields play no
    /// part. `None` if `present` lacks a field written, which a record type
    /// resolved from `fields` never does.
    ///
    /// The declared fields are kept in `sorted_fields`, by name, each with
    /// its position in `fields` and its type so far, so that a record
    /// block's fields, and the children of a segment its dotted paths
    /// build, are found by binary search, in one pass over its items (see
    /// [`Checker::infer_block_fields`]) or over the segment's children.
    fn inferred_record(
        &mut self,
        fields: FieldTyRange,
        present: TyId,
        value: BodyValue,
    ) -> Option<TyId> {
        let ast = self.ast;
        let written = ast.field_tys(fields);
        let base = self.sorted_fields.len();
        for (index, field) in written.iter().enumerate() {
            // The expansion of the file's types leaves no spread: a type
            // with one resolves to `Error`, which is not inferred. A
            // rest-field is kept as declared.
            let Some(name) = field.name() else {
                continue;
            };
            let Some(declared) = self.types.field(present, name) else {
                self.sorted_fields.truncate(base);
                return None;
            };
            self.sorted_fields.push((name, position(index), declared));
        }
        // Inferring a nested record type uses `sorted_fields` above `end` and
        // truncates back.
        let end = self.sorted_fields.len();
        self.sorted_fields
            .get_mut(base..end)
            .unwrap_or_default()
            .sort_unstable_by_key(|&(name, _, _)| name);
        match value {
            BodyValue::Typed(found) => {
                for at in base..end {
                    let Some(&(name, index, declared)) = self.sorted_fields.get(at) else {
                        continue;
                    };
                    let (Some(field), Some(found)) =
                        (written.get(index as usize), self.types.field(found, name))
                    else {
                        continue;
                    };
                    let ty = self.inferred(field.ty, declared, BodyValue::Typed(found));
                    if let Some(entry) = self.sorted_fields.get_mut(at) {
                        entry.2 = ty;
                    }
                }
            }
            BodyValue::Fields(items) => self.infer_block_fields(written, (base, end), items),
            BodyValue::Open(node) => {
                let mut child = self
                    .built
                    .get(node as usize)
                    .map_or(NO_NODE, |entry| entry.first_child);
                while let Some(&entry) = self.built.get(child as usize) {
                    let definition = match entry.leaf {
                        Some(value) => Definition::Whole(value),
                        None => Definition::Dotted(child),
                    };
                    if let Some(at) = self.sorted_at((base, end), entry.name) {
                        self.infer_field(written, at, definition);
                    }
                    child = entry.next_sibling;
                }
            }
        }
        let sorted: Vec<(Name, TyId)> = self
            .sorted_fields
            .get(base..end)
            .unwrap_or_default()
            .iter()
            .map(|&(name, _, ty)| (name, ty))
            .collect();
        // Sorted by name, so interning does not sort it again; the declared
        // fields keep their defaults (see [Field defaults](#field-defaults)).
        let rest = self.types.record_rest(present);
        let record = self.types.record_like(present, sorted, rest);
        self.sorted_fields.truncate(base);
        Some(record)
    }

    /// Infers each of the declared fields `sorted_fields[base..end]`,
    /// written as `written`, that the record block of `items` defines, in
    /// one pass over its items: a field defined whole from its value, a
    /// soft field from the type its spread's operand gives it, and an open
    /// field from the leaves of the dotted paths that build it, which the
    /// pass rebuilds in `built`, as a tree of segments under one root for
    /// each declared field, to be inferred once it ends. A field, or a
    /// segment, that the declared type lacks plays no part.
    ///
    /// Only the definition of a field that stands is inferred: a soft one
    /// may be overridden by a later spread or field item. So from the
    /// block's first spread on, each definition is recorded in `standing`,
    /// a later one replacing an earlier one, and inferred once the pass
    /// ends; before it, a field item cannot be overridden, the block having
    /// checked without an error, and is inferred at once.
    fn infer_block_fields(
        &mut self,
        written: &[FieldTy],
        (base, end): (usize, usize),
        items: ItemRange,
    ) {
        let ast = self.ast;
        // The root for `sorted_fields[at]` is at `roots + (at - base)`.
        // Inferring a value pushes onto `built` above what is there and
        // truncates back.
        let roots = self.built.len();
        self.built.extend(
            self.sorted_fields
                .get(base..end)
                .unwrap_or_default()
                .iter()
                .map(|&(name, _, _)| Built {
                    name,
                    leaf: None,
                    first_child: NO_NODE,
                    next_sibling: NO_NODE,
                }),
        );
        // The definition that stands for `sorted_fields[at]` is at
        // `standing + (at - base)`, once the block has had a spread.
        let standing = self.standing.len();
        let mut spread = false;
        for &item in ast.items(items) {
            let (path, value) = match ast.item(item).kind {
                ItemKind::Field { path, value, .. } => (path, value),
                ItemKind::Spread(operand) => {
                    if !spread {
                        spread = true;
                        self.standing.resize(standing + (end - base), None);
                    }
                    self.stand_spread(operand, (base, end), standing);
                    continue;
                }
                _ => continue,
            };
            let Some((first, rest)) = ast.path(path).split_first() else {
                continue;
            };
            let Some(at) = first
                .name()
                .and_then(|name| self.sorted_at((base, end), name))
            else {
                continue;
            };
            if !rest.is_empty() {
                self.build_path(position(roots + (at - base)), rest, value);
            } else if spread {
                if let Some(entry) = self.standing.get_mut(standing + (at - base)) {
                    *entry = Some(Definition::Whole(value));
                }
            } else {
                self.infer_field(written, at, Definition::Whole(value));
            }
        }
        if spread {
            for at in base..end {
                if let Some(Some(definition)) = self.standing.get(standing + (at - base)).copied() {
                    self.infer_field(written, at, definition);
                }
            }
            self.standing.truncate(standing);
        }
        for at in base..end {
            let root = position(roots + (at - base));
            // A field no dotted path builds.
            if self
                .built
                .get(root as usize)
                .is_none_or(|entry| entry.first_child == NO_NODE)
            {
                continue;
            }
            self.infer_field(written, at, Definition::Dotted(root));
        }
        self.built.truncate(roots);
    }

    /// Records, in `standing` from `standing` on, that the spread whose
    /// operand is `operand` defines each of the declared fields
    /// `sorted_fields[base..end]` that its operand's type has, once awaited,
    /// with the type it gives the field, which is what the spread reads
    /// from the operand and stores, never awaited.
    fn stand_spread(&mut self, operand: ExprId, (base, end): (usize, usize), standing: usize) {
        let awaited = self.awaited(operand);
        let record = self.types.strip(awaited);
        for &(name, ty) in self.types.record_fields(record).unwrap_or_default() {
            let Some(offset) = self
                .sorted_fields
                .get(base..end)
                .unwrap_or_default()
                .binary_search_by_key(&name, |&(name, _, _)| name)
                .ok()
            else {
                continue;
            };
            if let Some(entry) = self.standing.get_mut(standing + offset) {
                *entry = Some(Definition::Soft(ty));
            }
        }
    }

    /// Adds the dotted path whose segments after the first are `rest`,
    /// ending in `value`, to the tree of segments in `built` whose root, the
    /// path's first segment, is `root`: each segment that is not yet a child
    /// of the one before it is added as its newest child, and the last is
    /// the leaf holding `value`.
    fn build_path(&mut self, root: u32, rest: &[PathSegment], value: ExprId) {
        let mut node = root;
        // A field's path's segments are all names.
        for name in rest.iter().filter_map(PathSegment::name) {
            if let Some(child) = self.built_child(node, name) {
                node = child;
                continue;
            }
            let index = position(self.built.len());
            let next_sibling = match self.built.get_mut(node as usize) {
                Some(parent) => std::mem::replace(&mut parent.first_child, index),
                None => NO_NODE,
            };
            self.built.push(Built {
                name,
                leaf: None,
                first_child: NO_NODE,
                next_sibling,
            });
            node = index;
        }
        if let Some(leaf) = self.built.get_mut(node as usize) {
            leaf.leaf = Some(value);
        }
    }

    /// The child of the segment `node` of `built` named `name`, if it has
    /// one.
    fn built_child(&self, node: u32, name: Name) -> Option<u32> {
        let mut child = self.built.get(node as usize)?.first_child;
        while let Some(entry) = self.built.get(child as usize) {
            if entry.name == name {
                return Some(child);
            }
            child = entry.next_sibling;
        }
        None
    }

    /// The index in `sorted_fields` of the entry named `name` among those
    /// in `base..end`, which are sorted by name, if there is one.
    fn sorted_at(&self, (base, end): (usize, usize), name: Name) -> Option<usize> {
        self.sorted_fields
            .get(base..end)
            .unwrap_or_default()
            .binary_search_by_key(&name, |&(name, _, _)| name)
            .ok()
            .map(|offset| base + offset)
    }

    /// Infers the declared field `sorted_fields[at]`, written as one of
    /// `written`, which the record the body returns defines as
    /// `definition`, and records its inferred type there (see
    /// [`Checker::inferred`]): a value defined whole as it is once every
    /// await marked for fitting it to the field's declared type has run
    /// (see [`Checker::value_of`]), whether it is a field's or the leaf's
    /// of a dotted path, which is checked against the declared type at its
    /// path as a field's value is against its field's; a soft field as the
    /// type its spread's operand gives it; and a record dotted paths build
    /// as present, its fields inferred in turn.
    fn infer_field(&mut self, written: &[FieldTy], at: usize, definition: Definition) {
        let Some(&(_, index, declared)) = self.sorted_fields.get(at) else {
            return;
        };
        let Some(field) = written.get(index as usize) else {
            return;
        };
        let found = match definition {
            Definition::Whole(value) => self.value_of(value, declared),
            Definition::Soft(ty) => BodyValue::Typed(ty),
            Definition::Dotted(node) => BodyValue::Open(node),
        };
        let ty = self.inferred(field.ty, declared, found);
        if let Some(entry) = self.sorted_fields.get_mut(at) {
            entry.2 = ty;
        }
    }

    /// What the expression `expr`, checked where a value of type `expected`
    /// was expected, is known to yield where it is used: the record of its
    /// fields if it is a record block that was checked field by field
    /// against `expected` (on the condition [`Checker::check_expr_expecting`]
    /// and [`Checker::check_fn_body`] have), whose type is then the record
    /// of the expected fields' types rather than of what its fields hold;
    /// and otherwise its type once the await marked on it has run: as it
    /// is if none is, stripped for an await at the top, and made present
    /// for a deep one, which awaits every field of its type's record whose
    /// type is lazy, at any depth, as its lazy shape lists them (see
    /// [`TypeTable::present`]).
    fn value_of(&mut self, expr: ExprId, expected: TyId) -> BodyValue {
        if self.types.laziness(expected) != Laziness::Future
            && self.types.is_record(self.types.strip(expected))
            && let Some((_, items)) = self.record_block_within(expr)
        {
            return BodyValue::Fields(items);
        }
        BodyValue::Typed(self.awaited(expr))
    }

    /// The type of what `expr` yields where it is used, once the await
    /// marked on it has run: its type as it is if none is, stripped for an
    /// await at the top, and made present for a deep one (see
    /// [`Checker::value_of`]).
    fn awaited(&mut self, expr: ExprId) -> TyId {
        let ty = self.expr_types[expr.index()];
        match self.awaits[expr.index()] {
            AwaitMark::None => ty,
            AwaitMark::Top => self.types.strip(ty),
            AwaitMark::Deep => self.types.present(ty),
        }
    }

    /// Settles every provisional type of function `function`, the function
    /// expression `id` (for a default function, which has none, its body:
    /// see [Calls](#calls)), whose body has been checked, by `decisions`, which
    /// says for each of its parameters whether the body measures it: in the
    /// type of every expression of the function, its own included, which
    /// are the ids of its subtree, and in what every function and `lazy`
    /// nested in it captures. A provisional type reaches nothing outside
    /// the function but through the function's type, which is settled here
    /// with the rest.
    fn settle_function(&mut self, id: ExprId, function: FnId, decisions: &[bool]) {
        let ast = self.ast;
        for node in ast.subtree(id) {
            if let Some(ty) = self.expr_types.get(node.index()).copied() {
                let settled = self.types.settle(ty, function, decisions);
                self.expr_types[node.index()] = settled;
            }
            let (ExprKind::Fn(nested) | ExprKind::Lazy(nested)) = ast.get(node).kind else {
                continue;
            };
            // A function's default functions are nested in it with no node
            // of their own (see [Calls](#calls)).
            let defaults = ast.params(ast.fn_decl(nested).params);
            let defaults = defaults.iter().filter_map(|param| param.default_fn);
            for nested in std::iter::once(nested).chain(defaults) {
                let Some(resolution) = self.fn_resolutions.get_mut(nested.index()) else {
                    continue;
                };
                for (_, ty) in &mut resolution.captures {
                    *ty = self.types.settle(*ty, function, decisions);
                }
            }
        }
    }

    /// Types the call `id` of `callee` with the arguments `args` (see the
    /// module documentation): the callee's return type, or `Error` if the
    /// callee is not a function or the arguments do not fit its parameters.
    /// Every argument is checked whatever goes wrong. A callee that may be
    /// a future is awaited. A call of a callback parameter through its name
    /// records what it gives each position (see
    /// [`Checker::record_argument`]), and is not a use of it as a value.
    fn check_call(&mut self, id: ExprId, callee: ExprId, args: ArgRange) -> TyId {
        // TEMPORARY (alias rule): the callee of a call may be a built-in whose typing is special,
        // or an alias of one (see the [inference
        // limits](https://learn.peios.org/gcsl/language/inference-limits/)); the call is then typed
        // by its
        // rule.
        let outer = self.alias_site.replace(callee);
        let outer_call = self.call_site.replace(callee);
        let (callee_ty, callback) = self.check_callee(callee);
        (self.alias_site, self.call_site) = (outer, outer_call);
        if let TyKind::Builtin(builtin) = *self.types.kind(callee_ty) {
            return self.check_builtin_call(id, (callee, builtin), args);
        }
        let callee_ty = self.measure(callee, callee_ty);
        let Some(sig) = self.types.fn_sig(callee_ty).cloned() else {
            // A callee of type `Never` is never there to call.
            if callee_ty == self.never {
                for arg in self.ast.args(args) {
                    self.check_alone(arg.value);
                }
                return self.never;
            }
            if callee_ty != TyId::ERROR {
                let callee_ty = self.settle(callee, callee_ty);
                let message = format!("cannot call a value of type {}", self.in_message(callee_ty));
                let label = format!("this is {}", self.in_label(callee_ty));
                let mut error = Diagnostic::error(message, self.ast.get(id).span)
                    .with_context(self.ast.get(callee).span, label)
                    .with_note("only functions can be called");
                if let Some(note) = self.union_note(callee, callee_ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
            }
            // An erroneous callee, its error reported where it is, would
            // have typed its arguments: what it would have typed is no
            // error of theirs (see [`Checker::check_list`]).
            let unknown = u32::from(callee_ty == TyId::ERROR);
            self.quiet.expecting_error = self.quiet.expecting_error.saturating_add(unknown);
            for arg in self.ast.args(args) {
                self.check_alone(arg.value);
            }
            self.quiet.expecting_error = self.quiet.expecting_error.saturating_sub(unknown);
            return TyId::ERROR;
        };
        let erroneous = self.bind_arguments(id, (callee, callee_ty, callback), args, &sig);
        if sig.effect != Effect::Pure {
            self.impure_calls.insert(id);
        }
        if let Some(function) = self.functions.last()
            && function.effect == Effect::Pure
            && sig.effect != Effect::Pure
        {
            let marker = function.marker;
            let label = format!("this is {}", self.in_label(callee_ty));
            self.errors.push(
                Diagnostic::error(
                    "cannot call an impure function from a pure function",
                    self.ast.get(id).span,
                )
                .with_context(self.ast.get(callee).span, label)
                .with_context(marker, PURE_CONTEXT)
                .with_note(
                    "a pure function, declared with `:`, may call only pure functions; declare \
                     it with `->` to make it impure",
                ),
            );
        }
        if erroneous { TyId::ERROR } else { sig.ret }
    }

    /// TEMPORARY (alias rule): types the call `id` of the built-in function `builtin`, whose typing
    /// is special, the callee `callee` (a reference to it or to an alias of it, already checked),
    /// with the arguments `args` (see the module documentation): the arguments are bound to the
    /// built-in's parameters as a function's are (see [`Checker::bind_builtin_arguments`]), and
    /// then typed as the built-in says, each built-in its own way. The callee's type is the
    /// function type the built-in has at this call, and it resolves to [`VarRef::Builtin`], so that
    /// lowering makes the call the built-in's own node without reading the callee. Calling a
    /// built-in that is not pure from a pure function is an error, as for any function. Replaced by
    /// an ordinary call when built-in functions get real polymorphic types (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    fn check_builtin_call(
        &mut self,
        id: ExprId,
        (callee, builtin): (ExprId, Builtin),
        args: ArgRange,
    ) -> TyId {
        self.expr_vars[callee.index()] = VarRef::Builtin(builtin);
        let base = self.filled.len();
        let erroneous = self.bind_builtin_arguments(id, (callee, builtin), args);
        // Each parameter's argument, as bound.
        let bound: Vec<ExprId> = {
            let arg_list = self.ast.args(args);
            self.filled
                .get(base..)
                .unwrap_or_default()
                .iter()
                .filter_map(|&arg| arg_list.get(arg as usize).map(|arg| arg.value))
                .collect()
        };
        self.filled.truncate(base);
        let ty = match (builtin.typing(), bound.as_slice()) {
            (Typing::Special, &[list]) if builtin == Builtin::Len && !erroneous => {
                self.check_len(callee, list)
            }
            (Typing::Special, &[value]) if builtin == Builtin::Show && !erroneous => {
                self.check_show(callee, value)
            }
            _ => {
                // Every argument is checked, whatever goes wrong.
                for &arg in &bound {
                    self.check_alone(arg);
                }
                TyId::ERROR
            }
        };
        if builtin.effect() != Effect::Pure {
            self.impure_calls.insert(id);
        }
        if let Some(function) = self.functions.last()
            && function.effect == Effect::Pure
            && builtin.effect() != Effect::Pure
        {
            let marker = function.marker;
            self.errors.push(
                Diagnostic::error(
                    "cannot call an impure function from a pure function",
                    self.ast.get(id).span,
                )
                .with_context(
                    self.ast.get(callee).span,
                    format!("`{}` is an impure built-in function", builtin.name()),
                )
                .with_context(marker, PURE_CONTEXT),
            );
        }
        ty
    }

    /// TEMPORARY (alias rule): `len`'s rule, until its polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)). Types the call of `len`,
    /// the callee
    /// `callee`, whose argument is `list`: the argument is used as its bits at the top (see
    /// [`Checker::measure`]), its elements not at all, and must be a list, or a union of lists,
    /// "`len` takes a list, found T". The call is an `Int`, and the callee a pure function from
    /// that type to `Int`; `Error` if the argument is not a list or is erroneous.
    fn check_len(&mut self, callee: ExprId, list: ExprId) -> TyId {
        let found = self.check_expr(list);
        let found = self.settle(list, found);
        let ty = self.measure(list, found);
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        // A list of type `Never` is never there to count.
        if ty == self.never {
            self.expr_types[callee.index()] =
                self.types
                    .fn_type([(None, ty)], TyId::INT, Builtin::Len.effect());
            return self.never;
        }
        if self.element_of(ty).is_none() {
            let shown = self.in_message(ty);
            let label = format!("this is {}", self.in_label(ty));
            let mut error = Diagnostic::error(
                format!("`{}` takes a list, found {shown}", Builtin::Len.name()),
                self.ast.get(list).span,
            )
            .with_label(label);
            if let Some(note) = self.union_note(list, ty) {
                error = error.with_note(note);
            }
            self.errors.push(error);
            return TyId::ERROR;
        }
        self.expr_types[callee.index()] =
            self.types
                .fn_type([(None, ty)], TyId::INT, Builtin::Len.effect());
        TyId::INT
    }

    /// TEMPORARY (alias rule): `show`'s rule, until its polymorphic type (see the [inference
    /// limits](https://learn.peios.org/gcsl/language/inference-limits/)). Types the call of `show`,
    /// the
    /// callee `callee`, whose argument is `value`: the argument may be of any type without a
    /// function in it, anywhere, a field, an element, a member of a union, an alias's expansion, at
    /// any depth ([`TypeTable::contains_fn`], which a built-in function's type is too), else
    /// "cannot show a function: `show` renders data as source", at the argument, its type in a
    /// note; and it is used as its bits all the way down, as an interpolated expression is
    /// ([`Checker::measure_deep`]), so one that may be a future, or hold one, is awaited deeply,
    /// subject to the purity rule. The call is a `String`, and the callee a pure function from the
    /// argument's type, once awaited, to `String`; `Error` if the argument is erroneous or holds a
    /// function.
    fn check_show(&mut self, callee: ExprId, value: ExprId) -> TyId {
        let found = self.check_expr(value);
        let found = self.settle(value, found);
        if found == TyId::ERROR {
            return TyId::ERROR;
        }
        // Before measuring, so that an argument with a function in it that
        // may also be a future in a pure function is one error, not two.
        if self.types.contains_fn(found) {
            let shown = self.in_message(found);
            self.errors.push(
                Diagnostic::error(
                    format!(
                        "cannot show a function: `{}` renders data as source",
                        Builtin::Show.name()
                    ),
                    self.ast.get(value).span,
                )
                .with_note(format!("the argument is of type {shown}")),
            );
            return TyId::ERROR;
        }
        let ty = self.measure_deep(value, found);
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        self.expr_types[callee.index()] =
            self.types
                .fn_type([(None, ty)], TyId::STRING, Builtin::Show.effect());
        // A value of type `Never` is never there to show.
        if ty == self.never {
            return self.never;
        }
        TyId::STRING
    }

    /// Binds the arguments `args` of the call `id` of the built-in function
    /// `builtin`, the callee `callee`, to its parameters, as
    /// [`Checker::bind_arguments`] binds a function's, by position or by
    /// the names its table gives, and records the argument filling each
    /// parameter above `filled`'s length on entry, which the caller
    /// truncates back. An argument that fills no parameter is checked alone
    /// here; the others are left to the caller. Reports every argument that
    /// cannot be bound and every parameter left unfilled with the errors a
    /// function's call reports, and returns whether there was any.
    fn bind_builtin_arguments(
        &mut self,
        id: ExprId,
        (callee, builtin): (ExprId, Builtin),
        args: ArgRange,
    ) -> bool {
        let ast = self.ast;
        let call = ast.get(id);
        let callee_span = ast.get(callee).span;
        let label = format!("`{}` is a built-in function", builtin.name());
        let params = builtin.params();
        let arg_list = ast.args(args);
        let count = params.len();
        let positional = arg_list
            .iter()
            .filter(|arg| arg.kind == ArgKind::Positional)
            .count();
        let trailing = arg_list.iter().any(|arg| arg.kind == ArgKind::Trailing);
        let named = arg_list
            .iter()
            .any(|arg| matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional + usize::from(trailing);
        let mut erroneous = false;
        if found > count {
            self.report_extra_arguments((call, callee_span, &label), args, count);
            erroneous = true;
        }
        let base = self.filled.len();
        self.filled.resize(base + count, u32::MAX);
        let mut next_positional = 0;
        for (index, arg) in arg_list.iter().enumerate() {
            let target = match arg.kind {
                ArgKind::Positional => {
                    next_positional += 1;
                    Some(next_positional - 1).filter(|&param| param < count)
                }
                ArgKind::Trailing => (positional < count).then(|| count - 1),
                ArgKind::Named { name, .. } => {
                    let text = ast.names().get(name);
                    let param = params.iter().position(|&param| param == text);
                    if param.is_none() {
                        let at = (*arg, (callee_span, label.as_str()));
                        self.errors
                            .push(unknown_builtin_argument(ast, arg_list, params, at));
                        erroneous = true;
                    }
                    param
                }
            };
            let Some(param) = target else {
                self.check_alone(arg.value);
                continue;
            };
            let first = self.filled.get(base + param).copied().unwrap_or(u32::MAX);
            if let Some(first) = arg_list.get(first as usize) {
                let name = params.get(param).and_then(|&text| ast.names().lookup(text));
                self.report_given_twice((param, name), first.span(ast), arg.span(ast));
                erroneous = true;
                self.check_alone(arg.value);
                continue;
            }
            if let Some(entry) = self.filled.get_mut(base + param) {
                *entry = position(index);
            }
        }
        let unfilled: Vec<&str> = params
            .iter()
            .zip(self.filled.get(base..).unwrap_or_default())
            .filter(|&(_, &arg)| arg == u32::MAX)
            .map(|(&text, _)| text)
            .collect();
        if !unfilled.is_empty() {
            erroneous = true;
            if !named && !trailing {
                let noun = if count == 1 { "argument" } else { "arguments" };
                self.errors.push(
                    Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                        .with_context(callee_span, label.as_str()),
                );
            } else {
                let close = call.close_span().unwrap_or(call.span);
                for text in unfilled {
                    self.errors.push(
                        Diagnostic::error(format!("missing argument `{text}`"), close)
                            .with_context(callee_span, label.as_str()),
                    );
                }
            }
        }
        if !erroneous {
            for (param, &arg) in self
                .filled
                .get(base..)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                if let Some(entry) = self.arg_params.get_mut(args.start() + arg as usize) {
                    *entry = position(param);
                }
            }
        }
        erroneous
    }

    /// TEMPORARY (alias rule): the type of the reference `id`, spanning `span`, of type `ty`: `ty`,
    /// unless it is the opaque type of a built-in function whose typing is special, or holds one (a
    /// record read whole with an alias of one among its fields), and `id` is not where the alias
    /// rule lets one stand ([`Checker::alias_site`]: the callee of a call, or the whole initialiser
    /// of a top-level field or `let` of one name). Then it is "`len` is a built-in function and
    /// cannot be used as a value yet", reported here, the one place the restriction is made, and
    /// `Error`. Replaced, with the opaque type, when built-in functions get real polymorphic types
    /// (see the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    fn builtin_use(&mut self, id: ExprId, ty: TyId, span: Span) -> TyId {
        let Some(builtin) = self.types.special_builtin(ty) else {
            return ty;
        };
        let alias = self.alias_site == Some(id) && self.types.is_builtin(ty);
        if alias {
            return ty;
        }
        let name = builtin.name();
        // What the built-in's type is for, which no type can write yet.
        let any = if builtin == Builtin::Show {
            "value"
        } else {
            "list"
        };
        self.errors.push(
            Diagnostic::error(
                format!("`{name}` is a built-in function and cannot be used as a value yet"),
                span,
            )
            .with_note(format!(
                "a built-in function with a type for any {any} cannot be written as a type \
                 yet; call it, or bind it to a name"
            )),
        );
        TyId::ERROR
    }

    /// Binds the arguments `args` of the call `id` to the parameters of
    /// its callee, of function type `callee_ty` with signature `sig`:
    /// positional arguments fill parameters left to right, the trailing
    /// block the last, and named arguments the parameters of their names.
    /// Checks each argument against its parameter's type, or alone if it
    /// fills none, and records the parameter each fills. A known callee
    /// gives each parameter left unfilled that has a default its default,
    /// which is recorded for the call (see [Calls](#calls)). Reports every
    /// argument that cannot be bound and every parameter left unfilled
    /// after that, and returns whether there was any.
    ///
    /// An argument awaited at the top because the parameter it fills is
    /// present by inference, as far as the callee is known, is reported in
    /// a pure caller in the call-site form (see
    /// [`Checker::argument_await_error`]). In a recursive call through the
    /// function's own name, an argument whose type is a provisional type of
    /// the function is measured. In a call of the callback parameter
    /// `callbacks[callback]`, if `callback` is `Some`, what each argument
    /// gives the position it fills is recorded (see
    /// [`Checker::record_argument`]). A mismatched argument filling a
    /// callback parameter of a known callee of this input that was not
    /// narrowed is labelled with what kept it so (see
    /// [`Checker::narrowing_label`]).
    fn bind_arguments(
        &mut self,
        id: ExprId,
        (callee, callee_ty, callback): (ExprId, TyId, Option<usize>),
        args: ArgRange,
        sig: &FnTy,
    ) -> bool {
        let ast = self.ast;
        let call = ast.get(id);
        let callee_span = ast.get(callee).span;
        let known = self.known_callee(callee);
        let recursive = self.is_recursive(callee);
        let function_label = format!("the function is {}", self.in_label(callee_ty));
        let arg_list = ast.args(args);
        let count = sig.params.len();
        // `positional` counts the positional arguments before the trailing
        // block; the trailing block, if any, comes after them all.
        let positional = arg_list
            .iter()
            .filter(|arg| arg.kind == ArgKind::Positional)
            .count();
        let trailing = arg_list.iter().any(|arg| arg.kind == ArgKind::Trailing);
        let named = arg_list
            .iter()
            .any(|arg| matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional + usize::from(trailing);
        let mut erroneous = false;
        if found > count {
            self.report_extra_arguments((call, callee_span, &function_label), args, count);
            erroneous = true;
        }
        let base = self.filled.len();
        self.filled.resize(base + count, u32::MAX);
        let mut next_positional = 0;
        for (index, arg) in arg_list.iter().enumerate() {
            let target = match arg.kind {
                ArgKind::Positional => {
                    next_positional += 1;
                    Some(next_positional - 1).filter(|&param| param < count)
                }
                ArgKind::Trailing => (positional < count).then(|| count - 1),
                ArgKind::Named {
                    name,
                    span,
                    shorthand,
                } => {
                    let param = sig
                        .params
                        .iter()
                        .position(|&(param, _)| param == Some(name));
                    if param.is_none() {
                        let filled = filled_by(arg_list, count, |name| {
                            sig.params
                                .iter()
                                .position(|&(param, _)| param == Some(name))
                        });
                        self.report_unknown_argument(
                            (name, span, shorthand),
                            (callee, callee_ty),
                            sig,
                            &filled,
                        );
                        erroneous = true;
                    }
                    param
                }
            };
            let Some(param) = target else {
                self.check_alone(arg.value);
                continue;
            };
            let first = self.filled.get(base + param).copied().unwrap_or(u32::MAX);
            if let Some(first) = arg_list.get(first as usize) {
                let name = sig.params.get(param).and_then(|&(name, _)| name);
                self.report_given_twice((param, name), first.span(ast), arg.span(ast));
                erroneous = true;
                self.check_alone(arg.value);
                continue;
            }
            if let Some(entry) = self.filled.get_mut(base + param) {
                *entry = position(index);
            }
            let param_ty = sig.params.get(param).map_or(TyId::ERROR, |&(_, ty)| ty);
            let context = self.argument_context(known, param, (id, callee));
            let found = self.check_expr_expecting_at(
                arg.value,
                param_ty,
                (callee_span, &function_label),
                context,
            );
            // A recursive call measures every argument that is a parameter
            // of the function, conservatively.
            if recursive && self.types.laziness(found) == Laziness::Provisional {
                self.claim_top(ast.get(arg.value).span, found);
            }
            if let Some(callback) = callback {
                self.record_argument(callback, param, arg.value, param_ty);
            }
        }
        let defaults = known.map(|known| self.fill_defaults(known, base, count));
        erroneous |= self.report_unfilled(
            base,
            (call, callee_span, &function_label),
            sig,
            (found, named, trailing),
        );
        if !erroneous {
            self.record_bound((id, args), base, defaults);
        }
        self.filled.truncate(base);
        erroneous
    }

    /// Records, for lowering, the parameter each of the arguments `args` of
    /// the call `id` fills, as `filled[base..]` shows, and the defaults the
    /// call gives, `defaults`, if it gives any (see [Calls](#calls)).
    fn record_bound(
        &mut self,
        (id, args): (ExprId, ArgRange),
        base: usize,
        defaults: Option<Vec<(u32, ParamDefault)>>,
    ) {
        for (param, &arg) in self
            .filled
            .get(base..)
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if arg == DEFAULTED {
                continue;
            }
            if let Some(entry) = self.arg_params.get_mut(args.start() + arg as usize) {
                *entry = position(param);
            }
        }
        if let Some(defaults) = defaults.filter(|defaults| !defaults.is_empty()) {
            self.lowering.call_defaults.insert(id, defaults.into());
        }
    }

    /// Gives each of the `count` parameters of `known`, a known callee, that
    /// `filled[base..]` shows no argument fills and that has a default its
    /// default (see [Calls](#calls)): marks it [`DEFAULTED`] there, so that
    /// it is not unfilled, and returns each with its default, in parameter
    /// order (a default in error, which fails the check with its own error,
    /// left out).
    fn fill_defaults(
        &mut self,
        known: KnownCallee,
        base: usize,
        count: usize,
    ) -> Vec<(u32, ParamDefault)> {
        let mut defaults = Vec::new();
        for param in 0..count {
            if self.filled.get(base + param) != Some(&u32::MAX) || !self.has_default(known, param) {
                continue;
            }
            if let Some(entry) = self.filled.get_mut(base + param) {
                *entry = DEFAULTED;
            }
            if let Some(default) = self.default_of(known, param) {
                defaults.push((position(param), default));
            }
        }
        defaults
    }

    /// The function that `callee` calls, if it is known: `callee` is a
    /// plain name whose binding was initialised directly with a function
    /// expression of this input, or declared by an earlier session input
    /// (whose measured parameters and defaults the scope carries), or the
    /// path of a
    /// top-level function of this input named by a dotted path once its
    /// group is checked, as a plain function's binding is then.
    fn known_callee(&self, callee: ExprId) -> Option<KnownCallee> {
        if let Some(&node) = self.imported_callees.get(&callee) {
            let known = self.workspace.and_then(|workspace| workspace.known(node));
            return known.map(|_| KnownCallee::Imported(node));
        }
        if let ExprKind::Field { .. } = self.ast.get(callee).kind {
            if let Some(node) = self.committed_callee(callee) {
                return Some(KnownCallee::Imported(node));
            }
            // A reference reads its node's cell: a slot of the top level's
            // frame, from top-level code or from inside a function.
            let slot = match self.expr_vars.get(callee.index()) {
                Some(&VarRef::Fn(slot)) => slot,
                Some(&VarRef::Local(slot)) if self.functions.is_empty() => slot,
                _ => return None,
            };
            let top = self.top_fn_in(slot)?;
            let known =
                top.leaf.is_some() && top.state == BodyState::Done && top.ready != usize::MAX;
            return known.then_some(KnownCallee::Current(top.function));
        }
        let ExprKind::Name(name) = self.ast.get(callee).kind else {
            return None;
        };
        let (index, binding) = self.lookup(name)?;
        match binding.function {
            Some(function) => Some(KnownCallee::Current(function)),
            None if index < self.scope_known.len() => Some(KnownCallee::Earlier(index)),
            None => None,
        }
    }

    /// The merged node of `callee`, if it is the path of a `def` named by a
    /// dotted path that another file of the directory chain declares, read
    /// through the root the chain committed (an access of the root's
    /// record, which knows no function), whose file is checked: a call of
    /// it is of a known callee, as a call of an imported `def` is, or of a
    /// `def` of one name of the chain, through its committed binding.
    fn committed_callee(&self, callee: ExprId) -> Option<u32> {
        let workspace = self.workspace?;
        let (node, at) = workspace.scope.reference(self.ast, callee)?;
        if at != callee {
            return None;
        }
        let mut base = callee;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let (index, _) = self.lookup(name)?;
        if index >= self.top_start {
            return None;
        }
        let global = workspace.scope.merged(node)?;
        workspace.known(global).map(|_| global)
    }

    /// How an argument of the call `call` of `callee`, whose function is
    /// `known` if it is known, that fills its parameter number `param` is
    /// checked: how an await of it is reported in a pure caller (see
    /// [`Checker::argument_site`]), and, for a callee of this input, the
    /// callee's function and the parameter, for the label on a mismatch
    /// (see [`Checker::narrowing_label`]).
    fn argument_context(
        &self,
        known: Option<KnownCallee>,
        param: usize,
        (call, callee): (ExprId, ExprId),
    ) -> (AwaitSite, Option<(FnId, usize)>) {
        match known {
            Some(known @ KnownCallee::Current(function)) => (
                self.argument_site(known, param, (call, callee)),
                Some((function, param)),
            ),
            Some(known @ (KnownCallee::Earlier(_) | KnownCallee::Imported(_))) => {
                (self.argument_site(known, param, (call, callee)), None)
            }
            None => (AwaitSite::Implicit, None),
        }
    }

    /// How an await of an argument of the call `call` of `callee`, a known
    /// callee `known`, that fills its parameter number `param` is reported
    /// in a pure caller: [`AwaitSite::Argument`] if the parameter is present
    /// because the function's body measures it, with where the body first
    /// does and the parameter's name if the function is one of this
    /// input's; [`AwaitSite::Implicit`] otherwise.
    fn argument_site(
        &self,
        known: KnownCallee,
        param: usize,
        (call, callee): (ExprId, ExprId),
    ) -> AwaitSite {
        let argument = |measured| {
            AwaitSite::Argument(CallSite {
                call,
                callee,
                measured,
            })
        };
        match known {
            KnownCallee::Current(function) => {
                let demand = self
                    .fn_resolutions
                    .get(function.index())
                    .and_then(|resolution| resolution.param_demands.get(param).copied().flatten());
                let decl = self.ast.fn_decl(function);
                let name = self.ast.params(decl.params).get(param);
                match (demand, name) {
                    (Some(span), Some(name)) => argument(Some((span, name.name))),
                    _ => AwaitSite::Implicit,
                }
            }
            KnownCallee::Earlier(_) | KnownCallee::Imported(_) => {
                let measured = self
                    .known_params(known)
                    .is_some_and(|known| known.measured(param));
                if measured {
                    argument(None)
                } else {
                    AwaitSite::Implicit
                }
            }
        }
    }

    /// What a call of `known`, a known callee whose tree is gone, needs to
    /// know of its parameters ([`KnownParams`]): the scope's entry for a
    /// function of an earlier input, the workspace's for a `def` of another
    /// file; `None` for a function of this input, and if it is not known.
    fn known_params(&self, known: KnownCallee) -> Option<&KnownParams> {
        match known {
            KnownCallee::Current(_) => None,
            KnownCallee::Earlier(index) => self.scope_known.get(index)?.as_deref(),
            KnownCallee::Imported(node) => self.workspace?.known(node),
        }
    }

    /// Whether parameter number `param` of `known`, a known callee, has a
    /// default (see [Calls](#calls)): as its function expression declares,
    /// for a function of this input, and as its [`KnownParams`] say for one
    /// whose tree is gone.
    fn has_default(&self, known: KnownCallee, param: usize) -> bool {
        match known {
            KnownCallee::Current(function) => {
                let decl = self.ast.fn_decl(function);
                self.ast
                    .params(decl.params)
                    .get(param)
                    .is_some_and(|param| param.default.is_some())
            }
            KnownCallee::Earlier(_) | KnownCallee::Imported(_) => {
                self.default_of(known, param).is_some()
            }
        }
    }

    /// The default of parameter number `param` of `known`, a known callee,
    /// if it has one that is known (see [Calls](#calls)): a function of
    /// this input whose default is in error has none, and fails the check
    /// with that default's error.
    fn default_of(&self, known: KnownCallee, param: usize) -> Option<ParamDefault> {
        match known {
            KnownCallee::Current(function) => self
                .fn_resolutions
                .get(function.index())?
                .defaults
                .get(param)?
                .clone(),
            KnownCallee::Earlier(_) | KnownCallee::Imported(_) => {
                self.known_params(known)?.default(param).cloned()
            }
        }
    }

    /// Whether `callee`, through any parentheses, is the innermost
    /// function's own name for itself: whether the call is a recursive
    /// call through the callee binding.
    fn is_recursive(&self, callee: ExprId) -> bool {
        let mut node = callee;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        match self.expr_vars.get(node.index()) {
            Some(VarRef::Callee) => true,
            // A call of a member of the group being checked is treated as
            // a recursive call, from whichever member.
            Some(&VarRef::Fn(slot)) => self
                .top_fn_in(slot)
                .is_some_and(|top| top.state == BodyState::Checking),
            _ => false,
        }
    }

    /// Reports that the call `call`, whose callee is at `callee_span` and
    /// labelled `function_label`, has more positional arguments, among
    /// `args`, than its callee's `count` parameters, labelling each one
    /// beyond them (the trailing block counting as the last).
    fn report_extra_arguments(
        &mut self,
        (call, callee_span, function_label): (&Expr, Span, &str),
        args: ArgRange,
        count: usize,
    ) {
        let ast = self.ast;
        let positional = ast
            .args(args)
            .iter()
            .filter(|arg| !matches!(arg.kind, ArgKind::Named { .. }));
        let found = positional.clone().count();
        let noun = if count == 1 { "argument" } else { "arguments" };
        let mut error =
            Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                .with_context(callee_span, function_label);
        for extra in positional.skip(count) {
            error = error.with_context(extra.span(ast), "unexpected argument");
        }
        self.errors.push(error);
    }

    /// Reports the argument written at `second` for parameter number
    /// `param`, named `name` if it has one, which the argument at `first`
    /// already fills.
    fn report_given_twice(
        &mut self,
        (param, name): (usize, Option<Name>),
        first: Span,
        second: Span,
    ) {
        let shown = match name {
            Some(name) => format!("`{}`", self.ast.names().get(name)),
            None => (param + 1).to_string(),
        };
        self.errors.push(
            Diagnostic::error(format!("argument {shown} is given twice"), second)
                .with_context(first, "first given here"),
        );
    }

    /// Reports the named argument `name`, written at `span`, the shorthand
    /// `name` alone if `shorthand`, that no parameter of the callee, of type
    /// `callee_ty` with signature `sig`, is named: an unknown argument,
    /// suggesting the nearest of the named parameters that `filled` (by
    /// position) shows no argument fills ([`unknown_argument`]), or, if none
    /// of its parameters has a name, a function that takes none.
    fn report_unknown_argument(
        &mut self,
        (name, span, shorthand): (Name, Span, bool),
        (callee, callee_ty): (ExprId, TyId),
        sig: &FnTy,
        filled: &[bool],
    ) {
        let callee_span = self.ast.get(callee).span;
        let shown = self.in_label(callee_ty);
        let error = if !sig.params.is_empty() && sig.params.iter().all(|(name, _)| name.is_none()) {
            Diagnostic::error("the parameters of this function have no names", span)
                .with_context(callee_span, format!("this is {shown}"))
                .with_note("call it with positional arguments")
        } else {
            let names = self.ast.names();
            let text = names.get(name);
            let free = sig
                .params
                .iter()
                .zip(filled)
                .filter(|&(_, &filled)| !filled)
                .filter_map(|(&(param, _), _)| Some((names.get(param?), 0)));
            unknown_argument((text, span, shorthand), free)
                .with_context(callee_span, format!("the function is {shown}"))
        };
        self.errors.push(error);
    }

    /// Reports the parameters of the call `call`'s callee, at
    /// `callee_span` and labelled `function_label`, with signature `sig`,
    /// that `filled[base..]` shows no argument fills. A call with only
    /// positional arguments, of which it has `found`, is short of arguments
    /// as a whole; one with named arguments or a trailing block is missing
    /// each unfilled parameter. Returns whether any was unfilled.
    fn report_unfilled(
        &mut self,
        base: usize,
        (call, callee_span, function_label): (&Expr, Span, &str),
        sig: &FnTy,
        (found, named, trailing): (usize, bool, bool),
    ) -> bool {
        let filled = self.filled.get(base..).unwrap_or_default();
        if filled.iter().all(|&arg| arg != u32::MAX) {
            return false;
        }
        let count = sig.params.len();
        if !named && !trailing {
            let noun = if count == 1 { "argument" } else { "arguments" };
            self.errors.push(
                Diagnostic::error(format!("expected {count} {noun}, found {found}"), call.span)
                    .with_context(callee_span, function_label),
            );
            return true;
        }
        let close = call.close_span().unwrap_or(call.span);
        let unfilled: Vec<usize> = filled
            .iter()
            .enumerate()
            .filter(|&(_, &arg)| arg == u32::MAX)
            .map(|(param, _)| param)
            .collect();
        for param in unfilled {
            let message = match sig.params.get(param) {
                Some(&(Some(name), _)) => {
                    format!("missing argument `{}`", self.ast.names().get(name))
                }
                Some(&(None, ty)) => {
                    format!(
                        "missing argument {} of type {}",
                        param + 1,
                        self.in_message(ty)
                    )
                }
                None => continue,
            };
            self.errors
                .push(Diagnostic::error(message, close).with_context(callee_span, function_label));
        }
        true
    }

    /// If the parameter number `param` of an impure or memoised function,
    /// whose type is written as `ty` and is not erroneous, is a callback
    /// parameter, one written directly as an impure function type, adds it
    /// to `callbacks`, with its positions, and returns its index there;
    /// `None` otherwise. A position written bare, as a name, a record type
    /// or a list type (see [`Checker::is_bare`]), is narrowable, to its
    /// present type: the type it is written as with every bare type in it
    /// present. See the module documentation.
    fn callback_parameter(&mut self, param: usize, ty: TypeExprId) -> Option<usize> {
        let ast = self.ast;
        let TypeExprKind::Fn {
            params,
            effect: Effect::Impure,
            ..
        } = ast.type_expr(ty).kind
        else {
            return None;
        };
        let positions = self.callback_positions.len();
        for position in ast.param_tys(params) {
            // The type resolved without an error in its own mode, so it
            // resolves without one in this one too.
            let narrowable = self
                .is_bare(position.ty)
                .then(|| {
                    (
                        position.ty,
                        self.resolve_type(position.ty, Bareness::Present),
                    )
                })
                .filter(|&(_, present)| present != TyId::ERROR);
            self.callback_positions.push(CallbackPosition {
                narrowable,
                given: None,
            });
        }
        self.callbacks.push(Callback {
            param,
            used: None,
            positions,
            count: self.callback_positions.len() - positions,
        });
        Some(self.callbacks.len() - 1)
    }

    /// Checks the body of the function expression `decl`, whose parameters'
    /// types are `fn_params[base..]`, against its return type `ret`, in a
    /// scope of its own. The scope starts with the callee binding, if the
    /// function has a `callee` name (with the function's type and span),
    /// then the parameters, each in the next slot, a callback parameter of
    /// an impure or memoised function recording its calls and uses (see
    /// [`Checker::callback_parameter`]); then the items are
    /// checked, and the body's value against `ret` as an annotated
    /// initialiser's is against its annotation, a record block against a
    /// record return type field by field, and a record literal the body
    /// results in against a record return type with defaults, for the
    /// defaults alone ([`Expecting::literal_only`]).
    fn check_fn_body(
        &mut self,
        decl: &FnDecl,
        base: usize,
        (callee, fn_ty, fn_span): (Option<Name>, TyId, Span),
        ret: TyId,
    ) {
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let outer_spreads = std::mem::take(&mut self.spreads);
        let nodes_start = self.nodes.len();
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        if let Some(name) = callee {
            let binding = Binding {
                name,
                slot: Slot::NONE,
                ty: fn_ty,
                declared: Some(fn_span),
                kind: BindingKind::Callee,
                function: None,
                callback: None,
            };
            self.record_decl(binding);
            self.bindings.push(binding);
        }
        let params = self.ast.params(decl.params);
        for (index, param) in params.iter().enumerate() {
            if let Some(first) = params[..index]
                .iter()
                .find(|first| first.name == param.name)
            {
                let text = self.ast.names().get(param.name);
                self.errors.push(
                    Diagnostic::error(format!("parameter `{text}` is declared twice"), param.span)
                        .with_context(first.span, "first declared here"),
                );
            }
            let ty = self
                .fn_params
                .get(base + index)
                .map_or(TyId::ERROR, |&(_, ty)| ty);
            self.declare_binding(param.name, ty, Some(param.span), BindingKind::Variable);
            if decl.effect != Effect::Pure && ty != TyId::ERROR {
                let callback = self.callback_parameter(index, param.ty);
                if let Some(binding) = self.bindings.last_mut() {
                    binding.callback = callback;
                }
            }
        }
        let body = decl.body;
        let block = *self.ast.get(body);
        let context = decl
            .ret
            .map_or(block.span, |ret| self.ast.type_expr(ret).span);
        let why = RESULT_WHY;
        let ExprKind::Block { items, label } = block.kind else {
            // The parser only makes blocks bodies.
            self.errors.push(
                InternalError {
                    message: "a function body is not a block",
                    span: block.span,
                }
                .into(),
            );
            self.bindings.truncate(self.block_start);
            (self.block_start, self.fields_start, self.next_slot) = outer;
            self.spreads = outer_spreads;
            return;
        };
        // As for any record block, against the present member of the
        // either shape of a record type too.
        let record = self.types.strip(ret);
        if self.types.laziness(ret) != Laziness::Future
            && self.types.is_record(record)
            && self.is_record_body(items)
        {
            let expecting = Expecting {
                ty: record,
                context,
                why,
                close: Span::new(block.span.end.saturating_sub(1), block.span.end),
                fill: Some(body),
                reach: Reach::Whole,
            };
            let found = self.check_body(items, Some(expecting));
            self.expr_types[body.index()] = found;
        } else if let Some(found) = self.empty_body(body, ret, (context, why)) {
            self.expr_types[body.index()] = found;
        } else {
            let expecting = self.result_expecting(ret, items, (context, why), block.span);
            // A labelled body is a labelled block like any other, whose
            // `return`s give it its value, checked as ever where only a
            // record literal its result is is reached.
            let returns = expecting.filter(|expecting| expecting.reach == Reach::Whole);
            let label = label.map(|label| self.ast.label(label));
            if let Some(label) = label {
                self.enter_block(body, label, returns);
            }
            let mut found = self.check_body(items, expecting);
            if label.is_some() {
                found = self.leave_block(body, found, returns);
            }
            self.expr_types[body.index()] = found;
            self.expect(body, found, ret, context, why);
        }
        self.bindings.truncate(self.block_start);
        self.fields.truncate(self.fields_start);
        self.nodes.truncate(nodes_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        self.spreads = outer_spreads;
    }

    /// What is expected of the result of a function's body whose items are
    /// `items`, spanning `span`, which is not a record block, against the
    /// return type `ret`, stated at `context`, labelled `why`: if it has a
    /// result, a list literal it results in is checked against a list
    /// return type or a union, as a record block is against a record one
    /// (see [`Checker::check_list_expected`]); and a record literal it
    /// results in against a record return type with defaults, for the
    /// defaults alone ([`Expecting::literal_only`]). `None` otherwise.
    fn result_expecting<'w>(
        &self,
        ret: TyId,
        items: ItemRange,
        (context, why): (Span, &'w str),
        span: Span,
    ) -> Option<Expecting<'w>> {
        if !self.has_result(items) {
            None
        } else if self.expects_list(ret) {
            Some(Expecting::new(ret, (context, why), span))
        } else {
            self.defaulted_record(ret)
                .map(|record| Expecting::literal_only(record, (context, why), span))
        }
    }

    /// Types the field access `id`, reading field `name` of `record`, of
    /// type `record_ty`: the field's type, as the record type declares it,
    /// or for a union of record types each with the field, the join of
    /// theirs (see [`Checker::field_of`]). A record that may be a future is
    /// awaited at the top first. Reading a field the record type lacks, or
    /// reading any field of a value that is not a record, or of a union not
    /// every member of which has the field, is reported at the `.NAME`, a
    /// union's with the note saying to tell its members apart first.
    fn field_access(
        &mut self,
        id: ExprId,
        (record, record_ty): (ExprId, TyId),
        name: Name,
    ) -> TyId {
        let record_ty = self.measure(record, record_ty);
        // A record of type `Never` is never there to read.
        if record_ty == TyId::ERROR || record_ty == self.never {
            return record_ty;
        }
        // A field of a function with a meta is the meta's.
        if let Some(meta) = self.types.fn_sig(record_ty).and_then(|sig| sig.meta) {
            return self.meta_read(id, record, meta, name);
        }
        if let Some(ty) = self.field_of(record_ty, name) {
            return ty;
        }
        let expr = self.ast.get(id);
        let member = expr.member_span(self.ast).unwrap_or(expr.span);
        let text = FieldName(self.ast.names().get(name));
        if let Some(fields) = self.types.record_fields(record_ty) {
            let rest = self.types.record_rest(record_ty).is_some();
            // A record type with a rest-field and no named field has fields;
            // it names none.
            let note = (!(rest && fields.is_empty())).then(|| self.field_list(fields, rest));
            let suggestion = self.field_suggestion(id, name, fields);
            let message = format!("no field `{text}` in {}", self.in_message(record_ty));
            let mut error = Diagnostic::error(message, member).with_suggestion(suggestion);
            if let Some(note) = note {
                error = error.with_note(note);
            }
            if rest {
                error = error.with_note(REST_READ_NOTE);
            }
            // A path of the view's names nothing then: its field may be
            // declared in source that could not be parsed.
            self.unresolved(error);
            return TyId::ERROR;
        }
        let record_ty = self.settle(record, record_ty);
        let message = format!(
            "cannot access field `{text}` of {}",
            self.in_message(record_ty)
        );
        let label = format!("this is {}", self.in_label(record_ty));
        let mut error =
            Diagnostic::error(message, member).with_context(self.ast.get(record).span, label);
        if let Some(note) = self.union_note(record, record_ty) {
            error = error.with_note(note);
        }
        self.errors.push(error);
        TyId::ERROR
    }

    /// Types the field access `id`, `func.name`, which reads the meta `meta`
    /// of `func`, the function `record` evaluates to (see [Metas](#metas)):
    /// the type of the meta's field `name`. Reading it is calling the meta's
    /// closure, of the meta's effect: from a pure function, a `META->` or
    /// `META->>` meta is "cannot read an impure meta from a pure function",
    /// labelled "reading the meta of `func` is impure (`META->`)" at `func`
    /// and at the function's marker, as calling an impure function is. A
    /// meta that may be a future is awaited at the top, subject to the
    /// purity rule, as a record a field access reads is, unless the read is
    /// already that error. A field the meta lacks is "the meta of `func` has
    /// no field `x`", at the `.NAME`, noted with the meta's fields. Recorded
    /// for lowering ([`Resolution::meta_read`]). A def-type reference, read
    /// in parentheses or by a path past its definition, is named as it is
    /// written, "the meta of `package dev.peios.testpkg`", and a field
    /// missing past a definition is noted as read from its meta
    /// (`Checker::meta_of`).
    fn meta_read(&mut self, id: ExprId, record: ExprId, meta: Meta, name: Name) -> TyId {
        let MetaOf {
            subject,
            func: func_span,
            read: read_span,
            past,
        } = self.meta_of(id, record);
        let mut impure = false;
        if meta.effect != Effect::Pure
            && let Some(function) = self.functions.last()
            && function.effect == Effect::Pure
        {
            impure = true;
            let marker = function.marker;
            let label = format!(
                "reading {subject} is impure (`{}`)",
                meta.effect.meta_marker()
            );
            self.errors.push(
                Diagnostic::error("cannot read an impure meta from a pure function", read_span)
                    .with_context(func_span, label)
                    .with_context(marker, PURE_CONTEXT)
                    .with_note(
                        "a pure function, declared with `:`, may read only pure metas, `META:`; \
                         declare it with `->` to make it impure",
                    ),
            );
        }
        let mut mark = AwaitMark::None;
        let mut ty = meta.ty;
        if self.types.laziness(ty) != Laziness::Present {
            mark = AwaitMark::Top;
            if !impure && let Some(marker) = self.pure_marker() {
                let error = self.implicit_await_error(read_span, ty, marker, None);
                self.errors.push(error);
            }
            ty = self.types.strip(ty);
        }
        self.meta_reads.insert(id, (meta.ty, mark));
        if let Some(field) = self.types.field(ty, name) {
            return field;
        }
        let expr = self.ast.get(id);
        let member = expr.member_span(self.ast).unwrap_or(expr.span);
        let text = FieldName(self.ast.names().get(name));
        let fields = self.types.record_fields(ty);
        let rest = self.types.record_rest(ty).is_some();
        let note = fields.map_or_else(String::new, |fields| self.field_list(fields, rest));
        let suggestion = fields.and_then(|fields| self.field_suggestion(id, name, fields));
        let mut error = Diagnostic::error(format!("{subject} has no field `{text}`"), member)
            .with_note(note)
            .with_suggestion(suggestion);
        // A path past a definition that may have been meant as a longer
        // definition's.
        if let Some(reference) = past {
            error = error.with_note(format!(
                "`{reference}.{text}` is read as the field `{text}` of the meta of \
                 `{reference}`, its longest prefix that is a definition"
            ));
        }
        self.errors.push(error);
        TyId::ERROR
    }

    /// What the field access `id` reads the meta of, the function `record`
    /// evaluates to, as a meta read's errors show it (see [Metas](#metas)):
    /// "the meta of `func`" for a variable, "the meta of `package
    /// dev.peios.testpkg`" for a def-type reference, in parentheses or read
    /// by a path past its definition ([`Checker::reading_past`]), and "the
    /// meta of this function" for anything else; the span of the function
    /// and of the read, which for a path past a definition start at the
    /// reference's def-type; and for such a path, the reference to the
    /// definition, as written, for the error of a missing field's note.
    fn meta_of(&self, id: ExprId, record: ExprId) -> MetaOf {
        let ast = self.ast;
        if let Some((defined, def)) = self.reading_past
            && defined == record
        {
            let start = ast.get(ast.def_type(def).dt).span;
            let (shown, _) = ast.show_def_type(def);
            let mut path = Vec::new();
            let mut at = defined;
            while let ExprKind::Field { expr, name } = ast.get(at).kind {
                path.push(ast.names().get(name));
                at = expr;
            }
            path.reverse();
            let reference = format!("{shown} {}", path.join("."));
            return MetaOf {
                subject: format!("the meta of `{reference}`"),
                func: Span::join(start, ast.get(defined).span),
                read: Span::join(start, ast.get(id).span),
                past: Some(reference),
            };
        }
        let mut inner = record;
        while let ExprKind::Paren(expr) = ast.get(inner).kind {
            inner = expr;
        }
        let subject = match (ast.get(record).kind, ast.get(inner).kind) {
            (ExprKind::Name(func), _) => format!("the meta of `{}`", ast.names().get(func)),
            (_, ExprKind::DefTypeRef(def)) => {
                let (shown, path) = ast.show_def_type(def);
                format!("the meta of `{shown} {path}`")
            }
            _ => "the meta of this function".to_owned(),
        };
        MetaOf {
            subject,
            func: ast.get(record).span,
            read: ast.get(id).span,
            past: None,
        }
    }

    /// The suggestion for the field `name` that the field access `id` reads
    /// and that its record, of the record type with `fields`, lacks (see
    /// [Suggestions](#suggestions)): the field nearest it, written as a
    /// field's name is, in place of the name as it is written, plain or
    /// quoted.
    fn field_suggestion(
        &self,
        id: ExprId,
        name: Name,
        fields: &[(Name, TyId)],
    ) -> Option<(Span, String)> {
        let expr = self.ast.get(id);
        if !matches!(expr.kind, ExprKind::Field { .. }) {
            return None;
        }
        let names = self.ast.names();
        let text = names.get(name);
        let candidates = fields
            .iter()
            .map(|&(field, _)| names.get(field))
            .filter(|field| !crate::names::is_namespace_key(field))
            .map(|field| (field, 0));
        let best = suggest::best(text, candidates)?;
        let span = self.ast.written_name(expr.span.end, text);
        Some((span, FieldName(best).to_string()))
    }

    /// The note listing a record type's `fields`, in alphabetical order:
    /// "the fields are `x` and `y`", or for a record type with a
    /// rest-field, whose other fields it does not name, "the named fields
    /// are `x` and `y`".
    fn field_list(&self, fields: &[(Name, TyId)], rest: bool) -> String {
        let names = self.ast.names();
        let mut texts: Vec<&str> = fields.iter().map(|&(name, _)| names.get(name)).collect();
        texts.sort_unstable();
        let which = if rest { "named " } else { "" };
        match texts.as_slice() {
            [] => format!("the record has no {which}fields"),
            [only] => format!("the only {which}field is `{}`", FieldName(only)),
            [init @ .., last] => {
                let init: Vec<String> = init
                    .iter()
                    .map(|text| format!("`{}`", FieldName(text)))
                    .collect();
                format!(
                    "the {which}fields are {} and `{}`",
                    init.join(", "),
                    FieldName(last)
                )
            }
        }
    }

    /// Types an `if` whose keyword is at `keyword`: its condition must be
    /// `Bool`, and its branches agree (see [`Checker::branches`]) or, without
    /// `else`, its block is `{}`. A condition that narrows a variable gives
    /// each branch its binding (see [`Checker::if_condition`]). Each branch
    /// is checked as `branch` says: bottom-up, or, with `else`, where a
    /// record type with defaults is expected of the `if` itself, for a
    /// record literal it results in alone ([`Checker::fill_result`]), or of
    /// the elements of the list the `if` is, for those of a list literal it
    /// results in alone ([`Checker::fill_elements`]), the branches agreeing
    /// as ever.
    fn check_if(
        &mut self,
        keyword: Span,
        cond: ExprId,
        (then, else_): (ExprId, ExprId),
        (has_else, branch): (bool, Branch<'_>),
    ) -> TyId {
        let narrowing = self.if_condition(keyword, cond);
        let then_ty = self.check_branch(then, narrowing.then, branch);
        let else_ty = self.check_branch(else_, narrowing.else_, branch);
        if let Some(next_slot) = narrowing.next_slot {
            self.next_slot = next_slot;
        }
        // A condition of type `Never` never picks a branch.
        if self.expr_types[cond.index()] == self.never {
            self.settle(then, then_ty);
            self.settle(else_, else_ty);
            return self.never;
        }
        if has_else {
            self.branches((then, then_ty), (else_, else_ty))
        } else {
            self.empty_branch(keyword, then, then_ty)
        }
    }

    /// Checks the condition `cond` of the `if` whose keyword is at
    /// `keyword`, which must be `Bool`, and returns the bindings it gives
    /// the branches. A condition that is, through any parentheses, `NAME is
    /// TYPE`, NAME being, through any parentheses, a variable, a field, a
    /// parameter or a `for`'s variable in scope, narrows it (see the module
    /// documentation): the test's value is stored in a slot of its own,
    /// taken here, and the `then` branch has a binding of NAME there of the
    /// narrowed type, and the `else` branch one of the remainder, unless the
    /// test always passes, or the remainder is the whole of the value's
    /// type and the test awaits nothing. Any other condition narrows
    /// nothing.
    fn if_condition(&mut self, keyword: Span, cond: ExprId) -> Narrowing {
        let Some((test, operand, ty, name)) = self.narrowing_test(cond) else {
            let cond_ty = self.check_expr(cond);
            self.expect(
                cond,
                cond_ty,
                TyId::BOOL,
                keyword,
                "`if` conditions are Bool",
            );
            return Narrowing::default();
        };
        let tested = self.check_test(test, operand, ty, "is");
        let cond_ty = match tested {
            Some(tested) if tested.unreached => self.never,
            Some(_) => TyId::BOOL,
            None => TyId::ERROR,
        };
        // A test of a value of type `Never` is never run, and narrows
        // nothing.
        let tested = tested.filter(|tested| !tested.unreached);
        let mut node = cond;
        loop {
            self.expr_types[node.index()] = cond_ty;
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                _ => break,
            }
        }
        let Some(tested) = tested else {
            return Narrowing::default();
        };
        let next_slot = self.next_slot;
        let slot = Slot(next_slot);
        self.next_slot = next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        match self.slot_names.get_mut(slot.index()) {
            Some(entry) => *entry = name,
            None => self.slot_names.push(name),
        }
        if let Some(resolution) = self.tests.get_mut(&test) {
            resolution.slot = slot;
        }
        // Declared by the test, where a rebind of it points.
        let binding = |ty: TyId| Binding {
            name,
            slot,
            ty,
            declared: Some(self.ast.get(test).span),
            kind: BindingKind::Narrowed,
            function: None,
            callback: None,
        };
        let else_ = match tested.remainder {
            Some(remainder) if !tested.whole || tested.awaited => Some(binding(remainder)),
            _ => None,
        };
        Narrowing {
            then: Some(binding(tested.narrowed)),
            else_,
            next_slot: Some(next_slot),
        }
    }

    /// If `cond`, an `if`'s condition, narrows a variable (see
    /// [`Checker::if_condition`]): the `is` expression, its operand, its
    /// type and the variable's name. `None` for every other condition.
    fn narrowing_test(&self, cond: ExprId) -> Option<(ExprId, ExprId, TypeExprId, Name)> {
        let mut test = cond;
        while let ExprKind::Paren(inner) = self.ast.get(test).kind {
            test = inner;
        }
        let ExprKind::Is { expr: operand, ty } = self.ast.get(test).kind else {
            return None;
        };
        let mut node = operand;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let ExprKind::Name(name) = self.ast.get(node).kind else {
            return None;
        };
        let (_, binding) = self.lookup(name)?;
        matches!(
            binding.kind,
            BindingKind::Variable
                | BindingKind::Mutable
                | BindingKind::Header
                | BindingKind::Field
                | BindingKind::Narrowed
        )
        .then_some((test, operand, ty, name))
    }

    /// Checks the branch `branch` of an `if`, with the narrowed binding
    /// `narrowed`, if it has one, bound in a scope around it, as a `for`'s
    /// variable is around its body, so that a `:=` of it in the branch
    /// finds it and is rejected (see [`Checker::check_rebind`]). With
    /// `expected` [`Branch::Fits`], the type the branch is expected to have,
    /// with where that is stated and its label, it is checked against it
    /// and has what it is once any await marked on it has run (see
    /// [`Checker::fit`]); with [`Branch::Fills`], a record type with
    /// defaults reaches only a record literal it results in (see
    /// [`Checker::fill_result`]); with [`Branch::FillsElements`], only the
    /// elements of a list literal it results in, or the body of a `for` it
    /// does (see [`Checker::fill_elements`]); otherwise it is checked
    /// bottom-up.
    fn check_branch(
        &mut self,
        branch: ExprId,
        narrowed: Option<Binding>,
        expected: Branch<'_>,
    ) -> TyId {
        let Some(narrowed) = narrowed else {
            return self.branch_type(branch, expected);
        };
        let start = self.bindings.len();
        self.bind(narrowed);
        let ty = self.branch_type(branch, expected);
        self.bindings.truncate(start);
        ty
    }

    /// Checks the branch `branch` of an `if` as [`Checker::check_branch`]
    /// says, its narrowed binding, if any, being in place already.
    fn branch_type(&mut self, branch: ExprId, expected: Branch<'_>) -> TyId {
        match expected {
            Branch::Fits(expected, context) => self.fit(branch, expected, context),
            Branch::Fills(record, context) => self.fill_result(branch, record, context),
            Branch::FillsElements(record, context) => self.fill_elements(branch, record, context),
            Branch::Free => self.check_expr(branch),
        }
    }

    /// Types the `is` or `as`, as `keyword` says, expression `id`, testing
    /// whether `operand` is of the type written `ty` (see the module
    /// documentation): the type is resolved with every bare type present,
    /// and may contain no future and no function type; the operand is
    /// checked, an untyped integer settling to `Int`, and measured at the
    /// top, or deeply if the test's plan walks into it (see
    /// [`TypeTable::test_plan`]), subject to the purity rule. Each member of
    /// the operand's type once measured, made present, *always passes* if
    /// it is a subtype of the type tested, *may pass* if it does not but
    /// overlaps it (see [`TypeTable::overlaps`]), and *cannot pass*
    /// otherwise. If every member always passes, the test is `true`, which
    /// is a warning; if none passes or may, it is the error "`x` is never a
    /// String", at the type. Records the plan for lowering (see
    /// [`Resolution::test_of`]). `None` if the type, the operand or the
    /// test is erroneous.
    fn check_test(
        &mut self,
        id: ExprId,
        operand: ExprId,
        ty: TypeExprId,
        keyword: &str,
    ) -> Option<Tested> {
        let target = self.resolve_type(ty, Bareness::Present);
        let found = self.check_expr(operand);
        let found = self.settle(operand, found);
        if target == TyId::ERROR || found == TyId::ERROR || !self.testable(target, ty, keyword) {
            return None;
        }
        // An operand of type `Never` is never there to test.
        if found == self.never {
            return Some(Tested {
                narrowed: self.never,
                remainder: None,
                whole: false,
                awaited: false,
                unreached: true,
            });
        }
        let present = self.types.present(found);
        let present_members = self.flat_members(present);
        let plan = self.types.test_plan(Some(&present_members), target);
        let awaits = self.awaits[operand.index()];
        let used = if plan.walks() {
            self.measure_deep(operand, found)
        } else {
            self.measure(operand, found)
        };
        let awaited = self.awaits[operand.index()] != awaits;
        let (mut always, mut rest, mut may) = (Vec::new(), Vec::new(), false);
        for member in self.flat_members(used) {
            let shown = self.types.present(member);
            if self.types.is_subtype(shown, target) {
                always.push(member);
            } else {
                may |= self.types.overlaps(shown, target);
                rest.push(member);
            }
        }
        if always.is_empty() && !may {
            self.never_error((operand, ty), found, target);
            return None;
        }
        let (all, whole) = (rest.is_empty(), always.is_empty());
        if all {
            self.always_warning(id, operand, target, keyword);
        }
        if may {
            always.push(target);
        }
        let narrowed = self.types.union(always);
        let remainder = (!all).then(|| self.types.union(rest));
        let resolution = TestResolution {
            test: if all { ValueTest::Pass } else { plan },
            slot: Slot::NONE,
            subject: self.subject(operand).into(),
            target: self.with_article(target).into(),
        };
        self.tests.insert(id, resolution);
        Some(Tested {
            narrowed,
            remainder,
            whole,
            awaited,
            unreached: false,
        })
    }

    /// Whether `target`, the type written `ty` that the `is` or `as`, as
    /// `keyword` says, tests for, can be tested: it holds no future, nor an
    /// either shape, which "`is` cannot test a future" reports at the type,
    /// and no function type, which "`is` cannot test a function type"
    /// reports, since a function's signature is not something a value can
    /// be asked for.
    fn testable(&mut self, target: TyId, ty: TypeExprId, keyword: &str) -> bool {
        let span = self.ast.type_expr(ty).span;
        if self.types.present(target) != target {
            self.errors.push(
                Diagnostic::error(format!("`{keyword}` cannot test a future"), span)
                    .with_note("await the value and test what it yields"),
            );
            return false;
        }
        if self.types.contains_fn(target) {
            self.errors.push(
                Diagnostic::error(format!("`{keyword}` cannot test a function type"), span)
                    .with_note(
                        "a function's signature is not something a value can be asked for; tell \
                         a function apart by the other members, in the `else` branch",
                    ),
            );
            return false;
        }
        true
    }

    /// Reports that no member of `found`, the type of `operand`, the
    /// operand of an `is` or `as`, may be a `target`, the type written
    /// `ty`: "`x` is never a String", at the type, labelled with the
    /// operand's type at the operand.
    fn never_error(&mut self, (operand, ty): (ExprId, TypeExprId), found: TyId, target: TyId) {
        let message = format!(
            "{} is never {}",
            self.subject(operand),
            self.with_article(target)
        );
        let label = format!("this is {}", self.in_label(found));
        self.errors.push(
            Diagnostic::error(message, self.ast.type_expr(ty).span)
                .with_context(self.ast.get(operand).span, label),
        );
    }

    /// Warns that every member of the operand's type of the `is` or `as`,
    /// as `keyword` says, expression `id` is a `target`: for `is`, "`x` is
    /// always an Int", since the test is `true`; for `as`, "`as` changes
    /// nothing here: `x` is already an Int", with a note that an ascription
    /// says the same without a test.
    fn always_warning(&mut self, id: ExprId, operand: ExprId, target: TyId, keyword: &str) {
        let (subject, target_text) = (self.subject(operand), self.with_article(target));
        let span = self.ast.get(id).span;
        let warning = if keyword == "as" {
            let written = self.types.display(target, self.ast.names()).to_string();
            Diagnostic::warning(
                format!("`as` changes nothing here: {subject} is already {target_text}"),
                span,
            )
            .with_note(format!("`: {written}` says the same without a test"))
        } else {
            Diagnostic::warning(format!("{subject} is always {target_text}"), span)
        };
        self.warnings.push(warning);
    }

    /// The operand `operand` of an `is` or `as` as its messages name it:
    /// as written, in backticks, if it is a plain name or a dotted path, as
    /// in "`x`", and "this value" otherwise.
    fn subject(&self, operand: ExprId) -> String {
        let mut node = operand;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        match self.path_text(node) {
            Some(text) => format!("`{text}`"),
            None => "this value".to_owned(),
        }
    }

    /// Type `ty` as a message shows it (see [`Checker::in_message`]), after
    /// its indefinite article: "an Int", "a String", "a `List<Int>`", "an
    /// `Int | String`".
    fn with_article(&mut self, ty: TyId) -> String {
        let shown = self.in_message(ty);
        let first = shown
            .chars()
            .find(char::is_ascii_alphanumeric)
            .unwrap_or('x');
        let starts_type = shown.trim_start_matches('`').starts_with(first);
        let article = if starts_type && "AEIOUaeiou".contains(first) {
            "an"
        } else {
            "a"
        };
        format!("{article} {shown}")
    }

    /// The members of `ty` if it is a union, and `ty` alone otherwise.
    fn flat_members(&self, ty: TyId) -> Vec<TyId> {
        match self.types.union_members(ty) {
            Some(members) => members.to_vec(),
            None => vec![ty],
        }
    }

    /// Types the prefix operator `op`, at `op_span`, applied to `operand`
    /// of type `operand_ty`. Negation accepts every numeric type and
    /// preserves it; `not` needs a `Bool`; both await an operand that may be
    /// a future first. `await` awaits its operand fully, and its type is
    /// the operand's stripped of every future layer at its top; on an
    /// operand that is present it does nothing.
    fn unary(&mut self, op: UnaryOp, op_span: Span, (operand, operand_ty): (ExprId, TyId)) -> TyId {
        // An operand of type `Never` never gives the operator a value.
        if operand_ty == TyId::ERROR || operand_ty == self.never {
            return operand_ty;
        }
        if op == UnaryOp::Await {
            return self.explicit_await(op_span, operand, operand_ty);
        }
        let operand_ty = self.measure(operand, operand_ty);
        match op {
            UnaryOp::Neg if is_numeric(operand_ty) || is_quantity(operand_ty) => operand_ty,
            UnaryOp::Neg => {
                let message = format!("cannot {} {}", op.verb(), self.in_message(operand_ty));
                let label = self.in_label(operand_ty);
                let mut error = Diagnostic::error(message, op_span)
                    .with_context(self.ast.get(operand).span, label);
                if let Some(note) = self.union_note(operand, operand_ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
                TyId::ERROR
            }
            UnaryOp::Not => {
                self.logic_operand(op.verb(), op_span, operand, operand_ty);
                TyId::BOOL
            }
            // Handled above.
            UnaryOp::Await => operand_ty,
        }
    }

    /// Types `await OPERAND`, whose keyword is at `keyword`, `operand` being
    /// of type `ty`, which is not erroneous (see [`Checker::unary`]).
    fn explicit_await(&mut self, keyword: Span, operand: ExprId, ty: TyId) -> TyId {
        // Whether a value is present may be inferred, as a call's result is
        // from its callee's body, so awaiting one is not worth a warning.
        if self.types.laziness(ty) == Laziness::Present {
            return ty;
        }
        let claimed = self.claim_top(self.ast.get(operand).span, ty);
        if claimed != ty {
            // A parameter of this function, which is pure (only a pure
            // function's parameters are provisional), so the `await` is an
            // error before anything else; the parameter is measured.
            if let Some(marker) = self.pure_marker() {
                self.errors.push(explicit_await_error(keyword, marker));
            }
            return claimed;
        }
        self.await_here(operand, ty, AwaitKind::Top, AwaitSite::Explicit(keyword));
        self.types.strip(ty)
    }

    /// Resolves a use of the variable `name` at expression `id`, recording
    /// where it reads from, capturing it if it belongs to an enclosing
    /// function, and returns its type. A use of a callback parameter is
    /// recorded as a use of it as a value, which keeps it as written (see
    /// the module documentation); a call through its name is checked by
    /// [`Checker::check_callee`] instead.
    fn check_name(&mut self, id: ExprId, name: Name, span: Span) -> TyId {
        let (ty, callback) = self.use_name(id, name, span);
        if let Some(callback) = callback.and_then(|callback| self.callbacks.get_mut(callback)) {
            // The first use in source order.
            if callback.used.is_none_or(|first| span.start < first.start) {
                callback.used = Some(span);
            }
        }
        ty
    }

    /// Checks the callee `callee` of a call, returning its type and, if it
    /// is exactly a name, not in parentheses, whose binding is a callback
    /// parameter, the parameter's index in `callbacks`: a call through the
    /// name is recorded by what it gives each position of the callback's
    /// type (see [`Checker::record_argument`]), not as a use of it as a
    /// value.
    fn check_callee(&mut self, callee: ExprId) -> (TyId, Option<usize>) {
        let expr = self.ast.get(callee);
        let ExprKind::Name(name) = expr.kind else {
            return (self.check_expr(callee), None);
        };
        let (ty, callback) = self.use_name(callee, name, expr.span);
        self.expr_types[callee.index()] = ty;
        (ty, callback)
    }

    /// Whether `binding`, at `index` among the top level's bindings, the
    /// latest of `name`, is read rather than what the file's view has of
    /// `name` (see [Imports](#imports)): only when what the view has is
    /// shadowable, and the binding is one the clash family lets shadow it, a
    /// top-level `fn` of the file or an earlier session input's variable. A
    /// field root of the file, or one its directory chain committed, is a
    /// declaration of the view's own order, which the view has decided.
    fn binding_shadows_view(
        &self,
        workspace: &Context<'_>,
        (index, binding): (usize, Binding),
        name: Name,
    ) -> bool {
        if !workspace.scope.yields(name) {
            return false;
        }
        if index >= self.top_start {
            binding.kind != BindingKind::Field
        } else {
            workspace.session()
        }
    }

    /// As [`Checker::check_name`], but recording nothing of a callback
    /// parameter: returns the type, and the index in `callbacks` of the
    /// callback parameter the name's binding is, if it is one.
    fn use_name(&mut self, id: ExprId, name: Name, span: Span) -> (TyId, Option<usize>) {
        let found = self.lookup(name);
        if self.early.is_some() && self.reads_between(name, found) {
            return (TyId::ERROR, None);
        }
        if self.reads_poisoned(name, found) {
            return (TyId::ERROR, None);
        }
        // An imported name, or a root several sources of the file's view
        // have, that nothing nearer shadows (see [Imports](#imports)); a
        // shadowable one, the implicit prelude's among them, only if no
        // binding of the file's own or of an earlier input has it.
        if let Some(workspace) = self.workspace
            && workspace.scope.reads_through(name)
            && found.is_none_or(|(index, binding)| {
                index < self.top_end
                    && !self.binding_shadows_view(workspace, (index, binding), name)
            })
            && let Some(ty) = self.field_reference(id)
        {
            return (self.builtin_use(id, ty, span), None);
        }
        if let Some((index, binding)) = found {
            if binding.kind == BindingKind::Mutable {
                self.leaf_reads_mutable(index, binding.name);
            }
            if let Some(record) = &mut self.record
                && let Some(declared) = binding.declared
            {
                record.uses.push((id, declared));
            }
            self.expr_vars[id.index()] = self.resolve(index, binding);
            return (self.builtin_use(id, binding.ty, span), binding.callback);
        }
        let suggestion = self.variable_suggestion(name, span);
        let text = self.ast.names().get(name);
        let mut error = Diagnostic::error(format!("undefined variable `{text}`"), span)
            .with_suggestion(suggestion);
        if text.contains('-') {
            error = error.with_note(
                "names may contain `-`, so `x-1` is one name; write `x - 1` for subtraction",
            );
        }
        // In a default, its own parameter or a later one.
        if let Some(&(_, written)) = self
            .defaults_checked
            .unseen
            .iter()
            .rfind(|&&(unseen, _)| unseen == name)
        {
            error = error
                .with_context(written, format!("`{text}` is a parameter from here on"))
                .with_note("a parameter's default sees only the parameters before it");
        }
        // Inside a function, or a top-level field's initialiser, a name the
        // top level declares further down.
        let field = self.checking_leaf.is_some();
        if (field || !self.functions.is_empty())
            && let Some(below) = self.ast.items(self.ast.root()).iter().find(|&&item| {
                self.ast.item(item).span.start > span.start
                    && root_declares(self.ast, item) == Some(name)
            })
        {
            let (what, note) = if field {
                (
                    "field",
                    "a field's initialiser sees the variables declared above it",
                )
            } else {
                (
                    "function",
                    "a function sees the variables declared above it",
                )
            };
            error = error
                .with_context(
                    self.ast.item(*below).span,
                    format!("`{text}` is declared here, below the {what}"),
                )
                .with_note(note);
        }
        self.unresolved(error);
        (TyId::ERROR, None)
    }

    /// While a top-level function's body is checked before the walk has
    /// reached its declaration (see `early`): whether a read of `name`,
    /// which the lookup found as `found`, is of a name the top level
    /// declares between the item being checked and the function, by an
    /// item or possibly by a spread, if it reads the top level or an
    /// earlier input. If so, it records the first such read as why the
    /// function cannot be used yet, and the read is not resolved.
    fn reads_between(&mut self, name: Name, found: Option<(usize, Binding)>) -> bool {
        let Some((span, kind)) = self.early_blocker(name, found) else {
            return false;
        };
        let blocker = Some(Blocker { name, span, kind });
        if let Some(top) = self.checking_top.and_then(|top| self.top_fns.get_mut(top)) {
            top.blocker = top.blocker.or(blocker);
        } else if let Some(cell) = self
            .checking_leaf
            .and_then(|node| self.field_cells.get_mut(node as usize))
        {
            cell.blocker = cell.blocker.or(blocker);
        }
        true
    }

    /// What keeps a read of `name`, found as `found`, from being resolved
    /// while a top-level function's body is checked early, as
    /// [`Checker::reads_between`] says: the item between that declares it,
    /// or a spread there, with where it is; `None` if nothing does.
    fn early_blocker(
        &self,
        name: Name,
        found: Option<(usize, Binding)>,
    ) -> Option<(Span, BlockerKind)> {
        let (from, to) = self.early?;
        if self.checking_top.is_none() && self.checking_leaf.is_none() {
            return None;
        }
        let inner = self
            .functions
            .first()
            .map_or(usize::MAX, |function| function.bindings_start);
        let top_range = self.top_start..self.top_end;
        match found {
            Some((index, _)) if index >= inner || top_range.contains(&index) => return None,
            _ => {}
        }
        let mutable = found.is_some_and(|(_, binding)| binding.kind == BindingKind::Mutable);
        let root = self.ast.items(self.ast.root());
        let between = root.get(from..to).unwrap_or_default();
        between.iter().find_map(|&item| {
            let declared = self.ast.item(item);
            match declared.kind {
                ItemKind::Spread(_) => Some((declared.span, BlockerKind::Spread)),
                // A `:=` of a mutable variable declares nothing.
                ItemKind::Rebind { .. } if mutable => None,
                _ => (root_declares(self.ast, item) == Some(name))
                    .then_some((declared.span, BlockerKind::Bound)),
            }
        })
    }

    /// The suggestion for the undefined variable `name`, read at `span`
    /// (see [Suggestions](#suggestions)): the name nearest it among the
    /// bindings in scope and the roots of the file's view that a read here
    /// would resolve. None in a file whose view sees one with syntax
    /// errors, where the error is not reported.
    fn variable_suggestion(&mut self, name: Name, span: Span) -> Option<(Span, String)> {
        if self.sees_broken() {
            return None;
        }
        // The top-level field whose initialiser is being checked: a read of
        // it there would be of itself.
        let declaring = self
            .checking_leaf
            .and_then(|node| self.field_cells.get(node as usize))
            .and_then(|cell| cell.binding);
        let innermost = self.block_start;
        let function = self
            .functions
            .last()
            .map_or(usize::MAX, |function| function.bindings_start);
        let mut candidates: Vec<(Name, u8)> = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            if Some(index) == declaring {
                continue;
            }
            let rank = if index >= innermost {
                0
            } else if index >= function || index >= self.top_end {
                1
            } else if index >= self.top_start {
                2
            } else {
                3
            };
            candidates.push((binding.name, rank));
        }
        if let Some(workspace) = self.workspace {
            let roots = self
                .view_roots
                .get_or_insert_with(|| workspace.suggestion_roots());
            candidates.extend(roots.iter().copied());
        }
        let names = self.ast.names();
        let ranked = suggest::ranked(
            names.get(name),
            candidates
                .into_iter()
                .map(|(candidate, rank)| (names.get(candidate), rank, candidate)),
        );
        // The closest that a read here would resolve: looked for among those
        // close enough alone, which are few.
        let best = ranked.into_iter().find(|&candidate| {
            if !crate::lexer::is_identifier(names.get(candidate)) {
                return false;
            }
            let found = self.lookup(candidate);
            if declaring.is_some() && found.map(|(index, _)| index) == declaring {
                return false;
            }
            !self
                .workspace
                .is_some_and(|workspace| workspace.poisoned(candidate))
                && self.early_blocker(candidate, found).is_none()
        })?;
        Some((span, names.get(best).to_owned()))
    }

    /// While a top-level leaf's initialiser is checked (see
    /// `checking_leaf`): records a read of the mutable variable `name`, the
    /// binding `index`, if it is one of this input's top level, as what
    /// makes the leaf be computed at its own item and no earlier (see
    /// [Top-level fields](#top-level-fields)).
    fn leaf_reads_mutable(&mut self, index: usize, name: Name) {
        let Some(node) = self.checking_leaf else {
            return;
        };
        let inner = self
            .functions
            .first()
            .map_or(usize::MAX, |function| function.bindings_start);
        if !(self.top_end..inner).contains(&index) {
            return;
        }
        let Some(leaf) = self
            .mentions
            .tree
            .nodes
            .get(node as usize)
            .and_then(|entry| entry.leaf)
        else {
            return;
        };
        let span = self.ast.item(leaf.item).span;
        if let Some(cell) = self.field_cells.get_mut(node as usize)
            && cell.blocker.is_none()
        {
            cell.ready = leaf.position;
            cell.blocker = Some(Blocker {
                name,
                span,
                kind: BlockerKind::Mutable,
            });
        }
    }

    /// Types `+`, `-` and `*`: both operands must end up the same numeric
    /// type, with untyped operands adapting to a concrete one, except that
    /// `+` also concatenates two strings, and that a quantity is added to and
    /// subtracted from one of its type and multiplied by an `Int` on either
    /// side, giving its type (see [Quantities](#quantities)).
    fn arithmetic(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if op == BinaryOp::Add && lhs_ty == TyId::STRING && rhs_ty == TyId::STRING {
            return TyId::STRING;
        }
        match (op, lhs_ty, rhs_ty) {
            (BinaryOp::Add | BinaryOp::Sub, quantity, other)
                if is_quantity(quantity) && quantity == other =>
            {
                return quantity;
            }
            (BinaryOp::Mul, quantity, count) if is_quantity(quantity) && is_int(count) => {
                self.coerce(rhs, TyId::INT);
                return quantity;
            }
            (BinaryOp::Mul, count, quantity) if is_int(count) && is_quantity(quantity) => {
                self.coerce(lhs, TyId::INT);
                return quantity;
            }
            _ => {}
        }
        if let Some(ty) = self.numeric((lhs, lhs_ty), (rhs, rhs_ty)) {
            return ty;
        }
        let message = format!(
            "cannot {} {} and {}",
            op.verb(),
            self.in_message(lhs_ty),
            self.in_message(rhs_ty)
        );
        self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
        TyId::ERROR
    }

    /// Decides the common numeric type of two expressions that must end up
    /// the same numeric type, as the operands of arithmetic must: untyped
    /// ones adapt to a concrete `Int` or `Float`, and two untyped ones stay
    /// untyped. Returns `Some(TyId::ERROR)` without a report if either is
    /// already an error, and `None` if they cannot agree, having settled any
    /// untyped one to `Int` for the caller's report.
    fn numeric(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        match (lhs_ty, rhs_ty) {
            // An erroneous operand has been reported already. The other
            // operand still needs a concrete type in case it is untyped.
            (TyId::ERROR, _) => {
                self.coerce(rhs, TyId::INT);
                Some(TyId::ERROR)
            }
            (_, TyId::ERROR) => {
                self.coerce(lhs, TyId::INT);
                Some(TyId::ERROR)
            }
            (TyId::UNTYPED_INT, TyId::UNTYPED_INT) => Some(TyId::UNTYPED_INT),
            (TyId::UNTYPED_INT, concrete @ (TyId::INT | TyId::FLOAT)) => {
                self.coerce(lhs, concrete);
                Some(concrete)
            }
            (concrete @ (TyId::INT | TyId::FLOAT), TyId::UNTYPED_INT) => {
                self.coerce(rhs, concrete);
                Some(concrete)
            }
            (TyId::INT, TyId::INT) => Some(TyId::INT),
            (TyId::FLOAT, TyId::FLOAT) => Some(TyId::FLOAT),
            _ => {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                None
            }
        }
    }

    /// Types a comparison. Both operands must be the same type, untyped
    /// operands adapting as for arithmetic and two untyped operands
    /// settling to `Int`: any type without functions in it for `===` and
    /// `!==` (two records, two lists, or two unions, must be exactly the same
    /// type), and any such type but a union for `==` and `!=`, a union being
    /// "cannot compare `Int | String` with `==`", and `Int`, `Float` or
    /// `String` for ordering. The result is `Bool`.
    fn comparison(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        let ordering = !op.is_equality();
        if ordering && lhs_ty != TyId::ERROR && rhs_ty != TyId::ERROR {
            // An operand of a type with no order is reported as such, before
            // any mismatch.
            let unordered = [lhs_ty, rhs_ty].into_iter().find(|&ty| {
                ty == TyId::BOOL
                    || self.types.is_record(ty)
                    || self.types.list_element(ty).is_some()
                    || self.types.fn_sig(ty).is_some()
                    || self.types.union_members(ty).is_some()
            });
            if let Some(unordered) = unordered {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                let message = format!("cannot order {}", self.in_message(unordered));
                self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
                return TyId::ERROR;
            }
        }
        match self.agree((lhs, lhs_ty), (rhs, rhs_ty)) {
            Some(TyId::ERROR) => TyId::ERROR,
            // Two untyped literals compare as Int.
            Some(TyId::UNTYPED_INT) => {
                self.coerce(lhs, TyId::INT);
                self.coerce(rhs, TyId::INT);
                TyId::BOOL
            }
            // Functions have no equality, even inside records.
            Some(ty) if self.types.contains_fn(ty) => {
                let message = if self.types.fn_sig(ty).is_some() {
                    "cannot compare functions".to_owned()
                } else {
                    format!(
                        "cannot compare {}: it contains a function",
                        self.in_message(ty)
                    )
                };
                let expr = self.ast.get(id);
                let span = expr.operator_span().unwrap_or(expr.span);
                let label = format!("this is {}", self.in_label(ty));
                self.errors.push(
                    Diagnostic::error(message, span)
                        .with_context(self.ast.get(lhs).span, label.as_str())
                        .with_context(self.ast.get(rhs).span, label),
                );
                TyId::ERROR
            }
            // Typed equality compares by the static type, which a union's
            // value does not have.
            Some(ty)
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
                    && self.types.union_members(ty).is_some() =>
            {
                let (symbol, total) = if op == BinaryOp::Eq {
                    ("==", "===")
                } else {
                    ("!=", "!==")
                };
                let message = format!("cannot compare {} with `{symbol}`", self.in_message(ty));
                let label = format!("this is {}", self.in_label(ty));
                let expr = self.ast.get(id);
                let span = expr.operator_span().unwrap_or(expr.span);
                self.errors.push(
                    Diagnostic::error(message, span)
                        .with_context(self.ast.get(lhs).span, label.as_str())
                        .with_context(self.ast.get(rhs).span, label)
                        .with_note(format!("narrow it, or compare with `{total}`")),
                );
                TyId::ERROR
            }
            Some(_) => TyId::BOOL,
            None => {
                let message = format!(
                    "cannot {} {} and {}",
                    op.verb(),
                    self.in_message(lhs_ty),
                    self.in_message(rhs_ty)
                );
                self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
                TyId::ERROR
            }
        }
    }

    /// Decides the common type of two expressions that must have the same
    /// type: `Bool`, `String`, `Duration`, `Size`, record, list and function
    /// types and unions must match exactly, and numeric types agree as for
    /// [`Checker::numeric`], whose results this returns.
    fn agree(
        &mut self,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> Option<TyId> {
        // Two types that differ only in their fields' defaults are one (see
        // [Field defaults](#field-defaults)).
        if self.types.same(lhs_ty, rhs_ty)
            && (lhs_ty == TyId::BOOL
                || lhs_ty == TyId::STRING
                || is_quantity(lhs_ty)
                || self.types.is_record(lhs_ty)
                || self.types.list_element(lhs_ty).is_some()
                || self.types.fn_sig(lhs_ty).is_some()
                || self.types.union_members(lhs_ty).is_some())
        {
            Some(lhs_ty)
        } else {
            self.numeric((lhs, lhs_ty), (rhs, rhs_ty))
        }
    }

    /// Checks that `operand`, of type `ty`, of the logic operator `verb` at
    /// `op_span`, is a `Bool`.
    fn logic_operand(&mut self, verb: &str, op_span: Span, operand: ExprId, ty: TyId) {
        let why = format!("`{verb}` requires Bool operands");
        self.expect(operand, ty, TyId::BOOL, op_span, &why);
    }

    /// Types the branches of an `if` with `else`: they have their join (see
    /// [`TypeTable::join`]), an untyped branch first adapting to the other
    /// branch's `Int` or `Float` once stripped, or settling to `Int`, and
    /// two untyped branches leaving the `if` untyped. A join that is widened,
    /// a union of what the branches are where they do not agree, is a
    /// warning (see [`Checker::branches_widened`]).
    ///
    /// Two branches of the same type have it, a provisional type of the
    /// innermost function's included, so a parameter passes through an
    /// `if` unmeasured. Otherwise every parameter of the function whose
    /// provisional type is in either branch is measured, conservatively,
    /// and the branches join with those parameters present.
    fn branches(
        &mut self,
        (then, then_ty): (ExprId, TyId),
        (else_, else_ty): (ExprId, TyId),
    ) -> TyId {
        if then_ty == else_ty {
            return then_ty;
        }
        let (then_ty, else_ty) = (self.claim(then, then_ty), self.claim(else_, else_ty));
        let then_ty = self.adapt_to(then, then_ty, else_ty);
        let else_ty = self.adapt_to(else_, else_ty, then_ty);
        let (joined, widened) = self.types.join(then_ty, else_ty);
        if widened {
            self.branches_widened((then, then_ty), (else_, else_ty), joined);
        }
        joined
    }

    /// Warns that the branches of an `if`, `then` and `else_`, of types
    /// `then_ty` and `else_ty`, have different types, which `joined`, the
    /// `if`'s type, widened to a union of: at the `else` branch's value
    /// (see [`Checker::branch_value`]), labelled with its type, with a label
    /// of the other branch's on its value, and a note naming the union.
    fn branches_widened(
        &mut self,
        (then, then_ty): (ExprId, TyId),
        (else_, else_ty): (ExprId, TyId),
        joined: TyId,
    ) {
        let then_label = format!("this is {}", self.in_label(then_ty));
        let else_label = format!("this is {}", self.in_label(else_ty));
        let note = format!(
            "the `if` is {}; annotate it with that type if both are meant, or tell them apart \
             with `is`",
            self.in_message(joined)
        );
        let (then_at, else_at) = (self.branch_value(then), self.branch_value(else_));
        self.warnings.push(
            Diagnostic::warning(
                "the branches of this `if` have different types",
                self.ast.get(else_at).span,
            )
            .with_label(else_label)
            .with_context(self.ast.get(then_at).span, then_label)
            .with_note(note),
        );
    }

    /// The expression that gives the value of `branch`, an `if`'s branch:
    /// through any parentheses and blocks, each block's result, if it has
    /// one; `branch` itself if it is not a block with a result.
    fn branch_value(&self, branch: ExprId) -> ExprId {
        let mut node = branch;
        loop {
            match self.ast.get(node).kind {
                ExprKind::Paren(inner) => node = inner,
                ExprKind::Block { items: body, .. } if self.has_result(body) => {
                    match self
                        .ast
                        .items(body)
                        .last()
                        .map(|&last| self.ast.item(last).kind)
                    {
                        Some(ItemKind::Result(value) | ItemKind::Expr(value)) => node = value,
                        _ => return node,
                    }
                }
                _ => return node,
            }
        }
    }

    /// The type of `expr`, of type `ty`, once an untyped integer in it
    /// adapts to `other`, the type of the expression it must agree with (see
    /// [`Checker::numeric_target`]). Any other type is returned as it is.
    fn adapt_to(&mut self, expr: ExprId, ty: TyId, other: TyId) -> TyId {
        if ty != TyId::UNTYPED_INT {
            return ty;
        }
        let target = self.numeric_target(other);
        self.coerce(expr, target);
        target
    }

    /// Types an `if` without `else`, whose keyword is at `keyword`: it is
    /// `{}`, so its block must be too, which measures a parameter of the
    /// innermost function whose provisional type is in the block's type.
    fn empty_branch(&mut self, keyword: Span, then: ExprId, then_ty: TyId) -> TyId {
        match self.claim(then, then_ty) {
            TyId::EMPTY => TyId::EMPTY,
            TyId::ERROR => TyId::ERROR,
            // A block that never completes gives no value to drop: the
            // `if` is `{}`, as the `else` it lacks is.
            ty if ty == self.never => TyId::EMPTY,
            _ => {
                let then_ty = self.settle(then, then_ty);
                let label = format!("this branch is {}", self.in_label(then_ty));
                self.errors.push(
                    Diagnostic::error("`if` without `else` must be `{}`", keyword)
                        .with_context(self.ast.get(then).span, label)
                        .with_note("add an `else` branch, or make the block's result `{}`"),
                );
                TyId::ERROR
            }
        }
    }

    /// Types an interpolated string: each interpolated expression is
    /// checked, an untyped one settles to `Int`, and one that may be a
    /// future, or hold one in a field, is awaited deeply. The result is a
    /// `String`, or an error, without a further report, if a part is one.
    fn check_interpolation(&mut self, parts: PartRange) -> TyId {
        let mut ty = TyId::STRING;
        for &part in self.ast.parts(parts) {
            let StrPart::Expr(expr) = part else {
                continue;
            };
            let found = self.check_expr(expr);
            // A value of every type can be shown as text today, a record in
            // its `repr` form. Once some type's values cannot be, it is
            // reported here as "cannot display".
            let found = self.settle(expr, found);
            match self.measure_deep(expr, found) {
                TyId::ERROR => ty = TyId::ERROR,
                // A value of type `Never` is never there to show.
                found if found == self.never && ty != TyId::ERROR => ty = self.never,
                _ => {}
            }
        }
        ty
    }

    /// Types the list literal `id`, whose elements are `elements`, with no
    /// expected type (see the module documentation): each element is
    /// checked, a spread's operand being measured and having to be a list,
    /// and the literal is the list of the join of the elements' types, in
    /// order, a spread contributing its list's element type, untyped
    /// integers adapting as an `if`'s branches do. A join that is widened,
    /// a union of element types that do not agree, is a warning, at the
    /// first element whose type widens the join of those before it (see
    /// [`Checker::elements_widened`]). An empty literal has nothing to join,
    /// "cannot infer the element type of `[]`". `Error` for that, and for a
    /// literal with an erroneous element. With `fill`, a record type with
    /// defaults that the elements are expected to be of, with where that is
    /// stated and its label, each element that is no spread is checked for
    /// the defaults alone ([`Checker::fill_result`]), and each spread's
    /// operand as [`Checker::check_spread`] says with `fill`; nothing else
    /// of the literal changes (see [`Checker::fill_elements`]).
    fn check_list(&mut self, id: ExprId, elements: ElementRange, fill: Option<Stated<'_>>) -> TyId {
        let ast = self.ast;
        let list = ast.elements(elements);
        if list.is_empty() {
            // Against an erroneous expected type, which would have typed it,
            // an empty list is no error of its own.
            if self.quiet.expecting_error > 0 {
                self.quiet.failed = true;
            } else {
                self.errors.push(
                    Diagnostic::error("cannot infer the element type of `[]`", ast.get(id).span)
                        .with_note("annotate it, as in `let xs: List<Int> = []`"),
                );
            }
            return TyId::ERROR;
        }
        // Every element is checked, in order, before any is joined.
        let base = self.element_types.len();
        for element in list {
            let ty = match (element.spread, fill) {
                (Some(spread), _) => self
                    .check_spread(element.value, spread, fill)
                    .and_then(|list| self.types.list_element(list))
                    .unwrap_or(TyId::ERROR),
                (None, Some((record, context))) => self.fill_result(element.value, record, context),
                (None, None) => self.check_expr(element.value),
            };
            self.element_types.push(ty);
        }
        let mut joined = None;
        let mut failed = false;
        // The first element that widens the join, with the two types it
        // joined.
        let mut widened = None;
        for (index, element) in list.iter().enumerate() {
            let ty = self
                .element_types
                .get(base + index)
                .copied()
                .unwrap_or(TyId::ERROR);
            if ty == TyId::ERROR {
                failed = true;
            }
            if failed {
                continue;
            }
            let Some(so_far) = joined else {
                joined = Some(ty);
                continue;
            };
            let earlier = list
                .get(..index)
                .unwrap_or_default()
                .iter()
                .filter(|earlier| earlier.spread.is_none())
                .map(|earlier| earlier.value);
            let (ty, widens, sides) = self.join_element((earlier, element.value), so_far, ty);
            joined = Some(ty);
            if widens && widened.is_none() {
                widened = Some((index, sides));
            }
        }
        self.element_types.truncate(base);
        // Whatever has not adapted to a concrete type is an `Int`.
        for element in list.iter().filter(|element| element.spread.is_none()) {
            let ty = self.expr_types[element.value.index()];
            self.settle(element.value, ty);
        }
        let ty = match joined {
            _ if failed => return TyId::ERROR,
            Some(TyId::UNTYPED_INT) => self.types.list(TyId::INT),
            Some(element) => self.types.list(element),
            None => return TyId::ERROR,
        };
        if let Some((index, sides)) = widened {
            self.elements_widened(list, index, sides, ty);
        }
        ty
    }

    /// The join of `so_far`, the join of the types of the expressions
    /// `earlier`, the elements of a list literal but its spreads or the
    /// values a loop collects, with `ty`, the type of the one after them,
    /// whose expression is `value`, a spread's being its list's element
    /// type: the join, whether it is widened, and the two types as they
    /// were joined. As for an `if`'s branches, an untyped integer, or the
    /// untyped values so far, adapt to the other side's `Int` or `Float`
    /// once stripped, or to `Int`; and if the two types differ, each
    /// parameter of a claimable function (see [`Checker::claimable`]) whose
    /// provisional type is in either is measured, conservatively, at the
    /// value.
    fn join_element(
        &mut self,
        (earlier, value): (impl Iterator<Item = ExprId>, ExprId),
        so_far: TyId,
        ty: TyId,
    ) -> (TyId, bool, (TyId, TyId)) {
        let (mut so_far, mut ty) = (so_far, ty);
        if so_far == TyId::UNTYPED_INT && ty == TyId::UNTYPED_INT {
            return (so_far, false, (so_far, ty));
        }
        if so_far == TyId::UNTYPED_INT {
            so_far = self.numeric_target(ty);
            for earlier in earlier {
                self.coerce(earlier, so_far);
            }
        }
        if ty == TyId::UNTYPED_INT {
            ty = self.numeric_target(so_far);
            self.coerce(value, ty);
        }
        if so_far == ty {
            return (ty, false, (so_far, ty));
        }
        let so_far = self.claim(value, so_far);
        let ty = self.claim(value, ty);
        let (joined, widened) = self.types.join(so_far, ty);
        (joined, widened, (so_far, ty))
    }

    /// The type an untyped integer adapts to where it must agree with a
    /// value of type `other`: `other`'s `Int` or `Float`, once stripped, or
    /// its union's (`Int` if it has both), or `Int`.
    fn numeric_target(&self, other: TyId) -> TyId {
        self.numeric_member(self.types.strip(other))
            .unwrap_or(TyId::INT)
    }

    /// The numeric type of `ty`, a present type: `ty` itself if it is `Int`
    /// or `Float`, and for a union its `Int` member if it has one and its
    /// `Float` member otherwise; `None` if it has neither.
    fn numeric_member(&self, ty: TyId) -> Option<TyId> {
        match ty {
            TyId::INT | TyId::FLOAT => Some(ty),
            _ => {
                let members = self.types.union_members(ty)?;
                [TyId::INT, TyId::FLOAT]
                    .into_iter()
                    .find(|numeric| members.contains(numeric))
            }
        }
    }

    /// Warns that element number `index` of `list`, whose type (or, for a
    /// spread, whose list's element type) is `ty`, does not agree with the
    /// type `so_far` of the elements before it, the join of the two being
    /// widened to a union, and the literal being of type `list_ty`: at the
    /// element, labelled with its type, with a label of the elements' so
    /// far on the first, and a note naming the list's type.
    fn elements_widened(
        &mut self,
        list: &[Element],
        index: usize,
        (so_far, ty): (TyId, TyId),
        list_ty: TyId,
    ) {
        let (Some(first), Some(element)) = (list.first(), list.get(index)) else {
            return;
        };
        let label = match element.spread {
            Some(_) => format!("its elements are {}", self.in_label(ty)),
            None => format!("this is {}", self.in_label(ty)),
        };
        let context = format!("the elements so far are {}", self.in_label(so_far));
        let note = format!(
            "the list is {}; annotate it with that type if both are meant, or tell them apart \
             with `is`",
            self.in_message(list_ty)
        );
        self.warnings.push(
            Diagnostic::warning(
                "the elements of this list have different types",
                element.span(self.ast),
            )
            .with_label(label)
            .with_context(first.span(self.ast), context)
            .with_note(note),
        );
    }

    /// Checks the operand `value` of the spread whose `...` is at `spread`,
    /// which is used as its bits at the top (see [`Checker::measure`]):
    /// returns the list type it has once measured, or `None`, having
    /// reported "cannot spread a value of type T: it is not a list" for
    /// anything else, or if it is erroneous. With `fill`, a record type with
    /// defaults that the list's elements are expected to be of, with where
    /// that is stated and its label, an operand that is, through any
    /// parentheses, a `for` has the record literal its body results in
    /// given the defaults it lacks ([`Checker::check_for`]), and one that
    /// is a block with a result or an `if` with `else` has the elements of
    /// a list literal it results in, or the body of a `for` it does, checked
    /// so ([`Checker::fill_elements`]); the operand is otherwise checked as
    /// it is with nothing expected, a list literal included.
    fn check_spread(
        &mut self,
        value: ExprId,
        spread: Span,
        fill: Option<Stated<'_>>,
    ) -> Option<TyId> {
        let found = match fill {
            Some((record, context))
                if self.loop_within(value).is_some() || self.reaches_result(value) =>
            {
                self.fill_elements(value, record, context)
            }
            _ => self.check_expr(value),
        };
        let found = self.settle(value, found);
        let ty = self.measure(value, found);
        if ty == TyId::ERROR {
            return None;
        }
        if self.types.list_element(ty).is_some() {
            return Some(ty);
        }
        // A list of type `Never` is never there to spread: as a list of
        // `Never`, it adds nothing to the join of the elements.
        if ty == self.never {
            return Some(self.types.list(ty));
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        let span = self.ast.get(value).span;
        let mut error = Diagnostic::error(
            format!("cannot spread a value of type {shown}: it is not a list"),
            Span::join(spread, span),
        )
        .with_context(span, label);
        if let Some(note) = self.union_note(value, ty) {
            error = error.with_note(note);
        }
        self.errors.push(error);
        None
    }

    /// Types a list literal whose elements are `elements` where a value of
    /// type `expected` is expected, a list type `List<X>` once stripped of
    /// its either shape, as stated at `context`, labelled `why` (see the
    /// module documentation): each element is checked against `X` as an
    /// argument is against its parameter, and a spread's list's elements
    /// must fit `X` as they are. The literal is the list of the join of what
    /// its elements are once the awaits marked on them have run (see
    /// [`Checker::fit`]), a subtype of `List<X>`; `List<X>` itself should
    /// they have no join that is one. An empty literal is the list of `X`
    /// made present, where that fits `X`, since it has no element to be a
    /// future, and of `X` otherwise. `Error` if an element does not fit.
    fn check_list_expecting(
        &mut self,
        elements: ElementRange,
        expected: TyId,
        (context, why): (Span, &str),
    ) -> TyId {
        let stripped = self.types.strip(expected);
        let Some(element) = self.types.list_element(stripped) else {
            return TyId::ERROR;
        };
        let list = self.ast.elements(elements);
        if list.is_empty() {
            let present = self.types.present(element);
            let element = if self.types.is_subtype(present, element) {
                present
            } else {
                element
            };
            return self.types.list(element);
        }
        let base = self.element_types.len();
        for item in list {
            let ty = match item.spread {
                Some(spread) => self.fit_spread(item.value, spread, element, (context, why)),
                None => self.fit(item.value, element, (context, why)),
            };
            self.element_types.push(ty);
        }
        let end = self.element_types.len();
        // Each element fits what is expected, so a widened join is no
        // warning.
        let mut joined = self.element_types.get(base).copied();
        for at in base + 1..end {
            let value = self.element_types.get(at).copied().unwrap_or(TyId::ERROR);
            joined = joined.map(|so_far| self.types.join(so_far, value).0);
        }
        let failed = self
            .element_types
            .get(base..)
            .unwrap_or_default()
            .contains(&TyId::ERROR);
        self.element_types.truncate(base);
        if failed {
            return TyId::ERROR;
        }
        let joined = joined
            .filter(|&joined| self.types.is_subtype(joined, element))
            .unwrap_or(element);
        self.types.list(joined)
    }

    /// Checks the operand `value` of the spread whose `...` is at `spread`
    /// in a list literal whose elements are expected to be of type
    /// `element`, as stated at `context`, labelled `why`: the operand is
    /// measured at the top and must be a list (see [`Checker::check_spread`])
    /// whose element type fits `element` as it is, once a provisional type
    /// in it where `element` is present is measured. There is no expression
    /// to await for each element, so a list whose elements would fit only
    /// once awaited is a mismatch, whose note says so. Where `element` is a
    /// record type with defaults itself, an operand that is a `for` has the
    /// record literal its body results in given them first, and one that is
    /// a block with a result or an `if` with `else` the record literals
    /// among the elements of a list literal it results in (see
    /// [`Checker::check_spread`]). Returns the list's element type, or
    /// `Error` if it does not fit.
    fn fit_spread(
        &mut self,
        value: ExprId,
        spread: Span,
        element: TyId,
        (context, why): (Span, &str),
    ) -> TyId {
        let fill = self
            .defaulted_record(element)
            .map(|record| (record, (context, why)));
        let Some(list) = self.check_spread(value, spread, fill) else {
            return TyId::ERROR;
        };
        let expected = self.types.list(element);
        let list = if self.types.is_subtype(list, expected) {
            list
        } else {
            match self.claim_against(value, list, expected) {
                Some(claimed) if self.types.is_subtype(claimed, expected) => claimed,
                _ => {
                    let found = self.types.list_element(list).unwrap_or(TyId::ERROR);
                    let label = format!(
                        "expected {}, found {}",
                        self.in_label(expected),
                        self.in_label(list)
                    );
                    let mut error = Diagnostic::error("mismatched types", self.ast.get(value).span)
                        .with_label(label)
                        .with_context(context, why);
                    if self.types.needs_await(found, element).is_some() {
                        error = error.with_note(
                            "its elements would fit once awaited, but a spread cannot await them: \
                             there is no expression to await; spread a list of their awaited \
                             values instead",
                        );
                    }
                    self.errors.push(error);
                    return TyId::ERROR;
                }
            }
        };
        self.types.list_element(list).unwrap_or(TyId::ERROR)
    }

    /// Types the index `id`, reading the element of `list` at `index` (see
    /// the module documentation): the list is measured at the top, and must
    /// be a list, or a union of lists (see [`Checker::element_of`]),
    /// "cannot index a value of type T: it is not a list", reported at the
    /// `[INDEX]`; the position is measured, and must be an
    /// `Int`, an untyped integer being one, "a list index must be an Int,
    /// found T". The index's type is the list's element type as it is, which
    /// may be a future or the either shape; `Error` if either is wrong.
    fn check_index(&mut self, id: ExprId, list: ExprId, index: ExprId) -> TyId {
        let list_ty = self.check_expr(list);
        let list_ty = self.settle(list, list_ty);
        let list_ty = self.measure(list, list_ty);
        let index_fits = self.check_position(index);
        // A list or a position of type `Never` is never there to read.
        if list_ty == TyId::ERROR || list_ty == self.never {
            return list_ty;
        }
        if self.expr_types[index.index()] == self.never {
            return self.never;
        }
        let Some(element) = self.element_of(list_ty) else {
            let expr = self.ast.get(id);
            let brackets = expr.bracket_span(self.ast).unwrap_or(expr.span);
            let shown = self.in_message(list_ty);
            let label = format!("this is {}", self.in_label(list_ty));
            let mut error = Diagnostic::error(
                format!("cannot index a value of type {shown}: it is not a list"),
                brackets,
            )
            .with_context(self.ast.get(list).span, label);
            if let Some(note) = self.union_note(list, list_ty) {
                error = error.with_note(note);
            }
            self.errors.push(error);
            return TyId::ERROR;
        };
        if index_fits { element } else { TyId::ERROR }
    }

    /// Checks `index`, the position of an index or of an index segment of a
    /// `:=` path, an expression of the enclosing function like any other,
    /// and measures it at the top: it must be an `Int`, an untyped integer
    /// being one, which is coerced, and is otherwise reported, "a list index
    /// must be an Int, found T", at the position. Returns whether it is one;
    /// an erroneous position has been reported already.
    fn check_position(&mut self, index: ExprId) -> bool {
        let index_ty = self.check_expr(index);
        let index_ty = self.measure(index, index_ty);
        match index_ty {
            TyId::UNTYPED_INT => {
                self.coerce(index, TyId::INT);
                true
            }
            TyId::INT => true,
            TyId::ERROR => false,
            // A position of type `Never` is never there to use.
            ty if ty == self.never => true,
            _ => {
                let shown = self.in_message(index_ty);
                let label = format!("this is {}", self.in_label(index_ty));
                let mut error = Diagnostic::error(
                    format!("a list index must be an Int, found {shown}"),
                    self.ast.get(index).span,
                )
                .with_label(label);
                if let Some(note) = self.union_note(index, index_ty) {
                    error = error.with_note(note);
                }
                self.errors.push(error);
                false
            }
        }
    }

    /// Types the loop `id`, whose details are `for_id` (see the module
    /// documentation, [For](#for) and [Loops](#loops)). The loop has a
    /// scope of its own, as a block has, which holds an iteration's
    /// variable, or a header's items, and the body; a `do` loop's condition
    /// is outside its body's block. The body is checked with the loop on
    /// [`Checker::loops`], so that the `break`s and `continue`s in it find
    /// it, and what they and the body contribute make the loop's element
    /// type (see [`Checker::element_type`]). With `expected`, the type
    /// expected of the whole loop, with where that is stated and its label,
    /// every `return` for the loop is checked against it, and if it has a
    /// list type (see [`Checker::expected_member`]), the body and each
    /// value against that list's element type, as a list literal's elements
    /// are (see [`Checker::fit`]). Without one, but with `fill`, a record
    /// type with defaults that the loop's elements are expected to be of,
    /// with where that is stated and its label, as a spread's in a list
    /// literal are, the body is checked for a record literal it results in
    /// alone (see [`Checker::fill_result`]), and nothing else of the loop
    /// is. The loop's ordinary type is the list of
    /// its element type, or `Never` if what always runs first, an iterable
    /// or a condition before the first iteration, is, or if it cannot end
    /// (see [Never-ending loops](#never-ending-loops)); `Error` if the
    /// iterable, the body or a value is erroneous. A loop that a `return`
    /// leaves is the join of that and the values returned (see
    /// [`Checker::returned_loop`]).
    fn check_for(
        &mut self,
        id: ExprId,
        for_id: ForId,
        (expected, fill): (Option<Stated<'_>>, Option<Stated<'_>>),
    ) -> TyId {
        let element = expected.and_then(|(whole, context)| {
            let list = self.expected_member(whole, ValueKind::List)?;
            Some((
                self.types.list_element(list).unwrap_or(TyId::ERROR),
                context,
            ))
        });
        let decl = *self.ast.for_decl(for_id);
        let outer = (self.block_start, self.fields_start, self.next_slot);
        let outer_spreads = std::mem::take(&mut self.spreads);
        self.block_start = self.bindings.len();
        self.fields_start = self.fields.len();
        let head = match decl.head {
            ForHead::Each {
                name,
                name_span,
                iterable,
                guard,
            } => self.each_head(id, for_id, (name, name_span), (iterable, guard)),
            ForHead::While {
                before,
                cond,
                after,
            } => self.while_head(id, before, cond, after),
            ForHead::Do { .. } => TyId::EMPTY,
        };
        let owned = |(ty, (context, why)): (TyId, (Span, &str))| (ty, context, why.to_owned());
        let frame = LoopFrame::new(
            (Some(for_id), id),
            decl.label,
            self.functions.len(),
            element.map(owned),
            expected.map(owned),
        );
        self.enter_target(frame);
        let body = match (element, fill) {
            (Some((element, context)), _) => self.fit(decl.body, element, context),
            (None, Some((record, context))) => self.fill_result(decl.body, record, context),
            (None, None) => self.check_expr(decl.body),
        };
        let frame = self.loops.pop();
        if let ForHead::Do { cond } = decl.head {
            // Outside the body's block, and asked only after the body: it
            // makes the loop `Never` in no case.
            self.loop_condition(id, cond);
        }
        self.bindings.truncate(self.block_start);
        (self.block_start, self.fields_start, self.next_slot) = outer;
        self.spreads = outer_spreads;
        let (values, broken, returns) = frame
            .map(|frame| (frame.values, frame.broken, frame.returns))
            .unwrap_or_default();
        let collected = self.element_type(id, (decl.body, body), values, element);
        // A loop with no condition ends only by a `break`.
        let endless = matches!(decl.head, ForHead::While { cond: None, .. }) && !broken;
        if let Some(resolution) = self.loop_resolutions.get_mut(for_id.index()) {
            resolution.endless = endless;
        }
        let ordinary = if head == TyId::ERROR || collected == TyId::ERROR {
            TyId::ERROR
        } else if head == self.never || endless {
            self.never
        } else {
            self.types.list(collected)
        };
        if returns.is_empty() {
            return ordinary;
        }
        self.returned_loop(id, ordinary, &returns, expected)
    }

    /// The type of the loop `id`, whose ordinary type, what it is when it
    /// ends, is `ordinary`, and which the `return`s `returns` leave, with
    /// `expected`, the type expected of the whole loop, if one is, with
    /// where that is stated and its label: the join of its ordinary type
    /// and the values returned (see [`return`](#return)). Under an expected
    /// type each value fits it already, and a loop that can end must fit it
    /// as a list too: otherwise it is "this loop may end as a list,
    /// `List<{}>`, which is not Int", at its keyword, and `Error`. Without
    /// one, a join that widens is a warning, "this loop is its list or a
    /// value returned to it, of different types".
    fn returned_loop(
        &mut self,
        id: ExprId,
        ordinary: TyId,
        returns: &[Site],
        expected: Option<(TyId, (Span, &str))>,
    ) -> TyId {
        if ordinary == TyId::ERROR {
            return TyId::ERROR;
        }
        let keyword = self.ast.loop_keyword(id);
        let mut sites = Vec::with_capacity(returns.len() + 1);
        sites.push(Site {
            value: None,
            at: keyword,
            ty: ordinary,
        });
        sites.extend_from_slice(returns);
        let Some((whole, _)) = expected else {
            let (joined, widened) = self.join_sites(id, &sites, true);
            if widened {
                let note = self.union_note_of("loop", joined);
                let list = format!("its list is {}", self.in_label(ordinary));
                let mut warning = Diagnostic::warning(
                    "this loop is its list or a value returned to it, of different types",
                    keyword,
                )
                .with_label(list);
                let never = self.never;
                for site in returns.iter().filter(|site| site.ty != never) {
                    let label = format!("this is {}", self.in_label(self.site_type(site)));
                    warning = warning.with_context(site.at, label);
                }
                self.warnings.push(warning.with_note(note));
            }
            return joined;
        };
        // An expected type in error has been reported.
        if whole == TyId::ERROR {
            return TyId::ERROR;
        }
        if ordinary != self.never && !self.types.is_subtype(ordinary, whole) {
            let (list, shown) = (self.in_message(ordinary), self.in_message(whole));
            let (joined, _) = self.types.join(whole, ordinary);
            let annotation = self.types.display(joined, self.ast.names()).to_string();
            self.errors.push(
                Diagnostic::error(
                    format!("this loop may end as a list, {list}, which is not {shown}"),
                    keyword,
                )
                .with_note(format!(
                    "annotate it as `{annotation}`, or return from every way the loop can end"
                )),
            );
            return TyId::ERROR;
        }
        // Each value fits what is expected, so the join is no warning; and
        // what is expected describes them all should it not be a subtype.
        let (joined, _) = self.join_sites(id, &sites, true);
        if joined == TyId::ERROR || self.types.is_subtype(joined, whole) {
            joined
        } else {
            whole
        }
    }

    /// Checks the head of the iteration `id`, whose details are `for_id`,
    /// in the loop's scope: the iterable, outside it, measured at the top,
    /// which must be a list, "cannot iterate over a value of type T: it is
    /// not a list", its elements not measured; the variable `name`, written
    /// at `name_span`, declared with the element type as it is in the next
    /// free slot of the current frame, which the loop leaves when it ends;
    /// and the guard, if any, checked as an `if`'s condition is. Returns
    /// `Error` if the iterable is erroneous, `Never` if it is of that type,
    /// the variable then being `Never` too, and `{}` otherwise.
    fn each_head(
        &mut self,
        id: ExprId,
        for_id: ForId,
        (name, name_span): (Name, Span),
        (iterable, guard): (ExprId, Option<ExprId>),
    ) -> TyId {
        let ty = self.check_expr(iterable);
        let ty = self.settle(iterable, ty);
        let ty = self.measure(iterable, ty);
        // An iterable of type `Never` is never there to iterate over.
        let item = if ty == self.never {
            self.never
        } else {
            self.iterated(iterable, ty)
        };
        let slot = self.declare_binding(name, item, Some(name_span), BindingKind::Variable);
        self.for_slots[for_id.index()] = slot;
        if let Some(guard) = guard {
            let keyword = self.ast.loop_keyword(id);
            let ty = self.check_expr(guard);
            self.expect(guard, ty, TyId::BOOL, keyword, "`for` guards are Bool");
        }
        match item {
            TyId::ERROR => TyId::ERROR,
            item if item == self.never => item,
            _ => TyId::EMPTY,
        }
    }

    /// Checks the header of the condition loop `id`, in the loop's scope:
    /// the items `before` its condition, in order, the condition `cond`, if
    /// it has one (see [`Checker::loop_condition`]), and the items `after`
    /// it. A plain `let` there declares a variable that only the header may
    /// rebind (see [`Checker::header_item`]). Returns `Never` if the
    /// condition, which runs before the first iteration, is of that type,
    /// and `{}` otherwise.
    fn while_head(
        &mut self,
        id: ExprId,
        before: ItemRange,
        cond: Option<ExprId>,
        after: ItemRange,
    ) -> TyId {
        for &item in self.ast.items(before) {
            self.header_item(item);
        }
        let cond = cond.map(|cond| self.loop_condition(id, cond));
        for &item in self.ast.items(after) {
            self.header_item(item);
        }
        match cond {
            Some(cond) if cond == self.never => cond,
            _ => TyId::EMPTY,
        }
    }

    /// Checks the condition `cond` of the loop `id`, which is checked once
    /// and evaluated before every iteration, or after each for a `do` loop:
    /// as an `if`'s condition is, measured, and `Bool`, but narrowing
    /// nothing. Returns its type.
    fn loop_condition(&mut self, id: ExprId, cond: ExprId) -> TyId {
        let keyword = self.ast.loop_keyword(id);
        let ty = self.check_expr(cond);
        self.expect(cond, ty, TyId::BOOL, keyword, "`for` conditions are Bool");
        ty
    }

    /// Checks the item `id` of a `for` header: a `let`, checked as in a
    /// block, whose variable, unless it is a `let mut`, is one that only a
    /// `:=` of the header may rebind ([`BindingKind::Header`]), keeping its
    /// one slot and its type; or a `:=`, checked as in a block, so that it
    /// rebinds such a variable, or a `let mut`, in its slot.
    fn header_item(&mut self, id: ItemId) {
        match self.ast.item(id).kind {
            ItemKind::Let {
                path,
                ty,
                value,
                mutable,
                ..
            } => {
                let before = self.bindings.len();
                self.check_let(id, path, ty, (value, mutable));
                if self.bindings.len() > before
                    && let Some(binding) = self.bindings.last_mut()
                    && binding.kind == BindingKind::Variable
                {
                    binding.kind = BindingKind::Header;
                }
            }
            ItemKind::Rebind { path, value } => self.check_rebind(id, path, value),
            // The parser allows no other item in a header.
            ItemKind::Field { .. }
            | ItemKind::Spread(_)
            | ItemKind::Result(_)
            | ItemKind::Expr(_)
            | ItemKind::Break(_)
            | ItemKind::Continue(_)
            | ItemKind::Return(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::DefTypeDef { .. }
            | ItemKind::DefMeta { .. }
            | ItemKind::Error => {}
        }
    }

    /// Puts `frame`, a loop's or a labelled block's, on [`Checker::loops`],
    /// for its body. Loop and block labels are one namespace: a label that
    /// a loop or labelled block of the same function holding this one has
    /// already is "label `'L` is already in use", at the label, labelled at
    /// the other; a function starts with no labels.
    fn enter_target(&mut self, frame: LoopFrame) {
        if let Some(label) = frame.label
            && let Some((outer, noun)) = self
                .loops
                .iter()
                .rev()
                .take_while(|outer| outer.depth == frame.depth)
                .find_map(|outer| {
                    let found = outer.label.filter(|outer| outer.name == label.name)?;
                    Some((found, outer.noun()))
                })
        {
            let text = self.ast.names().get(label.name);
            self.errors.push(
                Diagnostic::error(format!("label `'{text}` is already in use"), label.span)
                    .with_context(
                        outer.span,
                        format!("the {noun} around this one is labelled so"),
                    )
                    .with_note(format!("give the inner {} another label", frame.noun())),
            );
        }
        self.loops.push(frame);
    }

    /// Checks the block `id`, labelled `label`, whose items are `items`, as
    /// [`Checker::check_block`] does, with `expecting` as for it, and with
    /// the block on [`Checker::loops`], so that the `return`s that name it
    /// find it: its type is the join of its result and of the values they
    /// return (see [`Checker::leave_block`]).
    fn labelled_block(
        &mut self,
        id: ExprId,
        label: LabelId,
        items: ItemRange,
        expecting: Option<Expecting<'_>>,
    ) -> TyId {
        self.enter_block(id, self.ast.label(label), expecting);
        let ty = self.check_block(items, expecting);
        self.leave_block(id, ty, expecting)
    }

    /// Puts the block `id`, labelled `label`, on [`Checker::loops`], for
    /// its items: the `return`s that name it are checked against what
    /// `expecting` says is expected of it, if anything is.
    fn enter_block(&mut self, id: ExprId, label: Label, expecting: Option<Expecting<'_>>) {
        let expected =
            expecting.map(|expecting| (expecting.ty, expecting.context, expecting.why.to_owned()));
        let frame = LoopFrame::new(
            (None, id),
            Some(label),
            self.functions.len(),
            None,
            expected,
        );
        self.enter_target(frame);
    }

    /// Takes the labelled block `id`, whose result is of type `result`, off
    /// [`Checker::loops`], and returns its type: the join of its result and
    /// of the values the `return`s that name it return, as an `if`'s
    /// branches are joined, untyped integers adapting to each other and
    /// staying untyped if all are (see [`Checker::join_sites`]). With
    /// `expecting`, each fits what it expects already, and the join is no
    /// warning, but is the type expected should it not be a subtype of it;
    /// without, a join that widens is a warning, "the values this block
    /// returns have different types", at its label, labelled at each.
    fn leave_block(&mut self, id: ExprId, result: TyId, expecting: Option<Expecting<'_>>) -> TyId {
        let Some(frame) = self.loops.pop() else {
            return result;
        };
        if frame.returns.is_empty() {
            return result;
        }
        let values: Vec<ExprId> = frame.returns.iter().filter_map(|site| site.value).collect();
        if !values.is_empty() {
            self.block_returns.insert(id, values);
        }
        // The result site is the block itself, which an untyped integer is
        // pushed into as into its result.
        self.expr_types[id.index()] = result;
        let at = self.ast.get(self.branch_value(id)).span;
        let mut sites = Vec::with_capacity(frame.returns.len() + 1);
        sites.push(Site {
            value: Some(id),
            at,
            ty: result,
        });
        sites.extend_from_slice(&frame.returns);
        let (joined, widened) = self.join_sites(id, &sites, false);
        match expecting {
            Some(expecting)
                if joined == TyId::ERROR || self.types.is_subtype(joined, expecting.ty) =>
            {
                joined
            }
            Some(expecting) => expecting.ty,
            None => {
                if widened && let Some(label) = frame.label {
                    self.sites_widened(
                        (
                            "the values this block returns have different types",
                            label.span,
                        ),
                        &sites,
                        ("block", joined),
                    );
                }
                joined
            }
        }
    }

    /// The join of the types of `sites`, the result sites of the block,
    /// loop or `lazy` `anchor`, in order, as the branches of an `if` are
    /// joined (see [`Checker::join_element`]): a site of type `Never` adds
    /// nothing, untyped integers adapt to each other and to the numeric
    /// types beside them, and staying untyped if all are, unless `settle`,
    /// when they settle to `Int`; and whether it widened. `Never` if every
    /// site is, and `Error` if any is erroneous. A site without a value, a
    /// `return` without one or a loop's list, is pushed nothing into; the
    /// parameters measured where the types differ are measured at a site's
    /// value, or at `anchor` for one without.
    fn join_sites(&mut self, anchor: ExprId, sites: &[Site], settle: bool) -> (TyId, bool) {
        let live: Vec<(ExprId, TyId)> = sites
            .iter()
            .filter(|site| site.ty != self.never)
            .map(|site| (site.value.unwrap_or(anchor), site.ty))
            .collect();
        if live.iter().any(|&(_, ty)| ty == TyId::ERROR) {
            return (TyId::ERROR, false);
        }
        let mut joined = None;
        let mut widened = false;
        for (index, &(value, ty)) in live.iter().enumerate() {
            let Some(so_far) = joined else {
                joined = Some(ty);
                continue;
            };
            // A site without a value of its own is never untyped, so
            // nothing is pushed into `anchor` for it.
            let earlier: Vec<ExprId> = live.iter().take(index).map(|&(at, _)| at).collect();
            let (ty, widens, _) = self.join_element((earlier.into_iter(), value), so_far, ty);
            joined = Some(ty);
            widened |= widens;
        }
        let joined = match joined {
            Some(TyId::UNTYPED_INT) if settle => {
                for &(value, _) in &live {
                    let ty = self.expr_types[value.index()];
                    self.settle(value, ty);
                }
                TyId::INT
            }
            Some(joined) => joined,
            None => self.never,
        };
        (joined, widened)
    }

    /// The type of `site` as a warning about it shows it: that of its value
    /// as it now is, an untyped integer having adapted, or the site's own.
    fn site_type(&self, site: &Site) -> TyId {
        site.value
            .map_or(site.ty, |value| self.expr_types[value.index()])
    }

    /// Warns, `message` at `at`, that `sites`, the result sites of a
    /// block, a loop or a `lazy`, have different types, which `joined`, the
    /// type of `what`, widened to a union of: labelled at each site with its
    /// type, with slice 20's note naming the union.
    fn sites_widened(
        &mut self,
        (message, at): (&str, Span),
        sites: &[Site],
        (what, joined): (&str, TyId),
    ) {
        let note = self.union_note_of(what, joined);
        let mut warning = Diagnostic::warning(message, at);
        let never = self.never;
        for site in sites.iter().filter(|site| site.ty != never) {
            let label = format!("this is {}", self.in_label(self.site_type(site)));
            warning = warning.with_context(site.at, label);
        }
        self.warnings.push(warning.with_note(note));
    }

    /// The note on a warning that the values a `what` may have widened to
    /// the union `joined`: "the `what` is `Int | String`; annotate it with
    /// that type if all are meant, or tell them apart with `is`".
    fn union_note_of(&mut self, what: &str, joined: TyId) -> String {
        format!(
            "the {what} is {}; annotate it with that type if all are meant, or tell them apart \
             with `is`",
            self.in_message(joined)
        )
    }

    /// The element type of the loop `id`, whose body, `body`, is of type
    /// `body_ty`, and for which the `break`s and `continue`s with a value
    /// gave `values`: each value with its type (see [`LoopFrame::values`]).
    /// These are joined as the elements of a list literal are (see
    /// [`Checker::check_list`]), in order, the body last, a `Never` among
    /// them adding nothing: with `element`, the element type expected, each
    /// fits it already, and the join is no warning, but is `element` should
    /// it not be a subtype of it; without, untyped integers adapt to each
    /// other and settle to `Int`, and a join that widens is a warning (see
    /// [`Checker::values_widened`]). If nothing contributes, or only
    /// `Never`, it is `element`, or `{}` without one. `Error` if anything
    /// contributing is.
    fn element_type(
        &mut self,
        id: ExprId,
        (body, body_ty): (ExprId, TyId),
        values: Vec<(ExprId, TyId)>,
        element: Option<(TyId, (Span, &str))>,
    ) -> TyId {
        let mut sites = values;
        sites.push((body, body_ty));
        sites.retain(|&(_, ty)| ty != self.never);
        if let Some((element, _)) = element {
            let mut joined = None;
            for &(_, ty) in &sites {
                joined = Some(match joined {
                    None => ty,
                    Some(so_far) => self.types.join(so_far, ty).0,
                });
            }
            return match joined {
                Some(TyId::ERROR) => TyId::ERROR,
                Some(joined) if self.types.is_subtype(joined, element) => joined,
                _ => element,
            };
        }
        let mut joined = None;
        let mut widened = false;
        for index in 0..sites.len() {
            let Some(&(value, ty)) = sites.get(index) else {
                continue;
            };
            let Some(so_far) = joined else {
                joined = Some(ty);
                continue;
            };
            let earlier: Vec<ExprId> = sites.iter().take(index).map(|&(at, _)| at).collect();
            let (ty, widens, _) = self.join_element((earlier.into_iter(), value), so_far, ty);
            joined = Some(ty);
            widened |= widens;
        }
        // Whatever has not adapted to a concrete type is an `Int`.
        for &(value, _) in &sites {
            let ty = self.expr_types[value.index()];
            self.settle(value, ty);
        }
        let element = match joined {
            _ if sites.iter().any(|&(_, ty)| ty == TyId::ERROR) => return TyId::ERROR,
            Some(TyId::UNTYPED_INT) => TyId::INT,
            Some(joined) => joined,
            None => TyId::EMPTY,
        };
        if widened {
            self.values_widened(id, &sites, element);
        }
        element
    }

    /// Warns that the values the loop `id` collects, at `sites`, its
    /// `break`s' and `continue`s' values and its body last, each with its
    /// type, have different types, which `element`, its element type,
    /// widened to a union of: "the values this loop collects have different
    /// types", at the loop's keyword, labelled at each site with its type,
    /// the body at its value (see [`Checker::branch_value`]), and a note
    /// naming the loop's type.
    fn values_widened(&mut self, id: ExprId, sites: &[(ExprId, TyId)], element: TyId) {
        let list = self.types.list(element);
        let note = format!(
            "the loop is {}; annotate it with that type if all are meant, or tell them apart \
             with `is`",
            self.in_message(list)
        );
        let mut warning = Diagnostic::warning(
            "the values this loop collects have different types",
            self.ast.loop_keyword(id),
        );
        for &(value, _) in sites {
            let ty = self.expr_types[value.index()];
            let label = format!("this is {}", self.in_label(ty));
            let at = self.branch_value(value);
            warning = warning.with_context(self.ast.get(at).span, label);
        }
        self.warnings.push(warning.with_note(note));
    }

    /// Checks the `break` or `continue` item `id`, `item`, whose label and
    /// value are `jump`, in a block whose first field, if it has one, is
    /// `first_field` (see the module documentation): as an item of a record
    /// block, it is "`break` cannot be an item of a record block";
    /// otherwise it is for the loop [`Checker::jump_target`] finds, which
    /// is recorded for lowering, and its value, if it has one, is one of
    /// what that loop collects (see [`Checker::element_type`]), checked
    /// against the loop's expected element type if it has one. Where there
    /// is no loop for it, its value is checked on its own.
    fn check_jump(&mut self, (id, item): (ItemId, Item), jump: Jump, first_field: Option<ItemId>) {
        let keyword = match item.kind {
            ItemKind::Break(_) => "break",
            _ => "continue",
        };
        let at = jump_span(keyword, item.span, jump.label);
        let target = if first_field.is_some() {
            self.in_record_error(keyword, at);
            None
        } else {
            self.jump_target(keyword, at, jump.label)
        };
        let Some(frame) = target else {
            if let Some(value) = jump.value {
                self.check_alone(value);
            }
            return;
        };
        let Some((target, expected)) = self.loops.get_mut(frame).map(|frame| {
            frame.broken |= keyword == "break";
            (frame.target(), frame.expected.clone())
        }) else {
            return;
        };
        self.jumps[id.index()] = Some(target);
        let Some(value) = jump.value else {
            return;
        };
        let ty = match expected {
            Some((element, context, why)) => self.fit(value, element, (context, &why)),
            None => self.check_expr(value),
        };
        if let Some(frame) = self.loops.get_mut(frame) {
            frame.values.push((value, ty));
        }
    }

    /// Reports the `break`, `continue` or `return`, as `keyword` says,
    /// written at `at`, as an item of a record block, which nothing can
    /// leave.
    fn in_record_error(&mut self, keyword: &str, at: Span) {
        let leave = if keyword == "return" {
            "return"
        } else {
            "leave the loop"
        };
        self.errors.push(
            Diagnostic::error(
                format!("`{keyword}` cannot be an item of a record block"),
                at,
            )
            .with_note(format!(
                "a record block's value is the record of its fields; {leave} from a block that \
                 is not one, such as a field's value"
            )),
        );
    }

    /// The index in [`Checker::loops`] of the loop that the `break` or
    /// `continue`, as `keyword` says, written at `at`, with `label`, if it
    /// has one, is for: the innermost loop of the function being checked
    /// whose body holds it, labelled `label` if it has one; or for a
    /// `return`, which always has a label, the innermost loop or labelled
    /// block of that label holding it. Where there is none, reports why and
    /// returns `None`: "`break` outside a loop", if no loop holds it; "no
    /// enclosing loop is labelled `'L`" (for a `return`, "no enclosing
    /// block or loop is labelled `'L`"), at the label, if nothing it could
    /// be for is labelled so; "`'L` labels a block, not a loop" if a
    /// `break` or `continue` names a block, labelled at the block's label;
    /// and if the only such loops or blocks are outside the function or
    /// `lazy` being checked, "`break` cannot leave a function" (or "a
    /// `lazy`"), labelled at the function's keyword, and at the loop's or
    /// block's label for a label, with a note saying when the body runs.
    fn jump_target(&mut self, keyword: &str, at: Span, label: Option<Label>) -> Option<usize> {
        let depth = self.functions.len();
        let returning = keyword == "return";
        let names = |frame: &LoopFrame| match label {
            Some(label) => frame.label.is_some_and(|own| own.name == label.name),
            // An unlabelled `break` or `continue` is for a loop.
            None => frame.id.is_some(),
        };
        let found = self.loops.iter().rposition(names);
        let found = match found.and_then(|index| Some((index, self.loops.get(index)?))) {
            Some((index, frame)) if frame.depth == depth => {
                if returning || frame.id.is_some() {
                    return Some(index);
                }
                let error = self.not_a_loop(at, frame.label);
                self.errors.push(error);
                return None;
            }
            found => found.map(|(_, frame)| frame),
        };
        let error = match (found, label) {
            (Some(outer), _) => self.leave_function_error(keyword, at, (label, outer)),
            (None, None) => Diagnostic::error(format!("`{keyword}` outside a loop"), at),
            (None, Some(label)) => {
                let text = self.ast.names().get(label.name);
                let what = if returning { "block or loop" } else { "loop" };
                Diagnostic::error(
                    format!("no enclosing {what} is labelled `'{text}`"),
                    label.span,
                )
            }
        };
        self.errors.push(error);
        None
    }

    /// The error for a `break` or `continue` written at `at` whose label
    /// names a block, labelled `label`: "`'L` labels a block, not a loop",
    /// labelled at the block's label, with a note saying to `return`.
    fn not_a_loop(&self, at: Span, label: Option<Label>) -> Diagnostic {
        let Some(label) = label else {
            return InternalError {
                message: "a block without a label is a target",
                span: at,
            }
            .into();
        };
        let text = self.ast.names().get(label.name);
        Diagnostic::error(format!("`'{text}` labels a block, not a loop"), at)
            .with_context(label.span, "the block is labelled here")
            .with_note(format!("use `return'{text}` to leave a block"))
    }

    /// The error for the `break`, `continue` or `return`, as `keyword`
    /// says, written at `at`, with `label`, if it has one, whose only
    /// target, `outer`, is outside the function or `lazy` being checked:
    /// "`break` cannot leave a function" (or "a `lazy`"), labelled at the
    /// function's keyword, and at the target's label for a label, with a
    /// note saying when the body runs.
    fn leave_function_error(
        &self,
        keyword: &str,
        at: Span,
        (label, outer): (Option<Label>, &LoopFrame),
    ) -> Diagnostic {
        let noun = outer.noun();
        let function = self.functions.last();
        let (kind, when) = match function.map(|function| self.ast.fn_decl(function.fn_id)) {
            Some(decl) if decl.ret.is_none() => ("a `lazy`", "awaited"),
            _ => ("a function", "called"),
        };
        let mut error = Diagnostic::error(format!("`{keyword}` cannot leave {kind}"), at)
            .with_note(format!(
                "the body of {kind} runs when it is {when}, which may be after the {noun} has \
                 ended"
            ));
        if let Some(function) = function {
            error = error.with_context(function.keyword, format!("inside {kind} here"));
        }
        match (label, outer.label) {
            (Some(_), Some(outer)) => {
                error.with_context(outer.span, format!("the {noun} it names is outside it"))
            }
            _ => error,
        }
    }

    /// Checks the `return` item `id`, `item`, whose label and value are
    /// `jump`, in a block whose first field, if it has one, is
    /// `first_field` (see [`return`](#return)): as an item of a record
    /// block, it is "`return` cannot be an item of a record block";
    /// without a label it is for the innermost function or `lazy`, and at
    /// the top level, outside any, "`return` outside a function"; with
    /// one, for the loop or labelled block [`Checker::jump_target`] finds.
    /// What it is for is recorded for lowering, and its value, `{}` without
    /// one, is one of that target's result sites (see
    /// [`Checker::return_site`]). Where there is no target, its value is
    /// checked on its own.
    fn check_return(
        &mut self,
        (id, item): (ItemId, Item),
        jump: Jump,
        first_field: Option<ItemId>,
    ) {
        let at = jump_span("return", item.span, jump.label);
        let target = if first_field.is_some() {
            self.in_record_error("return", at);
            None
        } else {
            match jump.label {
                None if self.functions.is_empty() => {
                    self.errors.push(
                        Diagnostic::error("`return` outside a function", at).with_note(
                            "only a function or a `lazy` can be returned from without a label; a \
                         block or a loop labelled `'L` is left with `return'L`",
                        ),
                    );
                    None
                }
                None => Some(None),
                Some(label) => self.jump_target("return", at, Some(label)).map(Some),
            }
        };
        let Some(frame) = target else {
            if let Some(value) = jump.value {
                self.check_alone(value);
            }
            return;
        };
        let target = match frame.and_then(|frame| self.loops.get(frame)) {
            Some(frame) => frame.target(),
            None => JumpTarget::Function,
        };
        self.jumps[id.index()] = Some(target);
        match target {
            JumpTarget::Loop(loop_id) => {
                if let Some(resolution) = self.loop_resolutions.get_mut(loop_id.index()) {
                    resolution.returned = true;
                }
            }
            JumpTarget::Block(block) => {
                self.returned_blocks.insert(block);
            }
            JumpTarget::Function => {}
        }
        let site = self.return_site(at, jump.value, frame);
        let returns = match frame {
            Some(frame) => self.loops.get_mut(frame).map(|frame| &mut frame.returns),
            None => self
                .functions
                .last_mut()
                .map(|function| &mut function.returns),
        };
        if let Some(returns) = returns {
            returns.push(site);
        }
    }

    /// Checks the value of the `return` written at `at`, if it has one, for
    /// the loop or labelled block `loops[frame]`, or with no frame for the
    /// innermost function or `lazy`, and returns it as one of the target's
    /// result sites. For a function, it is checked against the declared
    /// result type as the body's result is (see [`return`](#return)), and
    /// for a loop or block, against the type expected of the whole, if one
    /// is; otherwise on its own. A `return` without a value gives `{}`,
    /// which must fit the same.
    fn return_site(&mut self, at: Span, value: Option<ExprId>, frame: Option<usize>) -> Site {
        let expected = match frame {
            Some(frame) => self
                .loops
                .get(frame)
                .and_then(|frame| frame.returns_expected.clone()),
            None => self
                .functions
                .last()
                .and_then(|function| function.result)
                .map(|(ret, context)| (ret, context, RESULT_WHY.to_owned())),
        };
        let Some(value) = value else {
            if let Some((expected, context, why)) = expected
                && expected != TyId::ERROR
                && !self.types.is_subtype(TyId::EMPTY, expected)
            {
                let label = format!("expected {}, found {{}}", self.in_label(expected));
                self.errors.push(
                    Diagnostic::error("mismatched types", at)
                        .with_label(label)
                        .with_context(context, why)
                        .with_note("a `return` without a value returns `{}`"),
                );
            }
            return Site {
                value: None,
                at,
                ty: TyId::EMPTY,
            };
        };
        let ty = match (expected, frame) {
            // As the body's result is, whose type is its own.
            (Some((ret, context, why)), None) => {
                self.check_expr_expecting(value, ret, context, &why)
            }
            (Some((expected, context, why)), Some(_)) => self.fit(value, expected, (context, &why)),
            (None, _) => self.check_expr(value),
        };
        Site {
            value: Some(value),
            at: self.ast.get(value).span,
            ty,
        }
    }

    /// The element type of `ty`, the type of the iterable `iterable` of a
    /// `for` once measured, a list or a union of lists (see
    /// [`Checker::element_of`]), or `Error`, having reported "cannot iterate
    /// over a value of type T: it is not a list" if it is neither, or if it
    /// is erroneous.
    fn iterated(&mut self, iterable: ExprId, ty: TyId) -> TyId {
        if ty == TyId::ERROR {
            return TyId::ERROR;
        }
        if let Some(element) = self.element_of(ty) {
            return element;
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        let mut error = Diagnostic::error(
            format!("cannot iterate over a value of type {shown}: it is not a list"),
            self.ast.get(iterable).span,
        )
        .with_label(label);
        if let Some(note) = self.union_note(iterable, ty) {
            error = error.with_note(note);
        }
        self.errors.push(error);
        TyId::ERROR
    }

    /// Types `/`: it accepts any mix of numeric operands and always yields
    /// `Float`. Untyped operands become `Float` literals; concrete `Int`
    /// operands are converted by lowering.
    fn division(
        &mut self,
        id: ExprId,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        self.coerce(lhs, TyId::FLOAT);
        self.coerce(rhs, TyId::FLOAT);
        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        if !is_numeric(lhs_ty) || !is_numeric(rhs_ty) {
            let message = format!(
                "cannot {} {} and {}",
                BinaryOp::Div.verb(),
                self.in_message(lhs_ty),
                self.in_message(rhs_ty)
            );
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            // A quantity is whole: it divides only with `//`.
            if let Some(quantity) = [lhs_ty, rhs_ty].into_iter().find(|&ty| is_quantity(ty))
                && let Some(error) = self.errors.last_mut()
            {
                let quantity = quantity.quantity().unwrap_or(Quantity::Duration);
                error.notes.push(format!(
                    "use `//`; a {} is a whole number of {}",
                    quantity.type_name(),
                    quantity.smallest()
                ));
            }
            return TyId::ERROR;
        }
        TyId::FLOAT
    }

    /// Reports an operator rejecting its operands: the message points at the
    /// operator, and each operand is labelled with its type; an operand that
    /// is a union adds the note saying to tell its members apart first (see
    /// [`Checker::union_note`]).
    fn operand_error(
        &mut self,
        id: ExprId,
        message: String,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) {
        let expr = self.ast.get(id);
        let span = expr.operator_span().unwrap_or(expr.span);
        let (lhs_label, rhs_label) = (self.in_label(lhs_ty), self.in_label(rhs_ty));
        let mut error = Diagnostic::error(message, span)
            .with_context(self.ast.get(lhs).span, lhs_label)
            .with_context(self.ast.get(rhs).span, rhs_label);
        for (operand, ty) in [(lhs, lhs_ty), (rhs, rhs_ty)] {
            if let Some(note) = self.union_note(operand, ty)
                && !error.notes.contains(&note)
            {
                error = error.with_note(note);
            }
        }
        self.errors.push(error);
    }

    /// The note on an error where `operand`, of type `ty`, is used as one
    /// particular type and is a union, once stripped: "tell them apart
    /// first, as in `if x is Int { ... }`", naming the variable when
    /// `operand` is one, through any parentheses, and the union's first
    /// member as it shows it that `is` can test (one that is not a function
    /// type); for a union of function types only, which `is` cannot tell
    /// apart, a note saying to annotate the value with one function type
    /// instead. `None` if `ty` is not a union.
    fn union_note(&mut self, operand: ExprId, ty: TyId) -> Option<String> {
        let ty = self.shown(ty);
        let members = self.types.union_members(self.types.strip(ty))?.to_vec();
        let mut node = operand;
        while let ExprKind::Paren(inner) = self.ast.get(node).kind {
            node = inner;
        }
        let name = match self.ast.get(node).kind {
            ExprKind::Name(name) => self.ast.names().get(name),
            _ => "x",
        };
        let example = self
            .types
            .shown_members(&members, self.ast.names())
            .into_iter()
            .find(|&(member, _)| self.types.fn_sig(member).is_none());
        Some(match example {
            Some((_, shown)) => {
                format!("tell them apart first, as in `if {name} is {shown} {{ ... }}`")
            }
            None => "`is` cannot tell function types apart; annotate the value with one \
                     function type that each of them fits"
                .to_owned(),
        })
    }

    /// The element type of `ty`, a type once measured, if it is a list type,
    /// or a union whose members are all list types: the join of their
    /// element types, which is no warning, since the union says the value
    /// is any of them. `None` for any other type.
    fn element_of(&mut self, ty: TyId) -> Option<TyId> {
        if let Some(element) = self.types.list_element(ty) {
            return Some(element);
        }
        let members = self.types.union_members(ty)?.to_vec();
        let mut joined: Option<TyId> = None;
        for member in members {
            let element = self.types.list_element(member)?;
            joined = Some(match joined {
                Some(so_far) => self.types.join(so_far, element).0,
                None => element,
            });
        }
        joined
    }

    /// The type of field `name` of `ty`, a type once measured, if it is a
    /// record type with that field, or a union whose members are all record
    /// types with it: the join of their fields' types, which is no warning,
    /// since the union says the value is any of them. `None` for any other
    /// type.
    fn field_of(&mut self, ty: TyId, name: Name) -> Option<TyId> {
        if self.types.is_record(ty) {
            return self.types.field(ty, name);
        }
        let members = self.types.union_members(ty)?.to_vec();
        let mut joined: Option<TyId> = None;
        for member in members {
            let field = self.types.field(member, name)?;
            joined = Some(match joined {
                Some(so_far) => self.types.join(so_far, field).0,
                None => field,
            });
        }
        joined
    }

    /// Types `//` and `%`: both operands must be integers, and the result is
    /// a concrete `Int`. Untyped operands are coerced to `Int`; they never
    /// adapt to `Float`, since these operators do not accept it. A quantity
    /// divided by an `Int` with `//` is of its type, one divided by one of
    /// its type with `//` an `Int`, and the remainder of two of a type with
    /// `%` of their type (see [Quantities](#quantities)).
    fn integer_only(
        &mut self,
        id: ExprId,
        op: BinaryOp,
        (lhs, lhs_ty): (ExprId, TyId),
        (rhs, rhs_ty): (ExprId, TyId),
    ) -> TyId {
        if is_quantity(lhs_ty) {
            match op {
                BinaryOp::FloorDiv if is_int(rhs_ty) => {
                    self.coerce(rhs, TyId::INT);
                    return lhs_ty;
                }
                BinaryOp::FloorDiv if rhs_ty == lhs_ty => return TyId::INT,
                BinaryOp::Rem if rhs_ty == lhs_ty => return lhs_ty,
                _ => {}
            }
        }
        // A quantity on either side is no `Int` to reject alone: what it
        // takes is said.
        if let Some(quantity) = [lhs_ty, rhs_ty].into_iter().find_map(TyId::quantity) {
            self.coerce(lhs, TyId::INT);
            self.coerce(rhs, TyId::INT);
            if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
                return TyId::ERROR;
            }
            let message = format!(
                "cannot {} {} and {}",
                op.verb(),
                self.in_message(lhs_ty),
                self.in_message(rhs_ty)
            );
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            let name = quantity.type_name();
            if let Some(error) = self.errors.last_mut() {
                error.notes.push(format!(
                    "a {name} divides with `//` by an Int, giving a {name}, or by a {name}, \
                     giving an Int, and `%` gives the remainder of one {name} by another"
                ));
            }
            return TyId::ERROR;
        }
        // Whatever the outcome, untyped operands settle to Int: these
        // operators never accept Float, so nothing else could decide them.
        self.coerce(lhs, TyId::INT);
        self.coerce(rhs, TyId::INT);
        if lhs_ty == TyId::ERROR || rhs_ty == TyId::ERROR {
            return TyId::ERROR;
        }
        let rejected = [lhs_ty, rhs_ty]
            .into_iter()
            .find(|&ty| !matches!(ty, TyId::INT | TyId::UNTYPED_INT));
        if let Some(rejected) = rejected {
            let message = format!("cannot {} {}", op.verb(), self.in_message(rejected));
            self.operand_error(id, message, (lhs, lhs_ty), (rhs, rhs_ty));
            return TyId::ERROR;
        }
        TyId::INT
    }

    /// Gives an untyped expression its default type, `Int`, where it is
    /// bound or discarded rather than used in a typed context. Returns the
    /// resulting type.
    fn settle(&mut self, id: ExprId, ty: TyId) -> TyId {
        if ty == TyId::UNTYPED_INT {
            self.coerce(id, TyId::INT);
            TyId::INT
        } else {
            ty
        }
    }

    /// Pushes the concrete type `ty` down into the untyped subtree rooted at
    /// `id`. Does nothing if `id` already has a concrete type (or is an
    /// error).
    ///
    /// Only integer literals, parentheses, negation, `await` of a present
    /// value, `+ - *`, blocks and `if`s with `else` can be untyped, and an
    /// untyped node's children
    /// (an `if`'s branches, not its condition) are untyped or concrete of
    /// the same kind, so the walk only ever meets those nodes. Every other
    /// node always has a concrete type; a record block, in particular, is
    /// always a record.
    fn coerce(&mut self, id: ExprId, ty: TyId) {
        if self.expr_types[id.index()] != TyId::UNTYPED_INT {
            return;
        }
        self.expr_types[id.index()] = ty;
        match self.ast.get(id).kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Duration(_)
            | ExprKind::Size(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Interp(_)
            | ExprKind::Name(_)
            | ExprKind::Field { .. }
            | ExprKind::Ascribe { .. }
            | ExprKind::Is { .. }
            | ExprKind::As { .. }
            | ExprKind::Fn(_)
            | ExprKind::Call { .. }
            | ExprKind::Lazy(_)
            | ExprKind::List(_)
            | ExprKind::Index { .. }
            | ExprKind::For(_)
            | ExprKind::DefTypeRef(_)
            | ExprKind::Error
            | ExprKind::NoField(_)
            | ExprKind::Unclosed { .. } => {}
            // An `await` of an untyped operand is one of a present value,
            // which it passes through.
            ExprKind::Paren(inner) | ExprKind::Unary { operand: inner, .. } => {
                self.coerce(inner, ty);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.coerce(lhs, ty);
                self.coerce(rhs, ty);
            }
            ExprKind::If { then, else_, .. } => {
                self.coerce(then, ty);
                self.coerce(else_, ty);
            }
            ExprKind::Block { items, label } => {
                self.coerce_result(items, ty);
                // A labelled block's `return`s are its result sites too.
                if label.is_some()
                    && let Some(values) = self.block_returns.get(&id).cloned()
                {
                    for value in values {
                        self.coerce(value, ty);
                    }
                }
            }
        }
    }

    /// Coerces the result expression of `body`, if it has one, to `ty`. (In
    /// a record block the final expression statement is not a result, but
    /// it has been settled already, so coercing it does nothing.)
    fn coerce_result(&mut self, body: ItemRange, ty: TyId) {
        let Some(&last) = self.ast.items(body).last() else {
            return;
        };
        match self.ast.item(last).kind {
            ItemKind::Result(value) | ItemKind::Expr(value) => self.coerce(value, ty),
            ItemKind::Let { .. }
            | ItemKind::Rebind { .. }
            | ItemKind::Field { .. }
            | ItemKind::Spread(_)
            | ItemKind::Break(_)
            | ItemKind::Continue(_)
            | ItemKind::Return(_)
            | ItemKind::Import(_)
            | ItemKind::Type { .. }
            | ItemKind::DefTypeDef { .. }
            | ItemKind::DefMeta { .. }
            | ItemKind::Error => {}
        }
    }

    /// Type `ty` as a diagnostic's message or notes show it (see
    /// [`Checker::shown`]): plainly if it is written as a name, and in
    /// backticks if it is structural, as a record, list, function, future
    /// or union type is.
    fn in_message(&mut self, ty: TyId) -> String {
        let ty = self.shown(ty);
        let shown = self.types.display(ty, self.ast.names());
        match self.types.kind(ty) {
            TyKind::Record(_)
            | TyKind::List(_)
            | TyKind::Fn(_)
            | TyKind::Future(_)
            | TyKind::Either(_)
            | TyKind::Union(_)
            | TyKind::Provisional { .. }
            // TEMPORARY (alias rule): see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/).
            | TyKind::Builtin(_) => {
                format!("`{shown}`")
            }
            TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never => shown.to_string(),
        }
    }

    /// Type `ty` as a diagnostic's labels show it (see
    /// [`Checker::shown`]): always plainly.
    fn in_label(&mut self, ty: TyId) -> String {
        let ty = self.shown(ty);
        self.types.display(ty, self.ast.names()).to_string()
    }

    /// Type `ty` as a diagnostic shows it, with no provisional type in it,
    /// since it is shown before the parameters are decided: each of a
    /// claimable function's (see [`Checker::claimable`]) as what its
    /// parameter is if nothing more measures it, present if the body has
    /// measured it so far and the either shape otherwise, and each of any
    /// other enclosing function's as the either shape, which is what the
    /// body being checked treats it as.
    fn shown(&mut self, ty: TyId) -> TyId {
        if !self.types.contains_provisional(ty, None) {
            return ty;
        }
        let mut ty = ty;
        let claimable = self.claimable();
        for (index, function) in self.functions.iter().enumerate() {
            if function.demanded.is_empty() {
                continue;
            }
            let decisions: Vec<bool> = if claimable.contains(&index) {
                function.demanded.iter().map(Option::is_some).collect()
            } else {
                vec![false; function.demanded.len()]
            };
            ty = self.types.settle(ty, function.fn_id, &decisions);
        }
        ty
    }
}

/// The label on a function's declared result type where a value does not
/// fit it: the body's result, or a `return`'s value.
const RESULT_WHY: &str = "the return type is declared here";

/// What `Checker::filled` holds for a parameter a call gives its default
/// (see [Calls](#calls)): no argument's index, and not `u32::MAX`, which is
/// a parameter left unfilled.
const DEFAULTED: u32 = u32::MAX - 1;

/// The note on [`FIELD_NOT_CONSTANT`]: what a
/// constant is.
const CONSTANT_NOTE: &str = "a literal, or a list or record of literals";

/// The error for a field's default of a record type that is not a constant,
/// at the default (see [Field defaults](#field-defaults)).
const FIELD_NOT_CONSTANT: &str = "a field's default must be a constant";

/// The constant that `id` of `ast`, a parameter's or a field's default that
/// [`is_constant`] accepts, is, `types` being the type of each expression,
/// as the check found it, and `fills` the fields each record literal was
/// given ([`Resolution::record_fills`]): an integer literal that is a
/// `Float` there a `Float`, and an integer otherwise (an untyped one
/// settles to `Int`, and one of no type known is an `Int`); a record block
/// the record of its fields in the order they are written and then of
/// those it was given; and `{}` the record of the fields it was given, if
/// any, and the empty record otherwise. `None` if it is not one after all.
///
/// Recurses once per level of list and record nesting.
fn constant_of(ast: &Ast, id: ExprId, (types, fills): (&[TyId], &RecordFills)) -> Option<Constant> {
    let float = |id: ExprId| types.get(id.index()) == Some(&TyId::FLOAT);
    Some(match ast.get(id).kind {
        ExprKind::Int(value) if float(id) => Constant::Float(int_to_float(value).to_bits()),
        ExprKind::Int(value) => Constant::Int(value),
        ExprKind::Float(value) => Constant::Float(value.to_bits()),
        ExprKind::Duration(amount) => Constant::Quantity(Quantity::Duration, amount),
        ExprKind::Size(amount) => Constant::Quantity(Quantity::Size, amount),
        ExprKind::Bool(value) => Constant::Bool(value),
        ExprKind::Str(text) => Constant::Str(ast.strings().get(text).into()),
        // Negated as lowering negates it: as the whole is typed.
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => match ast.get(operand).kind {
            ExprKind::Int(value) if float(id) => Constant::Float((-int_to_float(value)).to_bits()),
            ExprKind::Int(value) => Constant::Int(value.checked_neg()?),
            ExprKind::Float(value) => Constant::Float((-value).to_bits()),
            ExprKind::Duration(amount) => {
                Constant::Quantity(Quantity::Duration, amount.checked_neg()?)
            }
            ExprKind::Size(amount) => Constant::Quantity(Quantity::Size, amount.checked_neg()?),
            _ => return None,
        },
        ExprKind::List(elements) => Constant::List(
            ast.elements(elements)
                .iter()
                .map(|element| constant_of(ast, element.value, (types, fills)))
                .collect::<Option<_>>()?,
        ),
        // `{}` given fields is the record of them.
        ExprKind::Block { items, label: None } if ast.items(items).is_empty() => {
            match fills.get(&id) {
                Some(filled) => Constant::Record(filled.clone()),
                None => Constant::Empty,
            }
        }
        ExprKind::Block { items, label: None } => {
            let mut fields = ast
                .items(items)
                .iter()
                .map(|&item| match ast.item(item).kind {
                    ItemKind::Field { path, value, .. } => {
                        let name = ast.path(path).first()?.name()?;
                        Some((name, constant_of(ast, value, (types, fills))?))
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            if let Some(filled) = fills.get(&id) {
                fields.extend(filled.iter().cloned());
            }
            Constant::Record(fields.into_boxed_slice())
        }
        _ => return None,
    })
}

/// The constant that `id` of `ast`, a field's default of a record type
/// copied out of its tree into a [`crate::ast::TypeStore`], is written as,
/// with no type known and nothing filled ([`constant_of`]), if
/// [`is_constant`] accepts it; `None` if it is not a constant, which the
/// check of its own tree reports (see [Field defaults](#field-defaults)).
/// The checker of each tree using the type fits it to its field's type
/// (`Checker::fit_constant`).
pub(crate) fn written_constant(ast: &Ast, id: ExprId) -> Option<Constant> {
    if !crate::ast::is_constant(ast, id) {
        return None;
    }
    constant_of(ast, id, (&[], &HashMap::new()))
}

/// The span of the keyword `keyword` of a `break`, `continue` or `return`
/// item spanning `item`, which it starts, through its label, if it has one.
fn jump_span(keyword: &str, item: Span, label: Option<Label>) -> Span {
    let written = symbol_span(keyword, item.start);
    label.map_or(written, |label| Span::join(written, label.span))
}

/// Whether `ty` is a numeric type, untyped integers included.
fn is_numeric(ty: TyId) -> bool {
    matches!(ty, TyId::INT | TyId::FLOAT | TyId::UNTYPED_INT)
}

/// Whether `ty` is a quantity, `Duration` or `Size`.
fn is_quantity(ty: TyId) -> bool {
    matches!(ty, TyId::DURATION | TyId::SIZE)
}

/// Whether `ty` is an `Int`, an untyped integer being one.
fn is_int(ty: TyId) -> bool {
    matches!(ty, TyId::INT | TyId::UNTYPED_INT)
}

/// A field's position in its block or type, as stored in
/// `Checker::sorted_fields`, a node's index in `Checker::nodes`, or a count
/// of path segments. There are fewer of each than the source has bytes, so
/// the conversion never saturates.
fn position(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// The label, at the path of the dotted `:=` whose path is `segments`, on
/// the type its value must fit: the type of its last field, or of the
/// elements of the list its last segment indexes.
fn leaf_label(segments: &[PathSegment]) -> &'static str {
    match segments.last().map(|segment| segment.kind) {
        Some(SegmentKind::Index { .. }) => "`:=` keeps the element's type",
        _ => "`:=` keeps the field's type",
    }
}

/// Which of the `count` parameters of a callee the arguments `args` fill,
/// by position: the positional arguments the first, a trailing block the
/// last, and each named argument the parameter `param_named` finds by its
/// name. What a suggestion for an unknown argument leaves out, a parameter
/// it would give twice.
fn filled_by(
    args: &[crate::ast::Arg],
    count: usize,
    param_named: impl Fn(Name) -> Option<usize>,
) -> Vec<bool> {
    let mut filled = vec![false; count];
    let positional = args
        .iter()
        .filter(|arg| arg.kind == ArgKind::Positional)
        .count();
    for slot in filled.iter_mut().take(positional) {
        *slot = true;
    }
    if positional < count
        && args.iter().any(|arg| arg.kind == ArgKind::Trailing)
        && let Some(last) = filled.last_mut()
    {
        *last = true;
    }
    for arg in args {
        if let ArgKind::Named { name, .. } = arg.kind
            && let Some(slot) = param_named(name).and_then(|at| filled.get_mut(at))
        {
            *slot = true;
        }
    }
    filled
}

/// The error of the named argument `arg`, of a call of `ast` whose
/// arguments are `args`, that no parameter of the built-in function, named
/// `params`, has: "unknown argument `x`", labelled `label` at the callee,
/// at `callee`, suggesting the nearest of the parameters that no other
/// argument fills ([`unknown_argument`]).
fn unknown_builtin_argument(
    ast: &Ast,
    args: &[crate::ast::Arg],
    params: &[&str],
    (arg, (callee, label)): (crate::ast::Arg, (Span, &str)),
) -> Diagnostic {
    let ArgKind::Named {
        name,
        span,
        shorthand,
    } = arg.kind
    else {
        // Only a named argument can name no parameter: kept for safety.
        return Diagnostic::error("unknown argument", arg.span(ast));
    };
    let text = ast.names().get(name);
    let filled = filled_by(args, params.len(), |name| {
        let text = ast.names().get(name);
        params.iter().position(|&param| param == text)
    });
    let free = params
        .iter()
        .zip(&filled)
        .filter(|&(_, &filled)| !filled)
        .map(|(&param, _)| (param, 0));
    unknown_argument((text, span, shorthand), free).with_context(callee, label)
}

/// The error of the named argument `text`, written at `span`, the shorthand
/// `text` alone if `shorthand`, that no parameter has: "unknown argument
/// `x`", suggesting the nearest of `free`, the named parameters no other
/// argument fills (see [Suggestions](#suggestions)). For the shorthand, which
/// reads the variable of its name too, the suggestion names the parameter
/// and keeps the variable, `best = x`, and a note says what the shorthand
/// is.
fn unknown_argument<'a>(
    (text, span, shorthand): (&str, Span, bool),
    free: impl Iterator<Item = (&'a str, u8)>,
) -> Diagnostic {
    let best = suggest::best(text, free);
    let error = Diagnostic::error(format!("unknown argument `{text}`"), span);
    if shorthand {
        let suggestion = best.map(|best| (span, format!("{best} = {text}")));
        error
            .with_note(format!(
                "`{text}` alone after a named argument is shorthand for `{text} = {text}`"
            ))
            .with_suggestion(suggestion)
    } else {
        error.with_suggestion(best.map(|best| (span, best.to_owned())))
    }
}

/// The error for a `:=`, whose name `text` is at `name_span`, of
/// `binding`, the copy of a variable that a branch of an `if NAME is TYPE`
/// sees: "cannot rebind `x` here: it is narrowed by `is`", labelled at the
/// test. Rebinding the copy would not change the variable it was copied
/// from, a mutable one included, so the note says how to keep the variable
/// free to rebind.
fn narrowed_rebind_error(text: &str, name_span: Span, binding: Binding) -> Diagnostic {
    let mut error = Diagnostic::error(
        format!("cannot rebind `{text}` here: it is narrowed by `is`"),
        name_span,
    );
    if let Some(declared) = binding.declared {
        error = error.with_context(declared, "narrowed here");
    }
    error.with_note(format!(
        "in this branch `{text}` is a copy of the value that was tested, so rebinding it would \
         not change the variable; test a copy under another name instead, as in `let v = \
         {text}` and `if v is ...`, which leaves `{text}` free to rebind"
    ))
}

/// The note on a dotted path under a field, or a segment, defined whole.
const WHOLE_FIELD_NOTE: &str = "a field defined as a whole cannot be extended with dotted paths";

/// The note on a def-type reference whose def-type is undefined or no
/// def-type: two names in a row that may have meant something else (see
/// [Def-types](#def-types)).
const JUXTAPOSED_NOTE: &str = "two names in a row are read as a def-type reference, as in \
     `package dev.x()`; is an operator or a line break missing?";

/// The note on a field that the expected record type lacks.
const EXTRA_FIELD_NOTE: &str = "it is allowed, but nothing with this type can see it";

/// The note on a read of a field that a record type's rest-field covers.
const REST_READ_NOTE: &str =
    "a field that only the rest-field `*: ...` covers cannot be read by name";

/// The note on a spread of a value whose type has a rest-field.
const SPREAD_REST_NOTE: &str = "a spread defines the fields its operand's type names, and the fields a rest-field covers are known only when it runs";

/// The note on a field that dotted paths build where a record type with a
/// rest-field is expected of it.
const DOTTED_REST_NOTE: &str =
    "dotted paths build a record without a rest-field; define the field whole, as in `a = {...}`";

/// The label on a pure function's marker, where it does something only an
/// impure function may.
const PURE_CONTEXT: &str = "this function is pure, declared with `:`";

/// The note on a use of the `fileroot` module other than a read of one of
/// its members.
pub(crate) const FILEROOT_MEMBERS: &str =
    "`fileroot` has the members `this`, `workspace` and `caller`, each a String, and `helper()`";

/// The note on "field `x` is defined twice" for a spread, of a value or of
/// a type, with a field of the name of one defined before it and `others`
/// more such fields.
fn spread_first_note(others: usize) -> String {
    let also = match others {
        0 => String::new(),
        1 => " (it also has 1 other field defined before it)".to_owned(),
        others => format!(" (it also has {others} other fields defined before it)"),
    };
    format!("a spread cannot replace a field defined before it; put the spread first{also}")
}

/// The error for a record type that has the field `name`, or the
/// rest-field for `None`, more than once: "field `x` appears twice in this
/// type", "the rest-field appears twice in this type".
fn appears_twice(names: &Names, name: Option<Name>) -> String {
    match name {
        Some(name) => format!(
            "field `{}` appears twice in this type",
            FieldName(names.get(name))
        ),
        None => "the rest-field appears twice in this type".to_owned(),
    }
}

/// The error for an explicit `await`, whose keyword is at `keyword`, in a
/// pure function whose marker is at `marker`.
fn explicit_await_error(keyword: Span, marker: Span) -> Diagnostic {
    Diagnostic::error("cannot await in a pure function", keyword)
        .with_context(marker, PURE_CONTEXT)
        .with_note(
            "awaiting runs a deferred expression, which may have effects; declare the function \
             with `->` to make it impure",
        )
}

#[cfg(test)]
mod tests;
