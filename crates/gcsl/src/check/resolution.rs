//! What the checker resolves for lowering: where each name reads from and
//! writes to, what each function's frame and captures are, which parameter each
//! argument fills, and what is awaited where.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{ArgRange, ExprId, FnId, ForId, GenericOwner, ItemId, RecoverId};
use crate::names::Name;
use crate::source::Span;
use crate::types::{AwaitKind, Decode, GenericId, Need, TyId, ValueTest};

use super::{
    Attribute, Constant, FieldSchedule, ImportedRecord, ImportedRecords, MadeAt, ParamDefault,
    Place, RecordFills, SharedChildren, Slot, VarRef, VariantTag, ViewRecord,
};

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
    /// declares one (see "Calls" under
    /// [Functions](crate::check::functions#functions)); `None` for every other
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

/// What the checker keeps of generic functions, methods and their calls
/// (see [Generics](crate::check::methods#generics) and [Methods](crate::check::methods#methods)).
#[derive(Debug, Default)]
pub(super) struct CallsChecked {
    /// What the type parameters of each generic function and `type` item
    /// belong to, made once for it.
    pub(super) ids: HashMap<GenericOwner, GenericId>,
    /// The arguments of the call being bound that were checked to infer its
    /// type parameters, each with its type: checked once, then only
    /// expected to fit its substituted parameter.
    pub(super) prechecked: HashMap<ExprId, TyId>,
    /// The callee of the call being checked, through any parentheses: the
    /// one place a generic function's name may be read.
    pub(super) callee: Option<ExprId>,
    /// The methods of the earlier inputs of a session, by their names, each
    /// with its binding's.
    pub(super) earlier_methods: HashMap<Name, Vec<Name>>,
}

/// What the check gives lowering besides types and slots, as
/// [`Resolution`] carries it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ForLowering {
    /// Each call that gives a parameter its default, by its id, with each such
    /// parameter and its default, in parameter order (see "Calls" under
    /// [Functions](crate::check::functions#functions)); only a program with one
    /// has entries.
    pub(super) call_defaults: HashMap<ExprId, Box<[(u32, ParamDefault)]>>,
    /// Each call of a method, by its id, with its receiver, its callee's
    /// parameter 0 (see [Methods](crate::check::methods#methods)).
    pub(super) method_calls: HashMap<ExprId, ExprId>,
    /// Each variant as a value, by its id (see [Enums](crate::check::enums#enums)).
    pub(super) variants: HashMap<ExprId, VariantTag>,
    /// Each read of a context, by its id, with its declaration's key (see
    /// [Context](crate::check::context#context)).
    pub(super) context_reads: HashMap<ExprId, Name>,
    /// Each `with`, by its id, with the key of the declaration of each
    /// binding's context, in order (see [Context](crate::check::context#context)).
    pub(super) withs: HashMap<ExprId, Box<[Name]>>,
    /// The slot each `recover` whose fallback names its failure stores the
    /// failure in (see [Failure](crate::check::failure#failure)).
    pub(super) recover_slots: HashMap<RecoverId, Slot>,
    /// Each operation on a secret, by its id, with the type of the value it
    /// computes, which lowering makes a secret (see [Secrets](crate::check::secrets#secrets)).
    pub(super) secret_ops: HashMap<ExprId, (TyId, bool)>,
    /// Each operand of an operation on secrets that is or holds a secret,
    /// by its id, with the type of what it is unwrapped to and whether that
    /// is deeply, every secret it holds its value (see [Secrets](crate::check::secrets#secrets)).
    pub(super) secret_operands: HashMap<ExprId, (TyId, bool)>,
    /// Each operand of an operation on secrets that neither is nor holds
    /// one, which lowering marks as an operand, so that its failure is not
    /// the operation's (see [Secrets](crate::check::secrets#secrets)).
    pub(super) plain_operands: HashSet<ExprId>,
    /// The number of the rest parameter of the callee of each call whose
    /// callee has one, which lowering gives the list of the arguments that
    /// fill it, the empty list if none does (see "Calls" under
    /// [Functions](crate::check::functions#functions)).
    pub(super) call_rests: HashMap<ExprId, u32>,
    /// The attributes of each function item that has any, by its function,
    /// in source order (see [Attributes](crate::check::attributes#attributes)).
    pub(super) fn_attributes: HashMap<FnId, Box<[Attribute]>>,
    /// The functions of the input that a deferring attribute is on, whose
    /// calls a host may run as steps (see [Deferring
    /// attributes](crate::check::attributes#deferring-attributes)).
    pub(super) deferred: HashSet<FnId>,
    /// Each record literal, a record block or `{}`, that the checker gave
    /// fields their defaults, by its id, with each such field and its default,
    /// in the order of their names' text (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)); only a program with one
    /// has entries.
    pub(super) record_fills: RecordFills,
    /// Each read of a file's `path`, `f.path`, by its id (see
    /// [Files](crate::check::files#files)).
    pub(super) file_paths: HashSet<ExprId>,
}

