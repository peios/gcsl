//! A stateful session: a sequence of inputs sharing their variables, as in
//! the REPL.
//!
//! Each input is a block body, like a file, run in the context of every
//! earlier successful input: variables and fields declared at their top
//! level are visible, `:=` at top level may rebind variables, `:=` in any
//! block of its top level (but not in a function or a `lazy`) may rebind
//! mutable ones, and new top-level declarations persist for later inputs.
//! The session keeps these things between inputs:
//!
//! - the [`Names`] table, lent to each stage in turn, so that the same name
//!   in two inputs is the same [`crate::names::Name`];
//! - the [`TypeTable`], lent to the checker, so that a type interned by one
//!   input is the same [`TyId`] in the next, and the types of the variables
//!   carried over stay meaningful;
//! - the checker's outermost [`Scope`]: each top-level variable with its
//!   slot, its type and whether it is mutable, from which the checker
//!   ([`crate::check::Checker`]) checks the next input exactly as if it
//!   continued the same block;
//! - the value stack, whose bottom is the top level's frame, holding each
//!   top-level variable's value in its slot, and which calls grow and
//!   shrink back while an input runs;
//! - the type of the last successful input's value, for showing it;
//! - the source of every committed input, in order: input `n` is the `n`th
//!   to succeed, counting from 0, the standard files first (see [The
//!   standard library](#the-standard-library)), and each input is lowered
//!   with its number. A closure or a future outlives the input that made
//!   it, keeping its code alive, and when one made by an earlier input
//!   fails, its diagnostic names that input (see
//!   [`crate::Diagnostic::input`]), whose source [`Session::source`]
//!   returns for rendering it. A future that
//!   fails is left unrun, so a later input that awaits it runs it again;
//! - the definitions of the types its inputs have declared, and which of
//!   them, by name, the next input inherits (see [Types](#types));
//! - the [`MemoTables`] of every memoised (`->>`) closure any input has
//!   called, from lists of arguments to what the closure returned, so that
//!   a later input calling the closure with equal arguments gets the
//!   earlier result without running the body again (see
//!   [`crate::eval`]). A table lives as long as its closure: the session
//!   holds the closure weakly and drops the tables of dead closures from
//!   time to time; an entry of a live closure is never evicted.
//!
//! A failed input, whether it fails to compile or fails at runtime, leaves
//! the session as it was: the scope is only replaced after evaluation
//! succeeds, names interned by the failed input are forgotten, and its
//! source is not kept, so it takes no number. The frame may have grown, and
//! slots above the last committed variable may hold values from the failed
//! run, but those slots belong to no variable and are always written before
//! they are read. A run may also have rebound a committed mutable variable
//! before it failed, which writes the variable's own slot: so before each
//! input runs, the session copies the values of the committed mutable
//! variables, the only committed slots an input can write (see
//! [`Scope::mutable_slots`]), and writes them back if the input fails at
//! runtime. The values are reference counted, so the copy is cheap, and
//! with no mutable variable there is none. Types the failed input interned
//! are kept: a type is a fact about structure, not about the input, so a stale
//! entry is harmless (see [`crate::types`]); forgetting them would mean
//! rebuilding the table's lookup map. The memo entries the failed input made
//! are forgotten with it (see [`MemoTables::rollback`]): their keys and
//! results may be records with names it interned, or closures of its code,
//! which would mean something else, or nothing, to a later input. So a
//! memoised call the failed input made runs again when a later input makes
//! it, although its effects have happened.
//!
//! Within one input, top-level functions are in any order: each is visible
//! in the whole input, and its closure is stored where it is made, which
//! may be before its own line (see the checker's "Top-level functions").
//! Between inputs, order is as it was: a later input sees the earlier ones
//! and may declare their names again, and an earlier one is never changed
//! by a later one. Nothing new is kept for this: a top-level function is a
//! variable of the scope like any other, whose slot is among its input's
//! first. A function of an earlier input reads the other top-level
//! functions of its input from their slots at the bottom of the shared
//! value stack, slots that no later input writes, since a later input's
//! declarations take slots above the scope's, so a pair of mutually
//! recursive functions keeps calling each other after a later input
//! declares one of their names again.
//!
//! Top-level fields are in any order within an input too, each computed at
//! its point (see the checker's "Top-level fields"), and each node of the
//! tree of their paths has a cell, a slot of the top level's frame. Every
//! cell slot is committed with its input, a root's and an interior node's
//! or a leaf's alike, although only a root's is a binding a name finds: the
//! scope's next free slot ([`crate::check::Scope`]) is past the last of
//! them, so no later input reuses one, and a closure or a future of the
//! input that reads a cell through the frame finds it there for as long as
//! the session lasts. A failed input's cells are forgotten with its other
//! slots. A later input sees an earlier input's field root whole, as a
//! field of its type, and `a.c = 2` in a later input declares its own `a`.
//!
//! A `def` named by a dotted path, `def dev.peios.count(...)`, is a leaf of
//! its input's tree like any dotted field, and needs nothing new: its leaf's
//! cell holds its closure, so it takes a slot of its own, committed with
//! the input's other cells (a `def` of one name has none: its cell is its
//! function's binding's slot), and its body reads its input's cells, the
//! cell of its own path among them, through the frame. A later input sees
//! it as it sees dotted fields: through the root's binding, `dev`, whose
//! record holds the closure, so `dev.peios.count(3)` calls it; it has no
//! name of its own (`count` is no variable), and `def dev.other()` in a
//! later input declares that input's own `dev`. Called from a later input,
//! it is no known callee, as a function held by a dotted field is not: the
//! call-site form of the implicit-await error is a `def` of one name's.
//!
//! # The standard library
//!
//! A session is not a workspace: its inputs share their `let`s, which the
//! files of a workspace never do. But it has the standard library, which
//! it runs once, as it starts ([`Session::new`]), through the driver of a
//! workspace that has no file of its own ([`crate::workspace`], "The
//! standard library"), into its own names, types, frame and memo tables,
//! keeping the layout of the records `exec` returns that the run built, for
//! every input (see [`crate::eval`], "The built-in functions"), and the
//! run's temporary directories, which `fs.temp_dir` makes in every input
//! from then on, owned as a workspace owns its own (see
//! [`crate::workspace`], "The standard library"): none is made until an
//! input asks for one, and every one is removed, with everything in it, as
//! the session is dropped, so when the REPL ends, whatever its inputs did.
//! A clone of the session shares them, and they are removed as the last
//! clone is dropped. Unlike a
//! workspace, which loads only the standard files its files import, a
//! session loads, checks and runs EVERY standard file then, each a scope of
//! its own as in a workspace, so that an input's `import path` finds it
//! run already; nothing is loaded mid-session. The built-in functions'
//! cells are the first slots of its frame and the standard files' cells
//! and slots the next, its scope's first free slot past them all; the
//! standard files are its first inputs, in path order (`<stdlib>/fs.gcsl`
//! is input 0, `<stdlib>/path.gcsl` input 1 and `<stdlib>/prelude.gcsl`
//! input 2), so its own first input is number 3, and [`Session::source`] of
//! a standard file is its source,
//! which a diagnostic of one, or a label in one, is rendered against.
//! Should that fail, which is a bug, every input fails with its
//! diagnostics.
//!
//! Every input is then checked and run as before, from the session's
//! scope, with two additions to its context: the prelude's declarations,
//! through its implicit `#[shadowable] import * from prelude`, first of all
//! in its view's order, which anything of their names shadows (see
//! [`crate::workspace`], "Shadowable declarations"), and `builtins` and the
//! standard modules importable by name. An input is no file, and cannot
//! turn the prelude off: one marked `#![no_prelude]` is "`#![no_prelude]`
//! applies only to a file". Its `#[shadowable]` marks are accepted, and are
//! a file's within the input, but change nothing between inputs, since a
//! later input shadows an earlier one's names already. To have them, an
//! input is scanned beside
//! every standard file as a lone file is beside those it needs, so that
//! the standard files' cells are numbered as they were when the session
//! started, its own cells taking the slots after the scope's
//! (`crate::workspace::check_input`), and checked as a
//! session's input, which reads the scope's variables as before. An
//! `import` of an input persists for later inputs, as its `let`s, `fn`s and
//! `def`s do: what the input imports, whole, as a member or by `*`, is kept
//! once it has run, apart from the scope, since an imported name has no
//! binding, and every later input inherits it as an import of its own that
//! follows its own `import`s (`crate::workspace::check_input`). A later
//! input's own import of the name, or its declaration of it, shadows it, as
//! inputs shadow earlier names, and it is no longer kept; an import shadows
//! an earlier input's variable of its name in turn. A failed input's
//! imports are forgotten with it. A later input declaring `len`
//! shadows the prelude's as inputs shadow earlier inputs' names, and an
//! earlier input's alias of `len` stays `len`. A failed input never
//! forgets the standard library, which no input changes.
//!
//! A session's root, for the `fileroot` module, is the process's working
//! directory when [`Session::new`] runs, canonical
//! ([`crate::workspace::working_directory`]), which the standard library's
//! run is located at: it is index 0 of the table of directories that run
//! built, which the session keeps for every input's run. Every input is in
//! it: an input's `fileroot.this`, `fileroot.workspace`, and its top
//! level's `fileroot.caller`, are the working directory, and a function of
//! an earlier input called from a later one has it as its caller too. A
//! later change of the working directory changes nothing; nor does an
//! input, so the table is never extended.
//!
//! # Types
//!
//! An input may declare types, `type Foo = ...` (see the checker's "Type
//! declarations"), which later inputs see, as they see its `fn`s: the
//! session keeps their definitions in a [`TypeStore`], each input's after
//! the earlier inputs', and, once an input has run, its view of types, its
//! own and those it imported or inherited and did not shadow, each with its
//! definition's number there, which the next input inherits as the last
//! source of its own view (`crate::workspace::check_input`). So a later
//! input's `type Foo` shadows an earlier one's, as a later `fn` does, for
//! itself and every input after it, and a type of a later input may be
//! built from an earlier one's. A failed input's types are forgotten with
//! its names: the store is put back where it was. An input that only
//! declares a type has the value `{}`, and shows nothing, as one that only
//! declares a `fn`. Types have no runtime existence: nothing of the frame
//! is theirs.
//!
//! # Def-types
//!
//! An input may declare a def-type, define with one and invoke one (see the
//! checker's "Def-types"), and later inputs see its definitions as they see
//! its dotted `def`s: through the binding of the namespace's key, a root
//! whose record holds their closures. The session keeps, as it keeps the
//! inputs' imports, the def-types of the committed inputs, each by its path
//! with its namespace's key, which the next input inherits
//! (`crate::workspace::check_input`): a `DT` the input's own view does not
//! have is looked up there. A name an input binds ends an earlier def-type
//! under it, and a def-type it declares takes the earlier one's place. A
//! later input's definitions made with a def-type are its own namespace,
//! the key's binding, which stands for the namespace's whole for itself and
//! every input after it, as a later input's dotted root stands for its
//! root's: so an earlier input's definitions of the def-type are no longer
//! seen. An input of definitions alone has the value `{}`.
//!
//! A definition's meta (see the checker's "Metas") belongs to the
//! definition of its path in its directory, and the inputs of a session
//! share one directory scope, but each input is checked on its own, as the
//! one file of its scan: so a definition whose def-type declares a meta and
//! its `defmeta` must be in the SAME input. An input with the definition
//! alone is "`package x` has no `defmeta` in its directory", the meta noted
//! as its type shows when the def-type is an earlier input's, and one with
//! the `defmeta` alone "`defmeta package x` has no definition `x` in its
//! directory". A definition of an earlier input keeps its meta: its code
//! names its meta's cell, which keeps its slot.
//!
//! # Workspaces
//!
//! A [`crate::workspace::Workspace`] is driven like a session: its files
//! are inputs, in its order, on one frame with one memo table, each named
//! by its [`crate::Diagnostic::input`], lowered by `lower_checked` and run
//! by `run_input`, the functions of this module [`Session::eval`] uses
//! too. It differs in what it commits and when: a session commits an
//! input's whole scope
//! after the input has run, while a workspace commits a file's
//! declarations (its field roots and `def`s, not its `let`s and `fn`s)
//! once the file is checked, checks every file before any runs, and then
//! runs only the files the file asked for needs. Its cells' slots are
//! allocated for every file before the first is checked, below every
//! private slot.

