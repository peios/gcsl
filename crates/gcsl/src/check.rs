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
//! This module holds the checker's state, its results and its driver, the
//! walk of a file's body and of each expression. The checking of each part
//! of the language is a submodule of its own, documented there:
//!
//! - Items and declarations: [`items`], [`records`], [`mutation`],
//!   [`scopes`] and [`resolve`], the type expressions.
//! - The top level: [`toplevel`], [`imports`], [`schedule`] and
//!   [`deftypes`].
//! - Expressions: [`expect`], [`operators`], [`names`], [`fields`],
//!   [`lists`], [`loops`] and [`narrowing`].
//! - Functions and calls: [`functions`], [`calls`], [`methods`],
//!   [`inference`], [`futures`] and [`attributes`].
//! - The rest of the language: [`enums`], [`context`], [`failure`] and
//!   [`secrets`].
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
//! error (see [Unions](crate::check::narrowing#unions)). An `if` without `else` is `{}`, and so
//! must its block be, or `Never` (see [Never](crate::check::narrowing#never)). There is no
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
//! [Type declarations](crate::check::resolve#type-declarations)). A declared type is a
//! STRUCTURAL ALIAS: it is its definition, not a new type, so two aliases of
//! one record type are one type, a record of the right fields is a
//! `PackageDef` with no ceremony, and a type is shown as what it stands for,
//! never by the name of an alias of it: an error about a `PackageDef` shows
//! the record type.
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
//! functions](crate::check::toplevel#top-level-functions)); between inputs, order is as it was: a
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
//! the view, as the check does (see [Imports](crate::check::imports#imports)), and so a read of
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

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{
    Arg, ArgKind, Ast, DefTypeId, Effect, ExprId, ExprKind, FnId, ForId, ItemId, ItemKind,
    ItemRange, Label, PathSegment, SegmentKind, Syntax, TypeExprId, UnaryOp, symbol_span,
};
use crate::diagnostic::{self, Diagnostic, InternalError};
use crate::mentions::{Mentions, Schedule};
use crate::names::{Name, Names};
use crate::source::Span;
use crate::strings::FieldName;
use crate::suggest;
use crate::types::{DefTypeShape, FnTy, GenericId, TyId, TyKind, TypeTable};
use crate::value::{Quantity, int_to_float};
use crate::workspace::{Context, Target};

pub use builtin::{Builtin, Typing};
use builtin::builtin_types;
pub use results::{
    Attribute, CheckOutput, Checked, Constant, FieldSchedule, Fileroot, MadeAt, ParamDefault,
    Place, Slot, TypeAnnotation, Types, VarRef, VariantTag,
};
use results::OtherTypes;
pub use resolution::{
    AwaitMark, FnResolution, JumpTarget, LoopResolution, NarrowingBlock, PathResolution,
    Resolution, TestResolution, Unnarrowed,
};
pub(crate) use resolution::KnownParams;
use resolution::{CallsChecked, ForLowering, merged_defaults};

/// What a call of a generic function has bound of its type parameters so
/// far, as [`Checker::instantiate`] binds them.
struct TypeBinding {
    /// What the type parameters belong to.
    generic: GenericId,
    /// Each type parameter's binding, by its position, once one is made.
    bound: Vec<Option<TyId>>,
    /// The argument that bound each first.
    binders: Vec<Option<ExprId>>,
}

/// What a def-type gives one of its uses (see [Def-types](crate::check::deftypes#def-types),
/// `Checker::use_signature`): the function type of its definitions, its
/// signature, with what the use's variant binds bound, where its parameters
/// are, and the use's variant, if its definitions are made with one.
struct UseSignature {
    definition: TyId,
    sig: FnTy,
    shape: DefTypeShape,
    tag: Option<VariantTag>,
}

/// Where a method's name is declared, in the input checked (`None`) or
/// another, if that is known (see [Methods](crate::check::methods#methods)).
pub(crate) type MethodSite = Option<(Option<u32>, Span)>;