/// A parameter's default function, as its check found it (see "Calls"
/// under [Functions](crate::check::functions#functions)).
#[derive(Clone, Debug)]
pub(super) struct DefaultFn {
    /// The parameter's number.
    pub(super) param: usize,
    /// Whether it checked without an error.
    pub(super) checked: bool,
    /// What it needs (see [Needs](crate::check::needs#needs)).
    pub(super) needs: Box<[Need]>,
}

/// The defaults of a function, for [`FnResolution::defaults`]: `constants`,
/// each parameter's constant default, if it has one that checked, and
/// `functions`, each parameter's default function, in parameter order, whose
/// closures follow the function's `captures` own captures in its closure (see
/// "Calls" under [Functions](crate::check::functions#functions)). Empty if no
/// parameter has a default.
pub(super) fn merged_defaults(
    constants: Vec<Option<Constant>>,
    functions: &[DefaultFn],
    captures: usize,
) -> Vec<Option<ParamDefault>> {
    let count = functions
        .iter()
        .map(|default| default.param + 1)
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
    for (at, default) in functions.iter().enumerate() {
        if let (true, Some(entry)) = (default.checked, merged.get_mut(default.param)) {
            let capture = u32::try_from(captures + at).unwrap_or(u32::MAX);
            *entry = Some(ParamDefault::Function {
                capture,
                needs: default.needs.clone(),
            });
        }
    }
    merged
}