use std::collections::HashSet;
use std::rc::Rc;

use crate::Outcome;
use crate::ast::{Ast, TypeStore};
use crate::check::{CheckOutput, Scope};
use crate::diagnostic::{self, Diagnostic};
use crate::eval::graph::GraphRun;
use crate::eval::{self, Direct, Effects, MemoTables, TempDirs};
use crate::ir::Ir;
use crate::lower;
use crate::names::{Name, Names};
use crate::source::SourceFile;
use crate::types::{TyId, TypeTable};
use crate::value::{Layout, Value, ContextFrame};
use crate::workspace::{self, Alias, Earlier, Standard};

/// A sequence of inputs sharing their top-level variables. See the module
/// documentation.
#[derive(Clone, Debug)]
pub struct Session {
    names: Names,
    types: TypeTable,
    scope: Scope,
    /// The value stack; its bottom is the top level's frame.
    frame: Vec<Value>,
    value_type: TyId,
    /// The source of each committed input, indexed by its number: the
    /// standard files first.
    sources: Vec<SourceFile>,
    /// The memo tables of every memoised closure called by any input.
    memo: MemoTables,
    /// The layout of the record `exec` returns, built once as the session
    /// started, for every input; `None` only if the standard library
    /// failed, when no input runs.
    exec_result: Option<Rc<Layout>>,
    /// The temporary directories `fs.temp_dir` makes for every input, the
    /// standard library's run's, removed as the session is dropped: shared
    /// by its clones, so removed as the last of them is (see [The standard
    /// library](#the-standard-library)).
    temp_dirs: Rc<TempDirs>,
    /// The standard library as it ran when the session started, beside
    /// which every input is checked; or, should it have failed, which is a
    /// bug, its diagnostics, which every input then fails with.
    standard: Result<Rc<Standard>, Vec<Diagnostic>>,
    /// The imports of the committed inputs that no later one has shadowed,
    /// which the next input inherits (see [The standard
    /// library](#the-standard-library)).
    imports: Vec<Alias>,
    /// The definitions of the types the inputs have declared (see
    /// [Types](#types)).
    type_store: TypeStore,
    /// The types of the committed inputs that no later one has shadowed,
    /// each with its declaration's number in `type_store`, which the next
    /// input inherits.
    declared_types: Vec<(Name, u32)>,
    /// The def-types of the committed inputs that no later one has
    /// shadowed, each by its path, with its namespace's key, which the next
    /// input inherits (see [Def-types](#def-types)).
    def_types: Vec<workspace::InheritedDefType>,
    /// The variants of the committed inputs that no later one has shadowed,
    /// each by its enum's key and its name, with its own key, which the next
    /// input inherits.
    variants: Vec<workspace::InheritedVariant>,
    /// The contexts of the committed inputs that no later one has
    /// redeclared, which the next input inherits.
    contexts: Vec<workspace::InheritedContext>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session with no variables of its own, which has run the standard
    /// library, every standard file of it: the standard files are its first
    /// inputs, its first slots their cells (see the module documentation).
    /// Its root, where every input is, is the process's working directory
    /// now ([`crate::workspace::working_directory`]).
    #[must_use]
    pub fn new() -> Self {
        let sources = workspace::standard_sources();
        match workspace::start(&workspace::working_directory()) {
            Ok(started) => Self {
                names: started.names,
                types: started.types,
                scope: Scope::of_slots(started.slot_names),
                frame: started.frame,
                value_type: TyId::EMPTY,
                sources,
                memo: started.memo,
                exec_result: Some(started.exec_result),
                temp_dirs: Rc::new(started.temp_dirs),
                standard: Ok(Rc::new(started.standard)),
                imports: Vec::new(),
                type_store: TypeStore::new(),
                declared_types: Vec::new(),
                def_types: Vec::new(),
                variants: Vec::new(),
                contexts: Vec::new(),
            },
            Err(diagnostics) => Self {
                names: Names::new(),
                types: TypeTable::new(),
                scope: Scope::new(),
                frame: Vec::new(),
                value_type: TyId::EMPTY,
                sources,
                memo: MemoTables::new(),
                exec_result: None,
                temp_dirs: Rc::new(TempDirs::new()),
                standard: Err(diagnostics),
                imports: Vec::new(),
                type_store: TypeStore::new(),
                declared_types: Vec::new(),
                def_types: Vec::new(),
                variants: Vec::new(),
                contexts: Vec::new(),
            },
        }
    }