/// The names of the built-in types applied to type arguments, each if the
/// program's table holds it.
#[derive(Clone, Copy, Debug)]
struct Constructors {
    /// `Future`, which applied to a type is a future of it.
    future: Option<Name>,
    /// `Present`, which applied to a type is it with every bare type in it
    /// present.
    present: Option<Name>,
    /// `List`, which applied to a type is a list of it.
    list: Option<Name>,
    /// `Variant`, for `Variant<E, T>` (see [Enums](crate::check::enums#enums)).
    variant: Option<Name>,
    /// `Partial`, for `Partial<T>` (see [Partial records](crate::check::enums#partial-records)).
    partial: Option<Name>,
    /// `Secret`, for `Secret<T>` (see [Secrets](crate::check::secrets#secrets)).
    secret: Option<Name>,
    /// `Revealed`, for `Revealed<T>` (see [Secrets](crate::check::secrets#secrets)).
    revealed: Option<Name>,
}

impl Constructors {
    /// The names `names` holds.
    fn of(names: &Names) -> Self {
        Self {
            future: names.lookup("Future"),
            present: names.lookup("Present"),
            list: names.lookup("List"),
            variant: names.lookup("Variant"),
            partial: names.lookup("Partial"),
            secret: names.lookup("Secret"),
            revealed: names.lookup("Revealed"),
        }
    }
}

/// A method a call of its name may mean (see [Methods](crate::check::methods#methods)).
#[derive(Clone, Copy, Debug)]
struct MethodCandidate {
    /// How its closure is read.
    source: MethodSource,
    /// Its function's type, the receiver its first parameter.
    ty: TyId,
    /// Where its name is declared, in this input (`None`) or another, if
    /// it is known.
    declared: MethodSite,
}

/// How a method's closure is read (see [Methods](crate::check::methods#methods)).
#[derive(Clone, Copy, Debug)]
enum MethodSource {
    /// From its binding, of this input or an earlier one of a session, as a
    /// top-level function's is.
    Binding(Name),
    /// From its slot, as another file's: by its name and its position among
    /// those of the name the file sees ([`Context::methods`]).
    Foreign(Name, u32),
}

/// How a call's arguments bind to its callee's parameters beyond what the
/// call shows (see [`Checker::bind_call`]): the callee, if it is known, and
/// how many of its first parameters the call fills apart from its argument
/// list, a method's receiver.
#[derive(Clone, Copy, Debug)]
struct Bound {
    /// The callee, if it is known.
    known: Option<KnownCallee>,
    /// The number of the parameter the signature bound starts at.
    offset: usize,
}

/// Each record literal given fields' defaults, by its id, with each such
/// field and its default: see [`Resolution::record_fills`].
type RecordFills = HashMap<ExprId, Box<[(Name, Constant)]>>;

/// For each node of a workspace file's tree, the children that other
/// files declare under it, each with its name, cell and type (see
/// [Directory scope](crate::check::imports#directory-scope)).
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

/// The field of a `recover`'s failure, `{message: String}` (see
/// [Failure](crate::check::failure#failure)).
pub(crate) const FAILURE_MESSAGE: &str = "message";

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
    /// defaults a call of it gives (see "Calls" under
    /// [Functions](crate::check::functions#functions)); `None` for every other
    /// binding, and for one declared by an earlier input of a session, whose
    /// [`FnId`]s belong to another tree.
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
    /// Each call of a method (see [Methods](crate::check::methods#methods)), by its callee, with
    /// where the method's name is declared, in this input (`None`) or
    /// another, if that is known.
    pub(crate) methods: Vec<(ExprId, MethodSite)>,
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
    /// in the body may (see [For](crate::check::loops#for)).
    Header,
    /// The copy of a variable that a branch of `if NAME is TYPE` sees, of
    /// the narrowed type or the remainder, which `:=` may not rebind: the
    /// rebind would change the copy, not the variable it was copied from.
    Narrowed,
}

/// How the checker resolves the copies of declared types' definitions in a
/// file (see [Type declarations](crate::check::resolve#type-declarations)).
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

/// What the checker knows of one node of the tree of the input's top-level
/// fields (see [Top-level fields](crate::check::toplevel#top-level-fields)), or, in a workspace,
/// of one node of the file's view numbered after them (see
/// [Imports](crate::check::imports#imports)).
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
    /// cycle (see [Top-level fields](crate::check::toplevel#top-level-fields)).
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
/// functions](crate::check::toplevel#top-level-functions)).
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
    /// binding (see [Top-level functions](crate::check::toplevel#top-level-functions)).
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
    /// used](crate::check::schedule#when-a-function-may-be-used)).
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
    /// [`return`](crate::check::loops#return)).
    returns: Vec<Site>,
}