/// What a call of a function whose tree is gone needs to know of its
/// parameters: a function declared by an earlier session input, through
/// the binding its scope carries ([`Scope`](super::Scope)), or a `def` of another file of
/// a workspace, through its cell (see [Imports](crate::check::imports#imports)). For each
/// parameter, in order, whether its body measures it, which makes it
/// present, for the call-site form of the implicit-await error, and its
/// default, if it has one, which a call that leaves it unfilled is given;
/// and its attributes, which a def-type gives its definitions (see
/// [Attributes](crate::check::attributes#attributes)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KnownParams {
    /// For each parameter, whether the body measures it.
    pub(super) measured: Box<[bool]>,
    /// For each parameter, its default, if it has one; empty if none has.
    pub(super) defaults: Box<[Option<ParamDefault>]>,
    /// The function's attributes, in source order.
    pub(super) attributes: Box<[Attribute]>,
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
    pub(super) fn measured(&self, param: usize) -> bool {
        self.measured.get(param).copied().unwrap_or(false)
    }

    /// The default of parameter number `param`, if it has one.
    pub(super) fn default(&self, param: usize) -> Option<&ParamDefault> {
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
    pub(super) const fn unawaited(source: VarRef, existing: u32) -> Self {
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

/// Where every variable of an [`Ast`](super::Ast) is stored, what every function's
/// frame and captures are, and which parameter every argument fills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// Where each `Name` expression reads, indexed by [`ExprId`];
    /// [`VarRef::None`] for every other expression.
    pub(super) expr_vars: Vec<VarRef>,
    /// Whether and how each expression is awaited where it is used,
    /// indexed by [`ExprId`].
    pub(super) awaits: Vec<AwaitMark>,
    /// The slot each `Let`, `Field` and `Rebind` item writes, indexed by
    /// [`ItemId`], and for a `Spread` item the slot its operand's value is
    /// held in while its fields are read; [`Slot::NONE`] for every other
    /// item. Every definition of an open field writes the field's one slot,
    /// and so does every definition of a field that overrides a soft one.
    pub(super) item_slots: Vec<Slot>,
    /// Where the slots of each `Spread` item's fields are in
    /// `spread_slots`, indexed by [`ItemId`]: their start and their number;
    /// `(0, 0)` for every other item.
    pub(super) item_spreads: Vec<(u32, u32)>,
    /// The slot each field of each spread's operand is stored in, in the
    /// order of the fields of the operand's static type, each spread's
    /// together.
    pub(super) spread_slots: Vec<Slot>,
    /// How the path of each dotted `Let`, `Field` and `Rebind` item
    /// resolves, indexed by [`ItemId`]; [`PathResolution::NONE`] for every
    /// other item, and for one that builds its record from nothing.
    pub(super) item_paths: Vec<PathResolution>,
    /// For each dotted `let` and `:=` item of which some record along its
    /// path is awaited, whether each of its existing records is, from its
    /// source down, where its [`PathResolution::awaits`] says. Items with no
    /// awaited record have no entries.
    pub(super) path_awaits: Vec<bool>,
    /// Each top-level function of the input, by its item, in source order,
    /// with where its closure is made.
    pub(super) top_functions: Vec<(ItemId, MadeAt)>,
    /// The cell of each node of the input's tree of top-level fields
    /// ([`Mentions::tree`](super::Mentions::tree)), by its index there: the slot of the top level's
    /// frame that holds its value. A root's is its binding's slot, a `def`'s
    /// its function's; the others are slots no name finds.
    pub(super) field_cells: Vec<Slot>,
    /// For a file of a workspace, the private slot of the file's own record
    /// of each node it shares with another file, by its index in the tree;
    /// [`Slot::NONE`] for any other node, and empty outside a workspace
    /// (see [Directory scope](crate::check::imports#directory-scope)).
    pub(super) own_records: Vec<Slot>,
    /// For a file of a workspace, the children of each node of its tree
    /// that other files declare, with their cells and types: what the
    /// node's assembly reads besides its own children. Empty outside a
    /// workspace.
    pub(super) shared_children: SharedChildren,
    /// For a file of a workspace, the records it reads whole through its
    /// imports (a directory's, a file's own, a file's part of a shared
    /// node), each with the private slot it is assembled into as the file
    /// starts and the name, cell and type of each field, nested records
    /// before the records that hold them (see [Imports](crate::check::imports#imports)).
    pub(super) imported_records: ImportedRecords,
    /// For a file of a workspace, the record of each node of its view with a
    /// leaf of the file under it, by its index among them
    /// ([`Mentions::views`](super::Mentions::views)): stored at its point in
    /// the schedule, after its own children, if the file reads it whole (see
    /// [Imports](crate::check::imports#imports)).
    pub(super) view_records: Vec<ViewRecord>,
    /// For a value file of a workspace that another file imports, the cell
    /// its result is stored in as its block ends; [`Slot::NONE`] otherwise.
    pub(super) value_cell: Slot,
    /// Where the input is, as its run's driver resolved it: what its code
    /// carries and what `fileroot.this` and `fileroot.workspace` are in it;
    /// the default, directory 0 and empty texts, outside a workspace's
    /// context, where nothing can import `fileroot`.
    pub(super) place: Place,
    /// The tree of the input's top-level fields and when each node is
    /// computed (see [Top-level fields](crate::check::toplevel#top-level-fields)).
    pub(super) field_schedule: FieldSchedule,
    /// The number of slots the top level's frame needs.
    pub(super) frame_size: u32,
    /// For each slot of the top level's frame, the name of the last
    /// variable assigned to it.
    pub(super) slot_names: Vec<Name>,
    /// What the checker found for each function expression, indexed by
    /// [`FnId`].
    pub(super) functions: Vec<FnResolution>,
    /// The index of the parameter each argument fills, indexed like the
    /// [`Ast`](super::Ast)'s argument pool; `u32::MAX` for an argument of an erroneous
    /// call.
    pub(super) arg_params: Vec<u32>,
    /// The slot each `for` expression's variable is stored in, indexed by
    /// [`ForId`].
    pub(super) for_slots: Vec<Slot>,
    /// What each `break`, `continue` and `return` item is for, indexed by
    /// [`ItemId`]; `None` for every other item, and for one that is in
    /// error.
    pub(super) jumps: Vec<Option<JumpTarget>>,
    /// What was found of each loop, indexed by [`ForId`].
    pub(super) loops: Vec<LoopResolution>,
    /// The labelled blocks that some `return` leaves, by their ids.
    pub(super) returned_blocks: HashSet<ExprId>,
    /// The statements whose values are discarded, by their ids: calls of a
    /// function that is not pure, outside a pure function.
    pub(super) discarded: HashSet<ExprId>,
    /// What was decided for each `is` and `as` expression, by its id; only
    /// a program that has one has entries.
    pub(super) tests: HashMap<ExprId, TestResolution>,
    /// The decoding plan of each call of `json.decode`, by its id; only a
    /// program that has one has entries.
    pub(super) decodes: HashMap<ExprId, Decode>,
    /// Each field access that reads a function's meta, `func.versions`, by
    /// its id, with the meta's type and how the meta is awaited before its
    /// field is read (see [Metas](crate::check::attributes#metas)); only a program that reads one
    /// has entries.
    pub(super) meta_reads: HashMap<ExprId, (TyId, AwaitMark)>,
    /// Each definition made with a def-type that declares a meta, by its
    /// function, with the cell of the top level's frame that holds its
    /// meta's closure (see [Metas](crate::check::attributes#metas)).
    pub(super) metas: Vec<(FnId, Slot)>,
    /// What the check gives lowering besides types and slots: see the
    /// accessors.
    pub(super) lowered: ForLowering,
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
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
    /// for.
    #[must_use]
    pub fn expr_var(&self, id: ExprId) -> VarRef {
        self.expr_vars[id.index()]
    }

    /// Whether and how expression `id` is awaited where it is used:
    /// [`AwaitMark::None`] if it is used as it is. An awaited expression's
    /// type in [`Types`](super::Types) is its type before the await.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
    /// for.
    #[must_use]
    pub fn await_of(&self, id: ExprId) -> AwaitMark {
        self.awaits[id.index()]
    }

    /// What the checker found for function expression `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
    /// for.
    #[must_use]
    pub fn function(&self, id: FnId) -> &FnResolution {
        &self.functions[id.index()]
    }

    /// The parameter each argument of `args`, a call's arguments, fills,
    /// in the order of [`Ast::args`](super::Ast::args); `u32::MAX` for an argument of an
    /// erroneous call.
    ///
    /// # Panics
    ///
    /// Panics if `args` does not belong to the [`Ast`](super::Ast) this table was built
    /// for.
    #[must_use]
    pub fn arg_params(&self, args: ArgRange) -> &[u32] {
        &self.arg_params[args.start()..args.start() + args.count()]
    }

    /// The number of the rest parameter of the callee of the call `id`, if
    /// it has one, which lowering gives the list of the arguments that fill
    /// it, the empty list if none does (see "Calls" under
    /// [Functions](crate::check::functions#functions)).
    #[must_use]
    pub fn call_rest(&self, id: ExprId) -> Option<u32> {
        self.lowered.call_rests.get(&id).copied()
    }

    /// Whether the field access `id` reads a file's `path` (see
    /// [Files](crate::check::files#files)).
    #[must_use]
    pub fn file_path(&self, id: ExprId) -> bool {
        self.lowered.file_paths.contains(&id)
    }

    /// The parameters that the call `id` gives their defaults, each with its
    /// default, in parameter order: those it leaves unfilled that have one, if
    /// its callee is known (see "Calls" under
    /// [Functions](crate::check::functions#functions)); none for any other
    /// call, and for an expression that is no call.
    #[must_use]
    pub fn call_defaults(&self, id: ExprId) -> &[(u32, ParamDefault)] {
        self.lowered
            .call_defaults
            .get(&id)
            .map_or(&[], |defaults| defaults)
    }

    /// The variant the expression `id` is, a variant as a value (see
    /// [Enums](crate::check::enums#enums)), which lowering makes a constant.
    #[must_use]
    pub fn variant(&self, id: ExprId) -> Option<&VariantTag> {
        self.lowered.variants.get(&id)
    }

    /// The key of the declaration of the context the expression `id`, a
    /// read of one, reads (see [Context](crate::check::context#context)); its default's function,
    /// if it has one, is the read's variable ([`Resolution::expr_var`]).
    #[must_use]
    pub fn context_read(&self, id: ExprId) -> Option<Name> {
        self.lowered.context_reads.get(&id).copied()
    }

    /// The type of the value the expression `id` computes, if it is an
    /// operation on a secret, whose result lowering makes a secret, and
    /// whether that secret is revealed (see
    /// [Secrets](crate::check::secrets#secrets)).
    #[must_use]
    pub fn secret_op(&self, id: ExprId) -> Option<(TyId, bool)> {
        self.lowered.secret_ops.get(&id).copied()
    }

    /// The type of what the expression `id` is unwrapped to, and whether
    /// deeply, every secret it holds its value, if it is an operand of an
    /// operation on secrets that is or holds a secret, which lowering
    /// unwraps (see [Secrets](crate::check::secrets#secrets)).
    #[must_use]
    pub fn secret_operand(&self, id: ExprId) -> Option<(TyId, bool)> {
        self.lowered.secret_operands.get(&id).copied()
    }

    /// Whether the expression `id` is an operand of an operation on secrets
    /// that neither is nor holds one, which lowering marks as an operand,
    /// so that its failure is not the operation's (see [Secrets](crate::check::secrets#secrets)).
    #[must_use]
    pub fn is_plain_operand(&self, id: ExprId) -> bool {
        self.lowered.plain_operands.contains(&id)
    }

    /// The slot the `recover` `id` stores its failure in for its fallback,
    /// if the fallback names it (see [Failure](crate::check::failure#failure)).
    #[must_use]
    pub fn recover_slot(&self, id: RecoverId) -> Option<Slot> {
        self.lowered.recover_slots.get(&id).copied()
    }

    /// The keys of the declarations of the contexts the `with` `id` binds,
    /// one for each of its bindings, in order (see [Context](crate::check::context#context)).
    #[must_use]
    pub fn with_keys(&self, id: ExprId) -> Option<&[Name]> {
        self.lowered.withs.get(&id).map(|keys| &keys[..])
    }

    /// The receiver of the call `id`, if it is a method's (see
    /// [Methods](crate::check::methods#methods)): its callee's parameter 0, the callee itself
    /// read as its method's binding, not as the field access it is written
    /// as.
    #[must_use]
    pub fn method_receiver(&self, id: ExprId) -> Option<ExprId> {
        self.lowered.method_calls.get(&id).copied()
    }

    /// The attributes of function `function`, in source order: none for a
    /// function that is no function item, or has none (see
    /// [Attributes](crate::check::attributes#attributes)).
    #[must_use]
    pub fn attributes(&self, function: FnId) -> &[Attribute] {
        self.lowered
            .fn_attributes
            .get(&function)
            .map_or(&[], |attributes| attributes)
    }

    /// Whether a deferring attribute is on function `function`, so that a
    /// host may run its calls as steps (see [Deferring
    /// attributes](crate::check::attributes#deferring-attributes)).
    #[must_use]
    pub fn defers(&self, function: FnId) -> bool {
        self.lowered.deferred.contains(&function)
    }

    /// The fields that the record literal `block`, a record block or `{}`,
    /// lacks and is given, each with its default, in the order of their names'
    /// text: the defaulted fields of the record type it is written where it is
    /// expected (see "Field defaults" under
    /// [Functions](crate::check::functions#functions)); none for any other
    /// block, and for an expression that is no block.
    #[must_use]
    pub fn record_fills(&self, block: ExprId) -> &[(Name, Constant)] {
        self.lowered
            .record_fills
            .get(&block)
            .map_or(&[], |fills| fills)
    }

    /// The slot written by item `id`, or [`Slot::NONE`] if it is not a
    /// declaration. For a spread item, the slot its operand's value is held
    /// in while its fields are read, which no binding owns (see
    /// [`Resolution::spread_slots`] for its fields').
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
    /// for.
    #[must_use]
    pub fn item_slot(&self, id: ItemId) -> Slot {
        self.item_slots[id.index()]
    }

    /// Each top-level function of the input, by its item, in source order,
    /// with where its closure is made: what lowering stores in each one's
    /// slot, and when (see [When a function may be
    /// used](crate::check::schedule#when-a-function-may-be-used)).
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
    /// [Imports](crate::check::imports#imports)).
    #[must_use]
    pub fn imported_records(&self) -> &[ImportedRecord] {
        &self.imported_records
    }

    /// For a file of a workspace, the record of the node of its view with a
    /// leaf of the file under it numbered `node` among the field nodes
    /// (after its tree's), if it is one: stored at the node's point (see
    /// [Imports](crate::check::imports#imports)).
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
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
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
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
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

    /// The decoding plan of the call `id` of `json.decode` (see [JSON
    /// decoding](crate::check::json#json-decoding)); `None` for any other
    /// expression.
    #[must_use]
    pub fn decode_of(&self, id: ExprId) -> Option<&Decode> {
        self.decodes.get(&id)
    }

    /// If the field access `id` reads a function's meta (see
    /// [Metas](crate::check::attributes#metas)): the meta's type, before any await, and how the
    /// meta is awaited before its field is read. `None` for any other
    /// expression.
    #[must_use]
    pub fn meta_read(&self, id: ExprId) -> Option<(TyId, AwaitMark)> {
        self.meta_reads.get(&id).copied()
    }

    /// Each definition made with a def-type that declares a meta, by its
    /// function, with the cell of the top level's frame that holds its
    /// meta's closure, which lowering gives its code (see [Metas](crate::check::attributes#metas)).
    #[must_use]
    pub fn metas(&self) -> &[(FnId, Slot)] {
        &self.metas
    }

    /// How the path of item `id` resolves: [`PathResolution::NONE`] if it
    /// is not a dotted declaration or builds its record from nothing.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to the [`Ast`](super::Ast) this table was built
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
            decodes: HashMap::new(),
            meta_reads: HashMap::new(),
            metas: Vec::new(),
            lowered: ForLowering::default(),
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
    /// the number of expressions in the [`Ast`](super::Ast).
    pub(crate) fn expr_len(&self) -> usize {
        self.expr_vars.len().min(self.awaits.len())
    }

    /// Number of function entries, equal to the number of function
    /// expressions in the [`Ast`](super::Ast).
    pub(crate) fn function_len(&self) -> usize {
        self.functions.len()
    }

    /// Number of argument entries, equal to the number of arguments in the
    /// [`Ast`](super::Ast).
    pub(crate) fn arg_len(&self) -> usize {
        self.arg_params.len()
    }

    /// Number of `for` entries, equal to the number of `for` expressions
    /// in the [`Ast`](super::Ast).
    pub(crate) fn for_len(&self) -> usize {
        self.for_slots.len()
    }

    /// Number of item entries in every item table, equal to the number of
    /// items in the [`Ast`](super::Ast).
    pub(crate) fn item_len(&self) -> usize {
        self.item_slots
            .len()
            .min(self.item_paths.len())
            .min(self.item_spreads.len())
            .min(self.jumps.len())
    }
}