    /// The source of committed input number `input`, the `input`th to have
    /// succeeded, counting from 0; `None` if no input of that number has
    /// been committed, as for the input being run. Diagnostics whose
    /// [`Diagnostic::input`] is set are rendered against it.
    #[must_use]
    pub fn source(&self, input: u32) -> Option<&SourceFile> {
        self.sources.get(input as usize)
    }

    /// Runs `source` as the next input and returns its value, `{}` if it
    /// has no result or fields, with its warnings in source order.
    ///
    /// # Errors
    ///
    /// Returns the diagnostics of the first stage that fails, as
    /// [`crate::run`] does, with the input's warnings among them in source
    /// order. The session is then unchanged.
    pub fn eval(&mut self, source: &SourceFile) -> Result<Outcome, Vec<Diagnostic>> {
        let names_before = self.names.len();
        let memo_before = self.memo.checkpoint();
        let types_before = self.type_store.mark();
        let result = self.eval_uncommitted(source);
        if result.is_err() {
            // What the input memoised may hold its names and closures of
            // its code, which mean nothing once it is forgotten; and so may
            // the definitions of the types it declared.
            self.memo.rollback(memo_before);
            self.names.truncate(names_before);
            self.type_store.truncate(types_before);
        }
        result
    }

    /// The table the session's types are interned into, for showing them.
    #[must_use]
    pub fn types(&self) -> &TypeTable {
        &self.types
    }