/// What the checker keeps while default functions are checked (see
/// "Calls" under [Functions](crate::check::functions#functions)).
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
/// [`return`](crate::check::loops#return)).
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
/// defaults (see "Calls" under [Functions](crate::check::functions#functions)).
#[derive(Clone, Copy, Debug)]
enum KnownCallee {
    /// A function expression of the input being checked.
    Current(FnId),
    /// A binding of an earlier session input, by its index in
    /// `Checker::bindings`, which is also its index in
    /// `Checker::scope_measured`.
    Earlier(usize),
    /// A `def` of another file reached through an import, by its merged
    /// node in the workspace (see [Imports](crate::check::imports#imports)), whose measured
    /// parameters the workspace context carries.
    Imported(u32),
    /// A method of another file, by its name and its position among those
    /// of the name the file sees ([`Context::methods`]).
    Method(Name, u32),
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
/// (see [Def-types](crate::check::deftypes#def-types)).
#[derive(Clone, Debug, PartialEq, Eq)]
enum DefinitionAt {
    /// A definition.
    Leaf,
    /// A definition at a prefix of the path, whose read is this prefix of
    /// the reference's read, the rest of the path being read from its meta
    /// (see [Metas](crate::check::attributes#metas)): only for a def-type that declares one.
    Past(ExprId),
    /// A node above definitions, no definition itself.
    Interior,
    /// Nothing: with the paths of the definitions of the namespace the file
    /// sees, as written after the def-type.
    Missing(Vec<String>),
}

/// What a read of a meta reads it of, as its errors show it (see
/// [Metas](crate::check::attributes#metas)); `Checker::meta_of` says what each is.
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
    /// lacks are recorded as given their defaults (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)).
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

/// A loop whose body, or a labelled block whose items, are being checked, in
/// `Checker::loops`, innermost last: what a `break`, `continue` or `return` in
/// it needs to find it, and what they give it (see [`break` and
/// `continue`](crate::check::loops#break-and-continue),
/// [Labels](crate::check::loops#labels) and
/// [`return`](crate::check::loops#return)). An unlabelled block has none.
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
    /// to end at all (see [Never-ending loops](crate::check::loops#never-ending-loops)).
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
    /// of the names it spells (see [Directory scope](crate::check::imports#directory-scope)).
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
    /// declarations](crate::check::items#shadowed-declarations)).
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

    /// The slot, the type and what a call of its function needs to know of
    /// its parameters of the latest binding of `name`, if there is one: a
    /// method's, which another file calls (see [Methods](crate::check::methods#methods)).
    pub(crate) fn function_binding(
        &self,
        name: Name,
    ) -> Option<(Slot, TyId, Option<Rc<KnownParams>>)> {
        let (binding, known) = self
            .bindings
            .iter()
            .zip(&self.known)
            .rev()
            .find(|(binding, _)| binding.name == name)?;
        Some((binding.slot, binding.ty, known.clone()))
    }

    /// For each slot bound, what a call of the function its latest binding
    /// holds needs to know of its parameters, if that binding was
    /// initialised directly with a function expression: what a call of an
    /// imported `def` needs (see [Imports](crate::check::imports#imports)). One map for every
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
/// [Type declarations](crate::check::resolve#type-declarations)). Returns the errors, none for a
/// file with no `type` item. If the file's view `sees_broken`, a file with
/// syntax errors, an unknown type is none of them (see
/// [`Checker::unresolved`]).
pub(crate) fn check_types(ast: &Ast, types: &mut TypeTable, sees_broken: bool) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    checker.quiet.sees_broken = sees_broken;
    checker.type_items();
    checker.errors
}

/// Resolves the bounds of the enums of `ast`, each by its item and its key
/// (see [Enums](crate::check::enums#enums)), interning into `types`, which records each: one
/// erroneous is no bound, its error reported. Returns the errors found.
pub(crate) fn check_enum_bounds(
    ast: &Ast,
    types: &mut TypeTable,
    enums: &[(ItemId, Name)],
) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    for &(item, key) in enums {
        let ItemKind::Enum { bound, .. } = ast.item(item).kind else {
            continue;
        };
        let bound = bound
            .map(|bound| checker.resolve_type(bound, Bareness::Present))
            .filter(|&bound| bound != TyId::ERROR);
        checker.types.set_enum_bound(key, bound);
    }
    checker.errors
}