    /// The table the session's names are interned into, for showing them.
    #[must_use]
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// The type of the value of the last input that succeeded, as the
    /// checker found it: `{}` before any has. Show it with
    /// [`Session::types`] and [`Session::names`].
    #[must_use]
    pub fn value_type(&self) -> TyId {
        self.value_type
    }

    /// Runs `source`, committing its scope only on success. On failure the
    /// caller must forget the names it interned.
    fn eval_uncommitted(&mut self, source: &SourceFile) -> Result<Outcome, Vec<Diagnostic>> {
        let standard = match &self.standard {
            Ok(standard) => Rc::clone(standard),
            Err(diagnostics) => return Err(diagnostics.clone()),
        };
        let input = u32::try_from(self.sources.len()).unwrap_or(u32::MAX);
        // The names are lent to the scan and given back whatever happens;
        // the input's tree, and its string table, which lowering copies
        // what the IR needs out of, are dropped once it is lowered.
        let earlier = Earlier {
            imports: &self.imports,
            types: &self.declared_types,
            store: &mut self.type_store,
            def_types: &self.def_types,
            variants: &self.variants,
            contexts: &self.contexts,
        };
        let (ir, output, inherited) = workspace::check_input(
            &standard,
            (source, earlier),
            (&mut self.names, &mut self.types),
            self.scope.clone(),
            input,
        )?;
        let committed = self.committed_mutables();
        let direct = Direct {
            temp_dirs: Some(&self.temp_dirs),
        };
        // The input's names, lent to its run, which adds the names of
        // records it makes with names known only as it goes.
        let names = std::cell::RefCell::new(std::mem::take(&mut self.names));
        let context = (
            &mut self.memo,
            self.exec_result.as_ref(),
            &standard.dirs[..],
            &direct as &dyn Effects,
            Some(&names),
        );
        let ran = run_input(ir, &mut self.frame, context);
        self.names = names.into_inner();
        match ran {
            Ok(value) => {
                // A name the input bound that its scan could not see, as a
                // spread's, shadows an import of it too.
                let declared: HashSet<Name> = output.scope.names().skip(self.scope.len()).collect();
                let mut inherited = inherited;
                inherited
                    .imports
                    .retain(|alias| !declared.contains(&alias.name));
                self.imports = inherited.imports;
                self.declared_types = inherited.types;
                // A name the input binds shadows an earlier def-type under
                // it, but a def-type the input declares.
                self.def_types
                    .retain(|(path, _)| path.first().is_none_or(|name| !declared.contains(name)));
                // Each other name it binds is no def-type of its name for
                // the later inputs, the prelude's among them.
                let defined: HashSet<Name> = inherited
                    .def_types
                    .iter()
                    .filter_map(|(path, _)| path.first().copied())
                    .collect();
                for &name in declared.iter().filter(|name| !defined.contains(name)) {
                    self.def_types.push((vec![name], None));
                }
                self.def_types.extend(
                    inherited
                        .def_types
                        .into_iter()
                        .map(|(path, key)| (path, Some(key))),
                );
                // A variant the input declares shadows an earlier one of its
                // enum and name.
                self.variants.retain(|&(of, name, _)| {
                    !inherited
                        .variants
                        .iter()
                        .any(|&(later_of, later, _)| later_of == of && later == name)
                });
                self.variants.extend(inherited.variants);
                // A context the input declares shadows an earlier one of its
                // name.
                self.contexts.retain(|&(name, ..)| {
                    !inherited.contexts.iter().any(|&(later, ..)| later == name)
                });
                self.contexts.extend(inherited.contexts);
                self.scope = output.scope;
                self.value_type = output.ty;
                self.sources.push(source.clone());
                Ok(Outcome {
                    value,
                    warnings: output.warnings,
                })
            }
            Err(mut error) => {
                // The run may have rebound committed mutable variables
                // before it failed.
                self.restore(committed);
                // A closure of this input called from a closure of an
                // earlier one stamps this input's number: that is the input
                // being run, which is what no number means.
                if error.input == Some(input) {
                    error.input = None;
                }
                Err(diagnostic::merge(vec![error], output.warnings))
            }
        }
    }

    /// A copy of the value of each committed mutable variable, with its
    /// slot: all of the committed part of the frame that an input can
    /// change (see [`Scope::mutable_slots`]). The values are reference
    /// counted, so this copies no record, list or string; and without a
    /// mutable variable it is empty, and allocates nothing.
    fn committed_mutables(&self) -> Vec<(usize, Value)> {
        self.scope
            .mutable_slots()
            .filter_map(|slot| Some((slot.index(), self.frame.get(slot.index())?.clone())))
            .collect()
    }

    /// Writes back the values `committed_mutables` copied, after an input
    /// that failed at runtime, which may have rebound them.
    fn restore(&mut self, committed: Vec<(usize, Value)>) {
        for (index, value) in committed {
            if let Some(slot) = self.frame.get_mut(index) {
                *slot = value;
            }
        }
    }
}

/// Lowers `ast`, input number `input` of a session or a workspace, which
/// the checker's `output` is of, returning the IR with the output. Shared
/// by [`Session`] and [`crate::workspace`], which check differently.
pub(crate) fn lower_checked(
    ast: &Ast,
    output: CheckOutput,
    types: &TypeTable,
    input: u32,
) -> Result<(Ir, CheckOutput), Vec<Diagnostic>> {
    match lower::lower(ast, &output.checked, types, input) {
        Ok(ir) => Ok((ir, output)),
        Err(error) => Err(diagnostic::merge(vec![error.into()], output.warnings)),
    }
}