/// Resolves the declared types of the contexts of `ast`, each by its
/// position among the file's contexts and its declaration's key (see
/// [Context](crate::check::context#context)), interning into `types`, which records each, `Error`
/// for one in error, its error reported. Returns the errors found.
pub(crate) fn check_context_types(
    ast: &Ast,
    types: &mut TypeTable,
    contexts: &[(u32, Name)],
) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    for &(decl, key) in contexts {
        let Some(context) = ast.contexts().get(decl as usize) else {
            continue;
        };
        let ty = checker.resolve_type(context.ty, Bareness::Present);
        checker.types.set_context_type(key, ty);
    }
    checker.errors
}

/// A variant's item, its enum's key and name, if what it names is an enum,
/// and its own key (see [Enums](crate::check::enums#enums)).
pub(crate) type VariantItem = (ItemId, Option<(Name, Name)>, Name);

/// Resolves the payloads of the variants of `ast`, each by its item, its
/// enum's key and name, if what it names is an enum, and its own key (see
/// [Enums](crate::check::enums#enums)), every enum's bound being known: each payload must fit
/// its enum's bound, "the payload of `Toolchain::C` must fit `{*: File |
/// Dir}`"; a variant of an enum one earlier of the file has the name of is
/// "variant `Toolchain::C` is declared twice", and a variant of what is no
/// enum "`Int` is not an enum". Records each payload, `Error` for one in
/// error, in `types`. Returns the errors found.
pub(crate) fn check_variant_payloads(
    ast: &Ast,
    types: &mut TypeTable,
    variants: &[VariantItem],
) -> Vec<Diagnostic> {
    let mut checker = Checker::new(ast, types);
    let mut declared: Vec<(Name, Name, Span)> = Vec::new();
    for &(item, enum_of, key) in variants {
        let ItemKind::Variant {
            of,
            name,
            name_span,
            payload,
        } = ast.item(item).kind
        else {
            continue;
        };
        let Some((enum_key, enum_name)) = enum_of else {
            let ty = checker.resolve_type(of, Bareness::Present);
            if ty != TyId::ERROR {
                let shown = checker.in_message(ty);
                checker.errors.push(Diagnostic::error(
                    format!("{shown} is not an enum"),
                    ast.type_expr(of).span,
                ));
            }
            continue;
        };
        let names = ast.names();
        let shown = format!("{}::{}", names.get(enum_name), names.get(name));
        if let Some(&(_, _, first)) = declared
            .iter()
            .find(|&&(of, earlier, _)| of == enum_key && earlier == name)
        {
            checker.errors.push(
                Diagnostic::error(format!("variant `{shown}` is declared twice"), name_span)
                    .with_context(first, "first declared here"),
            );
            continue;
        }
        declared.push((enum_key, name, name_span));
        let ty = match payload {
            Some(payload) => checker.resolve_type(payload, Bareness::Present),
            None => TyId::EMPTY,
        };
        let ty = match checker.types.enum_bound(enum_key) {
            Some(bound) if ty != TyId::ERROR && !checker.types.fits_bound(ty, bound) => {
                let (found, bound_shown) = (checker.in_message(ty), checker.in_message(bound));
                let at = payload.map_or(name_span, |payload| ast.type_expr(payload).span);
                checker.errors.push(
                    Diagnostic::error(
                        format!("the payload of `{shown}` must fit {bound_shown}"),
                        at,
                    )
                    .with_label(format!("this is {found}"))
                    .with_note(format!(
                        "`{}` bounds its variants' payloads as it is declared",
                        names.get(enum_name)
                    )),
                );
                TyId::ERROR
            }
            _ => ty,
        };
        checker.types.set_variant_payload(key, ty);
    }
    checker.errors
}

/// The names of the built-in types: those of [`BuiltinTypes`], and the
/// type constructors, `List`, `Future` and `Present`. A `type` may not be
/// named like one: "`Int` is a built-in type".
pub(crate) const BUILTIN_TYPE_NAMES: [&str; 16] = [
    "Int", "Float", "Bool", "String", "Duration", "Size", "File", "Dir", "Sandbox", "List",
    "Future", "Present", "Variant", "Partial", "Secret", "Revealed",
];

/// The name of the checker's attribute that retries a call.
const RETRY: &str = "retry";

/// The name of the checker's attribute that limits a call's time.
const TIMEOUT: &str = "timeout";

/// The name of the checker's attribute that reports the errors raised in a
/// function's body at its call.
pub(crate) const TRACK_CALLER: &str = "track_caller";

/// The error for an `#[attribute]` type that is no record of constants.
const NOT_CONSTANT_RECORD: &str = "an attribute's arguments are a record of constants";

/// The note on [`NOT_CONSTANT_RECORD`].
const CONSTANT_FIELDS_NOTE: &str = "each field must be an `Int`, `Float`, `Bool`, `String`, \
     `Duration` or `Size`, or a list or record of them";

/// The fields of an attribute's record, by name, in its order, each with
/// its type and default, if it has one (see [Attributes](crate::check::attributes#attributes)).
type AttributeFields = Vec<(Box<str>, TyId, Option<Constant>)>;

/// The arguments of `args`, by their values, each with the parameter of
/// `sig` it fills and whether it is a spread, as [`Checker::bind_arguments`]
/// binds them: positional ones left to right, those from the rest
/// parameter's position on and every spread the rest parameter, the
/// trailing block the last, and named ones the parameters of their names;
/// only the first that fills a parameter but the rest parameter, and none
/// that fills nothing.
fn argument_targets(args: &[Arg], sig: &FnTy) -> Vec<(ExprId, usize, bool)> {
    let count = sig.params.len();
    let rest = sig.rest.map(usize::from);
    let positional = args
        .iter()
        .filter(|arg| arg.kind == ArgKind::Positional)
        .count();
    let mut next = 0;
    let mut taken = vec![false; count];
    let mut targets = Vec::new();
    for arg in args {
        let param = match arg.kind {
            ArgKind::Positional => {
                next += 1;
                sig.positional(next - 1)
            }
            ArgKind::Spread { .. } => rest,
            ArgKind::Trailing => (positional < count).then(|| count - 1),
            ArgKind::Named { name, .. } => sig
                .params
                .iter()
                .position(|&(param, _)| param == Some(name))
                .filter(|&param| Some(param) != rest),
        };
        let Some(param) = param.filter(|&param| param < count) else {
            continue;
        };
        let spread = matches!(arg.kind, ArgKind::Spread { .. });
        if Some(param) == rest {
            targets.push((arg.value, param, spread));
        } else if let Some(slot @ false) = taken.get_mut(param) {
            *slot = true;
            targets.push((arg.value, param, spread));
        }
    }
    targets
}

/// The function of the item `item`, a function item that attributes apply to: a
/// named `fn`, a `def` or a definition (see
/// [Attributes](crate::check::attributes#attributes)).
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
/// and `timeout` (see [Attributes](crate::check::attributes#attributes)). No `#[attribute]` type
/// may be named like one: "`retry` is a built-in attribute".
pub(crate) const BUILTIN_ATTRIBUTES: [&str; 7] = [
    "shadowable",
    "def_type",
    "attribute",
    "no_prelude",
    "retry",
    "timeout",
    "track_caller",
];