/// Runs `ir`, an input of a session or a workspace, on `frame`, the top
/// level's frame of every input so far, grown to the input's size, with
/// `memo`, the memo tables of every input, `exec_result`, the layout of
/// what `exec` returns, built once for every input, `dirs`, the table of
/// the run's directories that its code names its own by, and `temp_dirs`,
/// the temporary directories of the session or workspace, which
/// `fs.temp_dir` makes (see [`eval::eval_with`]).
pub(crate) fn run_input(
    ir: Ir,
    frame: &mut Vec<Value>,
    context: RunContext,
) -> Result<Value, Diagnostic> {
    run_input_in(&Rc::new(ir), frame, context, (None, None))
}

/// [`run_input`], of `ir`, in the root context `root`, the frame of the
/// contexts a host sets for the run, and as the run of a unit in graph mode,
/// `graph`, if it is one (see [`eval::eval_rooted`]).
pub(crate) fn run_input_in(
    ir: &Rc<Ir>,
    frame: &mut Vec<Value>,
    (memo, exec_result, dirs, effects, names): RunContext,
    (root, graph): (Option<Rc<ContextFrame>>, Option<&GraphRun<'_>>),
) -> Result<Value, Diagnostic> {
    let frame_size = ir.frame_size() as usize;
    if frame.len() < frame_size {
        frame.resize(frame_size, Value::Empty);
    }
    let extras = (exec_result, dirs, effects, names);
    eval::eval_rooted(ir, (frame, memo), extras, (root, graph))
}

/// What [`run_input`] runs an input with besides its frame: the memo tables
/// of every input, the layout of what `exec` returns, the table of the
/// run's directories, the temporary directories of the run's owner, and the
/// names of its owner, which a record made with names known only as it runs
/// adds to, if it has a table of names.
pub(crate) type RunContext<'a> = (
    &'a mut MemoTables,
    Option<&'a Rc<Layout>>,
    &'a [Rc<str>],
    &'a dyn Effects,
    Option<&'a std::cell::RefCell<Names>>,
);

#[cfg(test)]
mod tests;