/// The type names every program can use, as `(name, type)` pairs. A name
/// the program never mentions has no [`Name`].
type BuiltinTypes = [(Option<Name>, TyId); 9];

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
    /// What the check keeps of generic functions, methods and their calls.
    calls: CallsChecked,
    /// The contexts the `with`s around the expression being checked bind,
    /// by their declarations' keys, innermost last (see [Context](crate::check::context#context)).
    with_bound: Vec<Name>,
    /// What the input's top-level items mention, and the groups of its
    /// top-level functions (see [`crate::mentions`]).
    mentions: Mentions,
    /// The workspace the input is a file of, if it is one (see [Directory
    /// scope](crate::check::imports#directory-scope)).
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
    /// fields](crate::check::toplevel#top-level-fields)).
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
    /// checked, if one is (see [Top-level fields](crate::check::toplevel#top-level-fields)).
    checking_leaf: Option<u32>,
    /// The top-level functions on a cycle through fields, already reported:
    /// erroneous for their users, as an erroneous function is.
    cyclic_fns: Vec<u32>,
    /// While a top-level function's body is checked before the walk has
    /// reached it: the positions among the outermost block's items of the
    /// item being checked and of the function's declaration. A read of a
    /// name an item between them declares is not resolved (see [When each
    /// body is checked](crate::check::toplevel#top-level-functions)).
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
    /// Reused for binding a call's arguments, as `filled` is: each argument
    /// given to the rest parameter, by its index in the call's arguments,
    /// with the parameter's number.
    rest_args: Vec<(usize, usize)>,
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
    /// [Def-types](crate::check::deftypes#def-types)).
    def_fields: Vec<FieldDecl>,
    /// The definition made with a def-type each function expression of the
    /// input's outermost block is, if it is one.
    definitions: HashMap<FnId, DefTypeId>,
    /// The function expressions of the input's `def`s marked `#[def_type]`.
    def_type_fns: HashSet<FnId>,
    /// The definition's meta each function expression of the input's
    /// outermost block is, if it is one (see [Metas](crate::check::attributes#metas)).
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
    /// name the reference (see [Metas](crate::check::attributes#metas)).
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
    /// (see [Shadowed declarations](crate::check::items#shadowed-declarations)).
    shadowed_items: Vec<(ItemId, usize)>,
    /// Whether a shadowed declaration is being checked, whose reads of
    /// records it alone would need are erroneous.
    checking_shadowed: bool,
    builtin_types: BuiltinTypes,
    /// The names of the built-in types applied to type arguments.
    constructors: Constructors,
    /// TEMPORARY (alias rule): the expression that may be a reference to a built-in function whose
    /// typing is special while it is checked: the callee of the call being checked, or the
    /// initialiser of the top-level field or `let` of one name being checked (see
    /// [`Checker::builtin_use`]). Replaced when built-in functions get real polymorphic types (see
    /// the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    alias_site: Option<ExprId>,
    /// The callee of the call being checked, while it is checked: where
    /// `fileroot.helper` may stand, which is called and never a value (see
    /// [Imports](crate::check::imports#imports)).
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
    /// [Suggestions](crate::check::names#suggestions)): gathered at the first undefined
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
    /// files committed (see [Directory scope](crate::check::imports#directory-scope)).
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
            calls: CallsChecked::default(),
            with_bound: Vec::new(),
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
            rest_args: Vec::new(),
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
            constructors: Constructors::of(names),
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
        self.declare_top_level();
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
                lowered: self.lowering,
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
    /// (see [Shadowed declarations](crate::check::items#shadowed-declarations)): they are
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
    /// warned about, "this is never run" (see [Never](crate::check::narrowing#never)).
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
                ItemKind::Type { .. }
                | ItemKind::Enum { .. }
                | ItemKind::Variant { .. }
                | ItemKind::Context { .. }
                | ItemKind::DefTypeDef { .. }
                | ItemKind::DefMeta { .. } => {}
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

    /// Computes, records and returns the type of `id`, bottom-up.
    fn check_expr(&mut self, id: ExprId) -> TyId {
        let expr = self.ast.get(id);
        let ty = match expr.kind {
            ExprKind::Int(_) => TyId::UNTYPED_INT,
            ExprKind::Float(_) => TyId::FLOAT,
            ExprKind::Duration(_) => TyId::DURATION,
            ExprKind::Variant { of, name } => self.check_variant(id, (of, name)),
            ExprKind::ContextRead(name) => self.check_context_read(id, name),
            ExprKind::With { bindings, body } => self.check_with(id, bindings, body),
            ExprKind::Assert { cond, message } => self.check_assert(id, cond, message),
            ExprKind::Recover(recover) => self.check_recover(recover),
            ExprKind::Size(_) => TyId::SIZE,
            ExprKind::Bool(_) => TyId::BOOL,
            ExprKind::Str(_) => TyId::STRING,
            ExprKind::Interp(parts) => self.check_interpolation(id, parts),
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
                self.unary((id, op), op_span, (operand, operand_ty))
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
        let ty = self.called_if_generic(id, ty);
        self.expr_types[id.index()] = ty;
        ty
    }

    /// Types `dt`, the def-type of a definition, a reference or a meta, as
    /// the callee it is: a reference calls it, so a generic def-type is read
    /// as a generic function called (see [Variant
    /// def-types](crate::check::deftypes#variant-def-types)).
    fn check_def_type_expr(&mut self, dt: ExprId) -> TyId {
        let outer = self.calls.callee.replace(dt);
        let ty = self.check_expr(dt);
        self.calls.callee = outer;
        ty
    }

    /// `ty`, the type of `id`, unless it is a generic function's and `id`, a
    /// name or a field access, is no call's callee: "a generic function must
    /// be called", and `Error` (see [Generics](crate::check::methods#generics)).
    fn called_if_generic(&mut self, id: ExprId, ty: TyId) -> TyId {
        let named = matches!(
            self.ast.get(id).kind,
            ExprKind::Name(_) | ExprKind::Field { .. }
        );
        let generic = self
            .types
            .fn_sig(ty)
            .is_some_and(|sig| sig.generic.is_some());
        if !named || !generic || self.calls.callee == Some(id) {
            return ty;
        }
        self.errors.push(
            Diagnostic::error("a generic function must be called", self.ast.get(id).span)
                .with_note("each call binds its type parameters; a value of it would bind none"),
        );
        TyId::ERROR
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
    /// value, `+ - *`, blocks, `with`s, `recover`s and `if`s with `else`
    /// can be untyped, and an
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
            | ExprKind::Variant { .. }
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
            | ExprKind::ContextRead(_)
            | ExprKind::Assert { .. }
            | ExprKind::Error
            | ExprKind::NoField(_)
            | ExprKind::Unclosed { .. } => {}
            // An `await` of an untyped operand is one of a present value,
            // which it passes through, and a `with` its body.
            ExprKind::Paren(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::With { body: inner, .. } => {
                self.coerce(inner, ty);
            }
            ExprKind::Recover(id) => {
                let decl = *self.ast.recover_decl(id);
                self.coerce(decl.body, ty);
                self.coerce(decl.fallback, ty);
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
            | ItemKind::Enum { .. }
            | ItemKind::Variant { .. }
            | ItemKind::Context { .. }
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
            | TyKind::Variant { .. }
            | TyKind::Partial(_)
            | TyKind::Secret(..)
            // TEMPORARY (alias rule): see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/).
            | TyKind::Builtin(_) => {
                format!("`{shown}`")
            }
            TyKind::Enum { .. }
            | TyKind::Int
            | TyKind::Float
            | TyKind::Bool
            | TyKind::String
            | TyKind::Quantity(_)
            | TyKind::File
            | TyKind::Dir
            | TyKind::Sandbox
            | TyKind::UntypedInt
            | TyKind::Error
            | TyKind::Never
            | TyKind::Param { .. } => shown.to_string(),
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

/// What `Checker::filled` holds for a parameter a call gives its default (see
/// "Calls" under [Functions](crate::check::functions#functions)): no argument's
/// index, and not `u32::MAX`, which is a parameter left unfilled.
const DEFAULTED: u32 = u32::MAX - 1;

/// The note on [`FIELD_NOT_CONSTANT`]: what a
/// constant is.
const CONSTANT_NOTE: &str = "a literal, or a list or record of literals";

/// The error for a field's default of a record type that is not a constant,
/// at the default (see "Field defaults" under [Functions](crate::check::functions#functions)).
const FIELD_NOT_CONSTANT: &str = "a field's default must be a constant";

/// The constant that `id` of `ast`, a parameter's or a field's default that
/// [`is_constant`](crate::ast::is_constant) accepts, is, `types` being the type of each expression,
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

/// The constant that `id` of `ast`, a field's default of a record type copied
/// out of its tree into a [`crate::ast::TypeStore`], is written as, with no
/// type known and nothing filled ([`constant_of`]), if
/// [`is_constant`](crate::ast::is_constant) accepts it; `None` if it is not a
/// constant, which the check of its own tree reports (see "Field defaults"
/// under [Functions](crate::check::functions#functions)). The checker of each
/// tree using the type fits it to its field's type (`Checker::fit_constant`).
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
/// argument fills (see [Suggestions](crate::check::names#suggestions)). For the shorthand, which
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
/// [Def-types](crate::check::deftypes#def-types)).
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

pub(crate) mod attributes;
pub(crate) mod builtin;
pub(crate) mod calls;
pub(crate) mod context;
pub(crate) mod deftypes;
pub(crate) mod enums;
pub(crate) mod expect;
pub(crate) mod failure;
pub(crate) mod fields;
pub(crate) mod files;
pub(crate) mod functions;
pub(crate) mod futures;
pub(crate) mod imports;
pub(crate) mod inference;
pub(crate) mod items;
pub(crate) mod lists;
pub(crate) mod loops;
pub(crate) mod methods;
pub(crate) mod mutation;
pub(crate) mod names;
pub(crate) mod narrowing;
pub(crate) mod operators;
pub(crate) mod records;
pub(crate) mod resolution;
pub(crate) mod resolve;
pub(crate) mod results;
pub(crate) mod schedule;
pub(crate) mod scopes;
pub(crate) mod secrets;
pub(crate) mod toplevel;

#[cfg(test)]
mod tests;
