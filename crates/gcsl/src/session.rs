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
use crate::eval::{self, MemoTables, TempDirs};
use crate::ir::Ir;
use crate::lower;
use crate::names::{Name, Names};
use crate::source::SourceFile;
use crate::types::{TyId, TypeTable};
use crate::value::{Layout, Value};
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
    def_types: Vec<(Vec<Name>, Name)>,
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
        };
        let (ir, output, (mut imports, declared_types, def_types)) = workspace::check_input(
            &standard,
            (source, earlier),
            (&mut self.names, &mut self.types),
            self.scope.clone(),
            input,
        )?;
        let committed = self.committed_mutables();
        let context = (
            &mut self.memo,
            self.exec_result.as_ref(),
            &standard.dirs[..],
            &*self.temp_dirs,
        );
        match run_input(ir, &mut self.frame, context) {
            Ok(value) => {
                // A name the input bound that its scan could not see, as a
                // spread's, shadows an import of it too.
                let declared: HashSet<Name> = output.scope.names().skip(self.scope.len()).collect();
                imports.retain(|alias| !declared.contains(&alias.name));
                self.imports = imports;
                self.declared_types = declared_types;
                // A name the input binds shadows an earlier def-type under
                // it, but a def-type the input declares.
                self.def_types
                    .retain(|(path, _)| path.first().is_none_or(|name| !declared.contains(name)));
                self.def_types.extend(def_types);
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
    (memo, exec_result, dirs, temp_dirs): RunContext,
) -> Result<Value, Diagnostic> {
    let ir = Rc::new(ir);
    let frame_size = ir.frame_size() as usize;
    if frame.len() < frame_size {
        frame.resize(frame_size, Value::Empty);
    }
    eval::eval_with(&ir, frame, memo, (exec_result, dirs, Some(temp_dirs)))
}

/// What [`run_input`] runs an input with besides its frame: the memo tables
/// of every input, the layout of what `exec` returns, the table of the
/// run's directories, and the temporary directories of the run's owner.
pub(crate) type RunContext<'a> = (
    &'a mut MemoTables,
    Option<&'a Rc<Layout>>,
    &'a [Rc<str>],
    &'a TempDirs,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::Builtin;
    use crate::diagnostic::Severity;

    fn eval(session: &mut Session, text: &str) -> Result<Outcome, Vec<Diagnostic>> {
        session.eval(&SourceFile::new("<test>", text).unwrap())
    }

    fn value(session: &mut Session, text: &str) -> Value {
        match eval(session, text) {
            Ok(outcome) => outcome.value,
            Err(diagnostics) => panic!("{text}: unexpected errors {diagnostics:?}"),
        }
    }

    fn messages(session: &mut Session, text: &str) -> Vec<String> {
        eval(session, text)
            .unwrap_err()
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    /// The number of the first input of a session's own: the standard
    /// files take the numbers before it.
    fn first() -> u32 {
        u32::try_from(crate::STANDARD.len()).unwrap()
    }

    /// The number of slots of the frame the standard library takes, the
    /// first, before any input of a session's own.
    fn standard_slots() -> usize {
        Session::new().frame.len()
    }

    /// The type of the last successful input's value, as shown.
    fn value_type(session: &Session) -> String {
        session
            .types()
            .display(session.value_type(), session.names())
            .to_string()
    }

    #[test]
    fn an_input_with_syntax_errors_reports_them_all_and_commits_nothing() {
        let mut session = Session::new();
        // Every syntax error, and nothing of the input runs or is kept: not
        // even `y`, which parsed.
        let marker = std::env::temp_dir().join(format!("gcsl-recovery-{}", std::process::id()));
        let text = format!(
            "x = 1 2\ny = 3\nz = sh(\"touch {}\")\nw = (4 4)",
            marker.display()
        );
        assert_eq!(
            messages(&mut session, &text),
            ["expected end of item, found `2`", "expected `)`, found `4`"]
        );
        assert!(!marker.exists());
        assert_eq!(messages(&mut session, "y"), ["undefined variable `y`"]);
        assert_eq!(value(&mut session, "= 1 + 1"), Value::Int(2));
    }

    #[test]
    fn def_types_and_their_definitions_persist() {
        let mut session = Session::new();
        // One input declares a def-type, defines with it and invokes it;
        // its value has no namespace.
        assert_eq!(
            value(
                &mut session,
                "#[def_type]\n\
                 def build(name: String, f: fn(t: String): String): String { f(name) }\n\
                 def build x(t) { \"x of {t}\" }\n\
                 a = build x()"
            )
            .to_string(),
            "{build = fn(name: String, f: fn(t: String): String): String; a = \"x of x\"}"
        );
        // Later inputs see the def-type and its definitions, define more,
        // and invoke both.
        assert_eq!(value(&mut session, "build x()").to_string(), "x of x");
        assert_eq!(
            value(&mut session, "def build y(t) { \"y of {t}\" }\nbuild y()").to_string(),
            "y of y"
        );
        // A later input's namespace stands for the def-type's whole, as a
        // later input's dotted root does.
        assert_eq!(
            messages(&mut session, "build x()"),
            ["`build` has no definition `x`"]
        );
        assert_eq!(
            value(&mut session, "let f = build y\nf()").to_string(),
            "y of y"
        );
        // A later input's binding of the def-type's name ends it.
        assert_eq!(value(&mut session, "build = 1").to_string(), "{build = 1}");
        assert_eq!(
            messages(&mut session, "build y()"),
            ["`build` is not a def-type"]
        );
    }

    #[test]
    fn a_definition_and_its_meta_are_of_one_input() {
        let mut session = Session::new();
        // One input declares a def-type whose definitions carry a meta, and a
        // definition with its meta; a later input invokes it.
        let first = "#[def_type]\n\
                     def build(name: String, f: fn(): String META->> {tag: String}) -> String {\n\
                     \x20   \"{f()} {f.tag}\"\n\
                     }\n\
                     defmeta build x { tag = \"tx\" }\n\
                     def build x() { \"x\" }";
        assert!(matches!(value(&mut session, first), Value::Record(_)));
        assert_eq!(value(&mut session, "build x()").to_string(), "x tx");
        // A later input defines another, with its meta, and keeps a function
        // of it, which an input after it calls.
        assert_eq!(
            value(
                &mut session,
                "defmeta build y = { tag = \"ty\" }\ndef build y() { \"y\" }\nlet g = build y\ng()"
            )
            .to_string(),
            "y ty"
        );
        assert_eq!(value(&mut session, "g()").to_string(), "y ty");
        // A definition alone in an input, or a meta alone, is an error: each
        // input is checked on its own. The meta is shown by its type, the
        // def-type's declaration being an earlier input's.
        let errors = eval(&mut session, "def build z() { \"z\" }").unwrap_err();
        assert_eq!(
            errors[0].message,
            "`build z` has no `defmeta` in its directory"
        );
        assert_eq!(
            errors[0].notes,
            ["`build` declares a meta, `META->> {tag: String}`, for each of its definitions"]
        );
        assert_eq!(
            messages(&mut session, "defmeta build w { tag = \"w\" }"),
            ["`defmeta build w` has no definition `w` in its directory"]
        );
        // The failed inputs changed nothing.
        assert_eq!(value(&mut session, "g()").to_string(), "y ty");
    }

    #[test]
    fn a_reference_reads_its_meta_in_later_inputs() {
        let mut session = Session::new();
        let first = "#[def_type]\n\
                     def build(name: String, f: fn(): String META: {tag: String}): String {\n\
                     \x20   \"{f()} {f.tag}\"\n\
                     }\n\
                     defmeta build dev.x { tag = \"tx\" }\n\
                     def build dev.x() { \"x\" }\n\
                     own = build dev.x.tag";
        assert_eq!(
            value(&mut session, first).to_string(),
            "{build = fn(name: String | Future<String>, f: fn(): String META: {tag: String}): \
             String; own = \"tx\"}"
        );
        // An earlier input's definition, by a path past it, in parentheses
        // and stored, its type carrying the meta.
        assert_eq!(value(&mut session, "build dev.x.tag").to_string(), "tx");
        assert_eq!(value(&mut session, "(build dev.x).tag").to_string(), "tx");
        assert_eq!(
            value(&mut session, "let r = build dev.x\nr").to_string(),
            "fn(): String META: {tag: String}"
        );
        assert_eq!(value(&mut session, "r.tag").to_string(), "tx");
        assert_eq!(
            messages(&mut session, "build dev.x.nope"),
            ["the meta of `build dev.x` has no field `nope`"]
        );
        assert_eq!(
            messages(&mut session, "build dev.y.tag"),
            ["`build` has no definition `dev.y.tag`"]
        );
    }

    #[test]
    fn types_persist_and_later_inputs_shadow_them() {
        let mut session = Session::new();
        // An input of only a type shows nothing, as one of only a `fn`.
        assert_eq!(value(&mut session, "type Foo = Int"), Value::Empty);
        assert_eq!(value(&mut session, "fn f(): Int { 1 }"), Value::Empty);
        assert_eq!(value(&mut session, "let x: Foo = 1\nx"), Value::Int(1));
        // A later input's `type Foo` shadows the earlier one, as a later
        // `fn` does, for itself and every input after it.
        assert_eq!(
            value(&mut session, "type Foo = String\nlet y: Foo = \"a\"\ny").to_string(),
            "a"
        );
        assert_eq!(
            value(&mut session, "let z: Foo = \"b\"\nz").to_string(),
            "b"
        );
        assert_eq!(
            messages(&mut session, "let w: Foo = 1"),
            ["mismatched types"]
        );
        // A type built from an earlier input's, by spread.
        assert_eq!(value(&mut session, "type R = {a: Foo}"), Value::Empty);
        assert_eq!(
            value(
                &mut session,
                "type S = {...R, b: Int}\nlet s: S = { a = \"x\"; b = 1 }\ns.b"
            ),
            Value::Int(1)
        );
        // A failed input's types are forgotten with it.
        assert_eq!(
            messages(&mut session, "type Bad = {a: Nope}"),
            ["unknown type `Nope`"]
        );
        assert_eq!(
            messages(&mut session, "let b: Bad = { a = 1 }"),
            ["unknown type `Bad`"]
        );
        assert_eq!(
            messages(&mut session, "type Gone = Int\nlet g: Gone = \"no\""),
            ["mismatched types"]
        );
        assert_eq!(
            messages(&mut session, "let g: Gone = 1"),
            ["unknown type `Gone`"]
        );
        // So are those of one that checks but fails as it runs.
        assert!(eval(&mut session, "type Late = Int\nlet l: Late = 1 // 0").is_err());
        assert_eq!(
            messages(&mut session, "let l: Late = 1"),
            ["unknown type `Late`"]
        );
        // A type and a value of one name are apart, across inputs too.
        assert_eq!(value(&mut session, "Foo = 3").to_string(), "{Foo = 3}");
        assert_eq!(
            value(&mut session, "let v: Foo = \"c\"\nFoo").to_string(),
            "3"
        );
    }

    #[test]
    fn an_import_in_a_session_needs_a_workspace() {
        let mut session = Session::new();
        assert_eq!(
            messages(&mut session, "import lib"),
            vec![
                "`lib` is not a built-in or standard module, and there is no `workspace.gcsl` \
                 above this file"
            ]
        );
        assert_eq!(value(&mut session, "1 + 1"), Value::Int(2));
    }

    #[test]
    fn a_session_runs_the_standard_library_first() {
        let mut session = Session::new();
        // The standard files are its first inputs, and the built-in
        // functions' cells its first slots, written as the frame was laid
        // out.
        assert_eq!(first(), 3);
        assert_eq!(
            session.source(2).map(SourceFile::name),
            Some("<stdlib>/prelude.gcsl")
        );
        assert!(session.source(first()).is_none());
        assert_eq!(
            &session.frame[..3],
            &[
                Value::Builtin(Builtin::Len),
                Value::Builtin(Builtin::Int),
                Value::Builtin(Builtin::Float)
            ]
        );
        // The prelude's names on the first input.
        assert_eq!(value(&mut session, "len([1, 2, 3])"), Value::Int(3));
        assert_eq!(value(&mut session, "half(int(7.9))"), Value::Float(3.5));
        assert_eq!(value_type(&session), "Float");
        // `builtins` and the prelude by name, in the input that imports
        // them and the later ones.
        assert_eq!(
            value(&mut session, "import builtins\nbuiltins.len([1])"),
            Value::Int(1)
        );
        assert_eq!(
            value(
                &mut session,
                "import len as count from prelude\ncount([1, 2])"
            ),
            Value::Int(2)
        );
        assert_eq!(value(&mut session, "builtins.len([1])"), Value::Int(1));
        // An alias of `len` crosses inputs, as an alias of an alias does.
        value(&mut session, "let l = len");
        value(&mut session, "m = l");
        assert_eq!(value(&mut session, "m([1, 2]) + l([1])"), Value::Int(3));
        assert_eq!(value_type(&session), "Int");
        assert_eq!(
            messages(&mut session, "[m]"),
            ["`len` is a built-in function and cannot be used as a value yet"]
        );
        // A failed input never forgets the standard library.
        assert_eq!(
            messages(&mut session, "let bad = 1 + true"),
            ["cannot add Int and Bool"]
        );
        assert_eq!(value(&mut session, "len([1])"), Value::Int(1));
        // A later input declaring `len` shadows it, as inputs shadow
        // earlier inputs' names.
        value(&mut session, "let len = 10");
        assert_eq!(value(&mut session, "len + 1"), Value::Int(11));
        value(&mut session, "fn len(): Int { 20 }");
        assert_eq!(value(&mut session, "len()"), Value::Int(20));
        // But an earlier input's alias still is `len`.
        assert_eq!(value(&mut session, "l([1, 2, 3])"), Value::Int(3));
    }

    #[test]
    fn a_conflict_with_the_prelude_names_its_file() {
        let mut session = Session::new();
        let diagnostics = eval(&mut session, "import * from prelude\nlen = 3").unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        let error = &diagnostics[0];
        assert_eq!(error.message, "field `len` is defined in two files");
        assert_eq!(error.input, None);
        let inputs: Vec<Option<u32>> = error.labels.iter().map(|label| label.input).collect();
        assert_eq!(inputs, [Some(2), None]);
        let prelude = session.source(2).unwrap();
        assert_eq!(prelude.slice(error.labels[0].span), "len");
    }

    #[test]
    fn a_whole_import_persists_for_later_inputs() {
        let mut session = Session::new();
        value(&mut session, "import builtins");
        assert_eq!(value(&mut session, "builtins.float(1)"), Value::Float(1.0));
        assert_eq!(value_type(&session), "Float");
        value(&mut session, "import builtins as b\nimport prelude");
        assert_eq!(value(&mut session, "b.int(2.5)"), Value::Int(2));
        assert_eq!(value(&mut session, "prelude.half(3)"), Value::Float(1.5));
        // Read from a function of a later input, and still after it.
        value(&mut session, "fn size(xs: Int[]): Int { builtins.len(xs) }");
        assert_eq!(value(&mut session, "size([1, 2, 3])"), Value::Int(3));
        assert_eq!(value(&mut session, "builtins.len([1])"), Value::Int(1));
        // The alias rule holds of it as of the input's own import.
        assert_eq!(
            messages(&mut session, "[builtins.len]"),
            ["`len` is a built-in function and cannot be used as a value yet"]
        );
    }

    #[test]
    fn a_member_import_persists_for_later_inputs() {
        let mut session = Session::new();
        value(&mut session, "import len as count from prelude");
        assert_eq!(value(&mut session, "count([1, 2, 3])"), Value::Int(3));
        value(&mut session, "import float, int as truncate from builtins");
        assert_eq!(value(&mut session, "float(2)"), Value::Float(2.0));
        assert_eq!(value(&mut session, "truncate(-2.5)"), Value::Int(-2));
        // An alias of it in a later input is `len` still.
        value(&mut session, "let c = count");
        assert_eq!(value(&mut session, "c([1]) + count([2, 3])"), Value::Int(3));
    }

    #[test]
    fn a_star_import_persists_for_later_inputs() {
        let mut session = Session::new();
        value(&mut session, "import * from builtins");
        assert_eq!(value(&mut session, "int(2.5) + len([1])"), Value::Int(3));
        value(&mut session, "import * from prelude");
        assert_eq!(value(&mut session, "half(3)"), Value::Float(1.5));
        // A later input's field of one of its names shadows it, with no
        // conflict, as it would the implicit prelude's.
        value(&mut session, "len = 5");
        assert_eq!(value(&mut session, "len * 2"), Value::Int(10));
        assert_eq!(value(&mut session, "half(1)"), Value::Float(0.5));
    }

    #[test]
    fn an_import_and_a_later_declaration_shadow_each_other() {
        let mut session = Session::new();
        // A later `let` shadows an import from its point on, in its own
        // input and after it, as it would an earlier input's variable.
        value(&mut session, "import len as count from prelude");
        assert_eq!(
            value(&mut session, "let x = count([1])\nlet count = 5\nx + count"),
            Value::Int(6)
        );
        assert_eq!(value(&mut session, "count + 1"), Value::Int(6));
        value(&mut session, "import builtins");
        assert_eq!(
            value(
                &mut session,
                "let n = builtins.int(2.5)\nlet builtins = 0\nn + builtins"
            ),
            Value::Int(2)
        );
        assert_eq!(value(&mut session, "builtins"), Value::Int(0));
        // A nearer `let` hides it only where it is in scope.
        value(&mut session, "import int from builtins");
        assert_eq!(value(&mut session, "{ let int = 3; int }"), Value::Int(3));
        assert_eq!(value(&mut session, "int(2.5)"), Value::Int(2));
        // A later import shadows a `let`, in its own input and after it.
        value(&mut session, "let size = 5");
        assert_eq!(
            value(
                &mut session,
                "import len as size from prelude\nsize([1, 2])"
            ),
            Value::Int(2)
        );
        assert_eq!(value(&mut session, "size([1])"), Value::Int(1));
        // And a later import an earlier one.
        value(&mut session, "import int as conv from builtins");
        value(&mut session, "import float as conv from builtins");
        assert_eq!(value(&mut session, "conv(2)"), Value::Float(2.0));
        // A `fn`, a field and a spread's name shadow it too.
        value(&mut session, "import builtins");
        value(&mut session, "fn builtins(): Int { 7 }");
        assert_eq!(value(&mut session, "builtins()"), Value::Int(7));
        value(&mut session, "import int from builtins");
        value(&mut session, "int = \"field\"");
        assert_eq!(value(&mut session, "int"), Value::Str("field".into()));
        value(&mut session, "import float from builtins");
        value(&mut session, "...{ float = 4 }");
        assert_eq!(value(&mut session, "float + 1"), Value::Int(5));
    }

    #[test]
    fn a_failed_input_forgets_its_imports() {
        let mut session = Session::new();
        // One that fails to check, and one that fails as it runs.
        assert_eq!(
            messages(&mut session, "import builtins\n1 + true"),
            ["cannot add Int and Bool"]
        );
        assert_eq!(
            messages(&mut session, "builtins.len([1])"),
            ["undefined variable `builtins`"]
        );
        assert!(eval(&mut session, "import builtins\n1 // 0").is_err());
        assert_eq!(
            messages(&mut session, "builtins.len([1])"),
            ["undefined variable `builtins`"]
        );
        // And a failed input's declaration shadows nothing.
        value(&mut session, "import builtins");
        assert!(eval(&mut session, "let builtins = 1\n1 // 0").is_err());
        assert_eq!(value(&mut session, "builtins.len([1])"), Value::Int(1));
    }

    #[test]
    fn three_inputs_share_a_variable() {
        let mut session = Session::new();
        assert_eq!(value(&mut session, "let x = 1"), Value::Empty);
        assert_eq!(value(&mut session, "let y = x + 1"), Value::Empty);
        assert_eq!(value(&mut session, "x + y"), Value::Int(3));
        assert_eq!(value(&mut session, "x * 10"), Value::Int(10));
    }

    #[test]
    fn rebinding_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let x = 1");
        assert_eq!(value(&mut session, "x := x + 10"), Value::Empty);
        assert_eq!(value(&mut session, "x"), Value::Int(11));
        // A rebind keeps the variable's type; changing it takes a `let`.
        assert_eq!(messages(&mut session, "x := 2.5"), vec!["mismatched types"]);
        assert_eq!(value(&mut session, "x"), Value::Int(11));
        // Only at the top level: an input's block is an inner block.
        assert_eq!(
            messages(&mut session, "{ x := 1.5 }"),
            vec!["cannot rebind `x` here: it is declared in an enclosing block"]
        );
        assert_eq!(
            messages(&mut session, "y := 1"),
            vec!["cannot rebind `y`: no variable named `y` is declared"]
        );
    }

    #[test]
    fn shadowing_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let x = 1");
        value(&mut session, "let x = 2.5");
        assert_eq!(value(&mut session, "x"), Value::Float(2.5));
        assert_eq!(value(&mut session, "{ let x = 7; x }"), Value::Int(7));
        assert_eq!(value(&mut session, "x"), Value::Float(2.5));
    }

    #[test]
    fn a_failing_input_leaves_earlier_variables_intact() {
        let mut session = Session::new();
        value(&mut session, "let x = 5");
        let names = session.names.len();
        let scope = session.scope.clone();

        // A type error, a syntax error, a lexical error and a runtime error.
        assert_eq!(
            messages(&mut session, "let fresh = x + 1.5"),
            vec!["cannot add Int and Float"]
        );
        assert_eq!(
            messages(&mut session, "let other ="),
            vec!["expected expression, found end of input"]
        );
        assert_eq!(
            messages(&mut session, "let third = x-"),
            vec!["`-` here is ambiguous"]
        );
        assert_eq!(
            messages(&mut session, "let y = 1\nlet z = 1 // 0\nx := 0"),
            vec!["division by zero"]
        );
        // A quoted field name interned by a failed parse is forgotten too.
        assert_eq!(
            messages(&mut session, "\"quoted\" = 1 +"),
            vec!["expected expression, found end of input"]
        );
        assert_eq!(session.names.len(), names);
        assert_eq!(session.scope, scope);

        // Nothing the failed inputs declared exists, and `x` still holds its
        // value even though the runtime failure wrote the slot after it.
        assert_eq!(
            messages(&mut session, "fresh"),
            vec!["undefined variable `fresh`"]
        );
        assert_eq!(messages(&mut session, "y"), vec!["undefined variable `y`"]);
        assert_eq!(value(&mut session, "x"), Value::Int(5));
        assert_eq!(value(&mut session, "let w = 7\nw + x"), Value::Int(12));
    }

    #[test]
    fn the_frame_grows_as_needed() {
        let mut session = Session::new();
        let base = standard_slots();
        value(&mut session, "let a = 1");
        assert_eq!(session.frame.len(), base + 1);
        value(&mut session, "{ let t = 1; let u = 2; let v = 3 }");
        // After each input, the stack is its top level's frame: the
        // temporaries' slots are gone, and reused by later variables, but
        // the capacity stays.
        assert_eq!(session.frame.len(), base + 4);
        value(&mut session, "let b = 2");
        assert_eq!(session.frame.len(), base + 2);
        assert!(session.frame.capacity() >= base + 4);
        assert_eq!(value(&mut session, "a + b"), Value::Int(3));
    }

    #[test]
    fn inputs_have_values_of_every_type() {
        let mut session = Session::new();
        assert_eq!(value(&mut session, ""), Value::Empty);
        assert_eq!(value_type(&session), "{}");
        assert_eq!(value(&mut session, "7 / 2"), Value::Float(3.5));
        assert_eq!(value_type(&session), "Float");
        assert_eq!(value(&mut session, "= 1 + 1"), Value::Int(2));
        assert_eq!(value_type(&session), "Int");
        assert_eq!(value(&mut session, "let u = {}\nu"), Value::Empty);
        assert_eq!(value(&mut session, "1 < 2"), Value::Bool(true));
        assert_eq!(
            value(&mut session, "\"a\" + \"b\""),
            Value::Str("ab".into())
        );
        assert_eq!(
            value(&mut session, "{ x = 1; y = \"a\" }").to_string(),
            "{x = 1; y = \"a\"}"
        );
        assert_eq!(value_type(&session), "{x: Int, y: String}");
        // A failed input leaves the last type as it was.
        messages(&mut session, "q");
        assert_eq!(value_type(&session), "{x: Int, y: String}");
    }

    #[test]
    fn strings_and_bools_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let greeting = \"hello\"");
        value(&mut session, "let loud = true");
        // The first input's string table is gone; its string lives on in
        // the frame.
        assert_eq!(
            value(&mut session, "\"{greeting}, world\""),
            Value::Str("hello, world".into())
        );
        value(&mut session, "greeting := greeting + \"!\"");
        assert_eq!(
            value(&mut session, "if loud { greeting } else { \"quiet\" }"),
            Value::Str("hello!".into())
        );
        assert_eq!(
            value(&mut session, "greeting == \"hello!\""),
            Value::Bool(true)
        );
        // A failing input leaves string variables intact.
        assert_eq!(
            messages(&mut session, "greeting := greeting + 1"),
            vec!["cannot add String and Int"]
        );
        assert_eq!(value(&mut session, "greeting"), Value::Str("hello!".into()));
    }

    #[test]
    fn records_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let p = { x = 1; inner = { s = \"a\" } }");
        assert_eq!(value(&mut session, "p.x + 1"), Value::Int(2));
        assert_eq!(value(&mut session, "p.inner.s"), Value::Str("a".into()));
        // The type of `p` is the same type in a later input, so equality
        // with a record built there type-checks.
        assert_eq!(
            value(&mut session, "p == { x = 1; inner = { s = \"a\" } }"),
            Value::Bool(true)
        );
        assert_eq!(
            value(&mut session, "let q: {x: Int} = p\nq").to_string(),
            "{x = 1; inner = {s = \"a\"}}"
        );
        assert_eq!(value_type(&session), "{x: Int}");
        // Top-level fields are in scope for later inputs, as fields.
        value(&mut session, "n = 41\nm = n + 1");
        assert_eq!(value(&mut session, "m"), Value::Int(42));
        assert_eq!(
            messages(&mut session, "n := 0"),
            vec!["cannot rebind `n`: it is a field, not a variable"]
        );
        // A failed input's types stay in the table, harmlessly.
        let types = session.types.len();
        messages(&mut session, "let r = { brand-new = 1 }\nr.missing");
        assert!(session.types.len() > types);
        assert_eq!(value(&mut session, "{ other = 2 }.other"), Value::Int(2));
    }

    #[test]
    fn dotted_fields_and_updates_across_inputs() {
        let mut session = Session::new();
        // An input's merged field is one top-level field, visible later with
        // its full type.
        value(&mut session, "server.host = \"h\"\nserver.port = 80");
        assert_eq!(value_type(&session), "{server: {host: String, port: Int}}");
        assert_eq!(value(&mut session, "server.port"), Value::Int(80));
        // A later input's dotted field is a field of that input's record,
        // declared anew, so it builds from nothing.
        assert_eq!(
            value(&mut session, "server.tls = true").to_string(),
            "{server = {tls = true}}"
        );
        assert_eq!(value(&mut session, "server").to_string(), "{tls = true}");
        // A dotted `let` updates a record from an earlier input, which is
        // left as it was for anything holding it.
        value(
            &mut session,
            "let config = { name = \"gcsl\"; build = { jobs = 4 } }",
        );
        value(&mut session, "let before = config");
        value(&mut session, "let config.build.jobs = 8");
        value(&mut session, "let config.build.dry-run = false");
        value(&mut session, "config.build.dry-run := true");
        assert_eq!(
            value(&mut session, "config").to_string(),
            "{name = \"gcsl\"; build = {jobs = 8; dry-run = true}}"
        );
        assert_eq!(
            value_type(&session),
            "{build: {dry-run: Bool, jobs: Int}, name: String}"
        );
        assert_eq!(
            value(&mut session, "before").to_string(),
            "{name = \"gcsl\"; build = {jobs = 4}}"
        );
        // A field of an earlier input cannot be rebound, dotted or not.
        assert_eq!(
            messages(&mut session, "server.tls := false"),
            vec!["cannot rebind `server`: it is a field, not a variable"]
        );
        // A failed update leaves the variable as it was.
        assert_eq!(
            messages(&mut session, "let config.name.first = 1"),
            vec!["cannot update `config.name.first`: `config.name` is String, not a record"]
        );
        assert_eq!(value(&mut session, "config.build.jobs"), Value::Int(8));
    }

    #[test]
    fn functions_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "fn double(n: Int): Int { n * 2 }");
        value(&mut session, "let x = 10");
        value(&mut session, "let get = fn(): Int { x }");
        // A function of an earlier input is called by a later one, and
        // captured what was in scope when it was made.
        assert_eq!(value(&mut session, "double(21)"), Value::Int(42));
        value(&mut session, "x := 20");
        assert_eq!(value(&mut session, "get()"), Value::Int(10));
        assert_eq!(value(&mut session, "x"), Value::Int(20));
        // Its value shows as its type.
        assert_eq!(value(&mut session, "double").to_string(), "fn(n: Int): Int");
        assert_eq!(value_type(&session), "fn(n: Int): Int");
        // A closure made by one input and passed to a function of another.
        value(
            &mut session,
            "fn apply(f: fn(Int): Int, v: Int): Int { f(v) }",
        );
        assert_eq!(
            value(&mut session, "apply(fn(n: Int): Int { n + x }, 1)"),
            Value::Int(21)
        );
    }

    #[test]
    fn top_level_functions_across_inputs() {
        let mut session = Session::new();
        let even = "fn even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }";
        let odd = "fn odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }";
        value(&mut session, &format!("let a = 1\n{odd}\n{even}"));
        // The frame is the input's variables plus a slot for each function.
        assert_eq!(session.frame.len(), standard_slots() + 3);
        // A later input calls the earlier input's mutually recursive pair.
        assert_eq!(value(&mut session, "even(10)"), Value::Bool(true));
        // Declaring one of them again leaves the earlier pair calling each
        // other: the earlier closures read their own input's slots.
        value(&mut session, "fn odd(n: Int): Bool { true }");
        assert_eq!(value(&mut session, "even(3)"), Value::Bool(false));
        assert_eq!(value(&mut session, "odd(4)"), Value::Bool(true));
        // A function shadows an earlier input's variable, and the reverse.
        value(&mut session, "let three = 3");
        value(&mut session, "let x = three()\nfn three(): Int { 4 }");
        assert_eq!(value(&mut session, "x"), Value::Int(4));
        value(&mut session, "let even = 5");
        assert_eq!(value(&mut session, "even + 1"), Value::Int(6));
        // A failed input leaves the session as it was.
        let scope = session.scope.clone();
        assert_eq!(
            messages(&mut session, "let y = f(0)\nfn f(n: Int): Int { 1 // n }"),
            vec!["division by zero"]
        );
        assert_eq!(session.scope, scope);
        assert_eq!(messages(&mut session, "f"), vec!["undefined variable `f`"]);
        assert_eq!(value(&mut session, "odd(4)"), Value::Bool(true));
    }

    #[test]
    fn top_level_fields_across_inputs() {
        let mut session = Session::new();
        value(
            &mut session,
            "com.x.app = com.x.lib.name + \"!\"\ncom.x.lib.name = \"lib\"\n\
             def get(): String { com.x.lib.name }",
        );
        // The frame is a slot for the root `com`, one for `get`, and a cell
        // for each of `com.x`, `com.x.app`, `com.x.lib` and `com.x.lib.name`.
        assert_eq!(session.frame.len(), standard_slots() + 6);
        // A later input reads the earlier input's root, a leaf by its path,
        // and its `def`, which reads the leaf's cell through the frame.
        assert_eq!(
            value(&mut session, "com").to_string(),
            "{x = {app = \"lib!\"; lib = {name = \"lib\"}}}"
        );
        assert_eq!(
            value_type(&session),
            "{x: {app: String, lib: {name: String}}}"
        );
        assert_eq!(
            value(&mut session, "com.x.lib.name"),
            Value::Str("lib".into())
        );
        assert_eq!(value(&mut session, "get()"), Value::Str("lib".into()));
        // The cells are committed with their input: later variables take
        // other slots, and the `def` still reads its own input's cell.
        value(&mut session, "let p = 1\nlet q = 2\nlet r = 3\nlet s = 4");
        assert_eq!(session.frame.len(), standard_slots() + 10);
        assert_eq!(value(&mut session, "get()"), Value::Str("lib".into()));
        // A dotted field of a later input declares that input's own root.
        value(&mut session, "com.y = 1");
        assert_eq!(value(&mut session, "com").to_string(), "{y = 1}");
        assert_eq!(value(&mut session, "get()"), Value::Str("lib".into()));
        // A failed input leaves the session as it was, its cells forgotten.
        let scope = session.scope.clone();
        assert_eq!(
            messages(&mut session, "t.b = t.a // 0\nt.a = 1"),
            vec!["division by zero"]
        );
        assert_eq!(session.scope, scope);
        assert_eq!(messages(&mut session, "t"), vec!["undefined variable `t`"]);
        assert_eq!(value(&mut session, "get()"), Value::Str("lib".into()));
    }

    #[test]
    fn dotted_defs_across_inputs() {
        let mut session = Session::new();
        value(
            &mut session,
            "def dev.peios.count(n: Int): Int { if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) } }\n\
             def dev.peios.hello(): String { \"hello {dev.peios.name}\" }\n\
             dev.peios.name = \"world\"",
        );
        // A slot for the root `dev`, and a cell for each of `dev.peios`, its
        // two functions, which hold their closures, and `name`.
        assert_eq!(session.frame.len(), standard_slots() + 5);
        assert_eq!(
            value_type(&session),
            "{dev: {peios: {count: fn(n: Int): Int, hello: fn(): String, name: String}}}"
        );
        // A later input sees them, as it sees dotted fields, by path or
        // through the root's record.
        assert_eq!(value(&mut session, "dev.peios.count(3)"), Value::Int(3));
        assert_eq!(
            value(&mut session, "let r = dev\nr.peios.hello()"),
            Value::Str("hello world".into())
        );
        // A later input's dotted `def` declares that input's own root, and
        // the earlier functions keep reading their own input's cells.
        value(&mut session, "def dev.other(): Int { 1 }");
        assert_eq!(
            value(&mut session, "dev").to_string(),
            "{other = fn(): Int}"
        );
        // It has no name: `count` is no variable.
        assert_eq!(
            messages(&mut session, "count(1)"),
            vec!["undefined variable `count`"]
        );
        // A failed input leaves the session as it was.
        let scope = session.scope.clone();
        assert_eq!(
            messages(&mut session, "def d.f(n: Int): Int { 1 // n }\nx = d.f(0)"),
            vec!["division by zero"]
        );
        assert_eq!(session.scope, scope);
        assert_eq!(messages(&mut session, "d"), vec!["undefined variable `d`"]);
    }

    #[test]
    fn an_error_in_a_function_of_an_earlier_input_names_that_input() {
        let mut session = Session::new();
        let first_text = "fn div(a: Int, b: Int): Int { a // b }";
        value(&mut session, first_text);
        value(&mut session, "let unrelated = 1");
        // Input 2 fails at runtime inside input 0's function (counting from
        // the session's own first input, after the standard files').
        let diagnostics = eval(&mut session, "div(1, 0)").unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.message, "division by zero");
        assert_eq!(diagnostic.input, Some(first()));
        let source = session.source(first()).unwrap();
        assert_eq!(source.text(), first_text);
        assert_eq!(source.slice(diagnostic.span), "//");
        assert_eq!(
            session.source(first() + 1).unwrap().text(),
            "let unrelated = 1"
        );
        // The failed input takes no number, and the one being run has none
        // yet.
        assert!(session.source(first() + 2).is_none());
        // An error in the input being run names no input, even from inside
        // a closure of its own called by a function of an earlier input.
        value(&mut session, "fn call(f: fn(): Int): Int { f() }");
        let diagnostics = eval(&mut session, "call(fn(): Int { 1 // 0 })").unwrap_err();
        assert_eq!(diagnostics[0].input, None);
        // The innermost foreign function names its input: input 0's `div`,
        // called by input 3's `bad`, called by input 2's `call`.
        value(&mut session, "fn bad(): Int { div(1, 0) }");
        let diagnostics = eval(&mut session, "call(bad)").unwrap_err();
        assert_eq!(diagnostics[0].input, Some(first()));
        let diagnostics = eval(&mut session, "bad()").unwrap_err();
        assert_eq!(diagnostics[0].input, Some(first()));
        // A file names no input: everything in it is input 0.
        let file = SourceFile::new("f.gcsl", "fn f(): Int { 1 // 0 }\nf()").unwrap();
        assert_eq!(crate::run(&file).unwrap_err()[0].input, None);
    }

    #[test]
    fn futures_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let r = { a = 20 }");
        value(&mut session, "let f = lazy r.a + 1");
        // The future holds a copy of the record until it has run.
        let Value::Record(record) = session.frame[standard_slots()].clone() else {
            panic!("expected a record");
        };
        assert_eq!(Rc::strong_count(&record), 3);
        // It shows as its type, and is run by a later input.
        assert_eq!(value(&mut session, "f").to_string(), "Future<Int>");
        assert_eq!(value_type(&session), "Future<Int>");
        assert_eq!(value(&mut session, "f * 2"), Value::Int(42));
        // Having run, it has dropped its captures, and keeps its value.
        assert_eq!(Rc::strong_count(&record), 2);
        assert_eq!(value(&mut session, "await f"), Value::Int(21));
        assert_eq!(value_type(&session), "Int");
    }

    #[test]
    fn a_failing_future_names_its_input_and_runs_again() {
        let mut session = Session::new();
        let first_text = "let bad = lazy 1 // 0";
        value(&mut session, first_text);
        // Awaited by a later input, it fails in the input that made it,
        // whose source the spans refer to; there is no await of that input
        // to label.
        for _ in 0..2 {
            let diagnostics = eval(&mut session, "await bad").unwrap_err();
            assert_eq!(diagnostics.len(), 1);
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.message, "division by zero");
            assert_eq!(diagnostic.input, Some(first()));
            assert_eq!(
                session.source(first()).unwrap().slice(diagnostic.span),
                "//"
            );
            assert!(
                diagnostic
                    .labels
                    .iter()
                    .all(|label| label.text != "awaited here")
            );
        }
        // Left unrun, so each await runs it again; one of the input that
        // made it, awaited there, is labelled.
        let diagnostics = eval(&mut session, "let again = lazy 1 // 0\nawait again").unwrap_err();
        assert_eq!(diagnostics[0].input, None);
        assert!(
            diagnostics[0]
                .labels
                .iter()
                .any(|label| label.text == "awaited here")
        );
    }

    #[test]
    fn a_memo_entry_of_one_input_is_found_by_a_later_one() {
        let mut session = Session::new();
        value(&mut session, "fn f(x: Int) ->> {v: Int} { v = x }");
        let Value::Record(first) = value(&mut session, "f(1)") else {
            panic!("expected a record");
        };
        value(&mut session, "let unrelated = 1");
        let Value::Record(again) = value(&mut session, "f(1)") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&first, &again));
        let Value::Record(other) = value(&mut session, "f(2)") else {
            panic!("expected a record");
        };
        assert!(!Rc::ptr_eq(&first, &other));
        assert_eq!(session.memo.table_count(), 1);
    }

    #[test]
    fn a_failed_input_forgets_what_it_memoised() {
        let mut session = Session::new();
        value(&mut session, "fn f(r: {x: Int}) ->> {v: Int} { v = r.x }");
        let Value::Record(first) = value(&mut session, "f({ x = 1 })") else {
            panic!("expected a record");
        };
        // The failed input's entries go, with the table it made for `g`;
        // those of earlier inputs stay.
        assert_eq!(
            messages(
                &mut session,
                "fn g(x: Int) ->> Int { x }\nlet a = g(1)\nlet r = { x = 2; zzz = 3 }\n\
                 let b = f(r)\nlet h = 1 // 0"
            ),
            vec!["division by zero"]
        );
        assert_eq!(session.memo.table_count(), 1);
        let Value::Record(again) = value(&mut session, "f({ x = 1 })") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&first, &again));
        // A name the failed input interned is another input's now: a key
        // holding it would be found by arguments it does not equal.
        let Value::Record(other) = value(&mut session, "let q = { x = 2; qqq = 3 }\nf(q)") else {
            panic!("expected a record");
        };
        assert_eq!(Rc::strong_count(&other), 2);
        let Value::Record(third) = value(&mut session, "let z = { x = 2; zzz = 3 }\nf(z)") else {
            panic!("expected a record");
        };
        assert!(!Rc::ptr_eq(&other, &third));
    }

    #[test]
    fn a_memoised_call_that_fails_runs_again_in_a_later_input() {
        let mut session = Session::new();
        let first_text = "fn d(x: Int) ->> Int { 10 // x }";
        value(&mut session, first_text);
        for _ in 0..2 {
            let diagnostics = eval(&mut session, "d(0)").unwrap_err();
            assert_eq!(diagnostics[0].message, "division by zero");
            // In the body, which ran again.
            assert_eq!(
                session.source(first()).unwrap().slice(diagnostics[0].span),
                "//"
            );
            assert_eq!(session.memo.running_count(), 0);
        }
        assert_eq!(value(&mut session, "d(5)"), Value::Int(2));
        // A call that depends on its own result, likewise: no running mark
        // is left, so the next input runs the body again, and fails in it,
        // at the inner call, with the outer one labelled.
        let text = "fn r(n: Int) ->> Int { if n == 0 { r(0) } else { n } }";
        value(&mut session, text);
        for _ in 0..2 {
            let diagnostics = eval(&mut session, "r(0)").unwrap_err();
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.message, "this call depends on its own result");
            assert_eq!(diagnostic.input, Some(first() + 2));
            assert_eq!(
                session.source(first() + 2).unwrap().slice(diagnostic.span),
                "r(0)"
            );
            // The call running it is of the input being run, and the
            // error's spans are of the function's: it is not labelled.
            assert!(diagnostic.labels.is_empty());
            assert_eq!(session.memo.running_count(), 0);
        }
        assert_eq!(value(&mut session, "r(4)"), Value::Int(4));
    }

    #[test]
    fn loops_over_a_session_variable() {
        let mut session = Session::new();
        value(&mut session, "let mut n = 0");
        // A later input's loop rebinds it from its condition loop's body,
        // and its header.
        value(&mut session, "for n < 3 { n := n + 1 }");
        assert_eq!(value(&mut session, "n"), Value::Int(3));
        value(&mut session, "for n < 6; n := n + 1 { }");
        assert_eq!(value(&mut session, "n"), Value::Int(6));
        assert_eq!(
            value(
                &mut session,
                "for { if n > 7 { break n }; n := n + 1; continue }"
            )
            .repr()
            .to_string(),
            "[8]"
        );
        assert_eq!(value_type(&session), "List<Int>");
        // A failed input that loops leaves the session as it was: its
        // rebinds of `n` are undone, and its loop's header variable is not
        // declared.
        assert_eq!(
            messages(
                &mut session,
                "for let i = 0; i < 3; i := i + 1 { n := n + 10; i // 0 }"
            ),
            vec!["division by zero"]
        );
        assert_eq!(value(&mut session, "n"), Value::Int(8));
        assert_eq!(messages(&mut session, "i"), vec!["undefined variable `i`"]);
    }

    #[test]
    fn a_memoised_function_that_breaks_keeps_what_it_recorded() {
        let mut session = Session::new();
        value(
            &mut session,
            "fn f(n: Int) ->> {v: List<Int>} { v = for let i = 0; true; i := i + 1 \
             { if i == n { break }; i } }",
        );
        let Value::Record(first) = value(&mut session, "f(2)") else {
            panic!("expected a record");
        };
        // A later input finds the entry the `break` in the body did not
        // disturb, and so does one after an input that failed.
        messages(&mut session, "let a = f(3)\nlet b = 1 // 0");
        let Value::Record(again) = value(&mut session, "f(2)") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&first, &again));
        assert_eq!(session.memo.running_count(), 0);
    }

    #[test]
    fn a_function_that_returns_serves_later_inputs() {
        let mut session = Session::new();
        value(
            &mut session,
            "fn find(xs: List<Int>, t: Int): Int { for x in xs { if x == t { return x * 10 } }; -1 }",
        );
        assert_eq!(value(&mut session, "find([1, 2, 3], 2)"), Value::Int(20));
        assert_eq!(value(&mut session, "find([1], 5)"), Value::Int(-1));
        // A memoised one that returns early keeps its entry across inputs,
        // and across a failed one.
        value(
            &mut session,
            "fn m(n: Int) ->> {v: Int} { if n > 0 { return { v = n } }; { v = 0 } }",
        );
        let Value::Record(first) = value(&mut session, "m(2)") else {
            panic!("expected a record");
        };
        messages(&mut session, "let a = m(3)\nlet b = 1 // 0");
        let Value::Record(again) = value(&mut session, "m(2)") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&first, &again));
        assert_eq!(session.memo.running_count(), 0);
        // A failed input with a `return` and a labelled block leaves the
        // session as it was.
        value(&mut session, "let mut n = 0");
        assert_eq!(
            messages(
                &mut session,
                "let k = {'b n := 5; if n > 0 { return'b n // 0 }; 1 }"
            ),
            vec!["division by zero"]
        );
        assert_eq!(value(&mut session, "n"), Value::Int(0));
        assert_eq!(messages(&mut session, "k"), vec!["undefined variable `k`"]);
        // A `return` at the top of an input is an error, as in a file.
        assert_eq!(
            messages(&mut session, "return 1"),
            vec!["`return` outside a function"]
        );
        assert_eq!(value(&mut session, "{'b return'b 4 }"), Value::Int(4));
    }

    #[test]
    fn lists_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let xs = [1, 2, 3]");
        assert_eq!(value(&mut session, "xs[2]"), Value::Int(3));
        assert_eq!(value_type(&session), "Int");
        value(&mut session, "let ys: Int[] = [...xs, 4]");
        assert_eq!(value(&mut session, "len(ys)"), Value::Int(4));
        assert_eq!(value(&mut session, "ys").to_string(), "[1, 2, 3, 4]");
        assert_eq!(value_type(&session), "List<Int>");
        // A function of one input takes a list of a later one.
        value(
            &mut session,
            "fn total(xs: Int[]): Int { xs[0] + xs[len(xs) - 1] }",
        );
        assert_eq!(value(&mut session, "total(ys)"), Value::Int(5));
        // An index out of range in a function of an earlier input names
        // that input.
        let diagnostics = eval(&mut session, "total([])").unwrap_err();
        assert_eq!(diagnostics[0].message, "index out of range");
        assert_eq!(diagnostics[0].input, Some(first() + 5));
        // `len` is in every input's scope, and shadowed as any name.
        value(&mut session, "let len = 5");
        assert_eq!(value(&mut session, "len"), Value::Int(5));
        assert_eq!(
            messages(&mut session, "let n = [1]\nn := [\"a\"]"),
            vec!["mismatched types"]
        );
        // An element that may be a future is awaited deeply when shown.
        value(&mut session, "let zs = [lazy 1, 2]");
        assert_eq!(value(&mut session, "\"{zs}\""), Value::Str("[1, 2]".into()));
    }

    #[test]
    fn fors_and_ranges_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let xs = for i in 0..3 { i * 10 }");
        assert_eq!(value(&mut session, "xs").to_string(), "[0, 10, 20]");
        assert_eq!(value_type(&session), "List<Int>");
        // A `for`'s variable is not a top-level variable, and its slot is
        // free for the next declaration.
        assert_eq!(messages(&mut session, "x"), vec!["undefined variable `x`"]);
        value(
            &mut session,
            "let fs = for x in xs if x > 0 { fn(): Int { x } }",
        );
        value(&mut session, "let y = 7");
        // A closure made in an earlier input keeps its iteration's value.
        assert_eq!(value(&mut session, "fs[0]() + fs[1]() + y"), Value::Int(37));
        // A range too large for any list fails the input, which leaves the
        // session as it was.
        assert_eq!(
            messages(&mut session, "let big = 0..9223372036854775807"),
            vec!["range too large"]
        );
        assert_eq!(
            messages(&mut session, "big"),
            vec!["undefined variable `big`"]
        );
        assert_eq!(value(&mut session, "len(xs)"), Value::Int(3));
        // An error in a `for` in a function of an earlier input names that
        // input.
        value(
            &mut session,
            "fn inverses(xs: Int[]): Int[] { for x in xs { 60 // x } }",
        );
        assert_eq!(
            value(&mut session, "inverses([1, 2])").to_string(),
            "[60, 30]"
        );
        let diagnostics = eval(&mut session, "inverses([1, 0])").unwrap_err();
        assert_eq!(diagnostics[0].message, "division by zero");
        assert_eq!(diagnostics[0].input, Some(first() + 6));
        // A unit-like statement.
        assert_eq!(
            value(&mut session, "for x in xs { }\n[]\nlen(xs)"),
            Value::Int(3)
        );
    }

    #[test]
    fn mutable_variables_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let mut total = 0");
        // Later inputs rebind it from inner blocks, in its slot.
        value(&mut session, "for x in [1, 2, 3] { total := total + x }");
        value(&mut session, "if total > 5 { total := total * 2 }");
        value(&mut session, "{ { total := total + 1 } }");
        assert_eq!(value(&mut session, "total"), Value::Int(13));
        assert_eq!(value_type(&session), "Int");
        assert_eq!(session.frame.len(), standard_slots() + 1);
        // A function or a `lazy` of a later input cannot rebind it, and
        // there is no declaration to point at.
        let diagnostics =
            eval(&mut session, "let f = fn(): Int { total := 1; total }").unwrap_err();
        assert_eq!(
            diagnostics[0].message,
            "cannot rebind `total` here: it is declared outside this function"
        );
        assert!(diagnostics[0].labels.is_empty());
        assert_eq!(
            messages(&mut session, "let l = lazy { total := 1; total }"),
            vec!["cannot rebind `total` here: it is declared outside this `lazy`"]
        );
        // A function of a later input reads it as it was when the function
        // was made.
        value(&mut session, "let get = fn(): Int { total }");
        value(&mut session, "{ total := 100 }");
        assert_eq!(value(&mut session, "get()"), Value::Int(13));
        assert_eq!(value(&mut session, "total"), Value::Int(100));
    }

    #[test]
    fn a_failed_input_leaves_a_mutable_variable_as_it_was() {
        let mut session = Session::new();
        value(&mut session, "let mut n = 1");
        value(&mut session, "let mut conf = { port = 80; host = \"h\" }");
        // Rebound, plainly or along a path, and then failing at runtime.
        for text in [
            "n := 2\nlet z = 1 // 0",
            "{ n := 3 }\n1 // 0",
            "for x in [1, 0] { n := n + 10 // x }",
            "conf.port := 443\nlet z = 1 // 0",
            "if true { conf.port := 8080; conf.host := \"x\" }\n1 // 0",
        ] {
            assert_eq!(
                messages(&mut session, text),
                vec!["division by zero"],
                "{text}"
            );
            assert_eq!(value(&mut session, "n"), Value::Int(1), "{text}");
            assert_eq!(
                value(&mut session, "conf").to_string(),
                "{port = 80; host = \"h\"}",
                "{text}"
            );
        }
        // One that succeeds keeps what it rebound.
        value(&mut session, "{ n := 5; conf.port := 443 }");
        assert_eq!(value(&mut session, "n"), Value::Int(5));
        assert_eq!(value(&mut session, "conf.port"), Value::Int(443));
        // A variable declared after it is restored too, and one that is not
        // mutable was never written.
        value(&mut session, "let fixed = 7\nlet mut m = 8");
        assert_eq!(
            messages(&mut session, "m := 9\nn := 6\nlet q = fixed // 0"),
            vec!["division by zero"]
        );
        assert_eq!(
            value(&mut session, "\"{n} {conf.port} {fixed} {m}\""),
            Value::Str("5 443 7 8".into())
        );
    }

    #[test]
    fn an_element_of_an_earlier_inputs_list_is_updated_by_a_later_one() {
        let mut session = Session::new();
        value(&mut session, "let mut xs = [1, 2, 3]");
        value(
            &mut session,
            "let mut conf = { hosts = [{ up = false }, { up = false }] }",
        );
        value(&mut session, "let old = xs");
        // From the top level, an inner block and a `for`, in its slot.
        value(&mut session, "xs[0] := 10");
        value(&mut session, "{ conf.hosts[1].up := true }");
        value(&mut session, "for i in 1..3 { xs[i] := xs[i] * 2 }");
        assert_eq!(value(&mut session, "xs").to_string(), "[10, 4, 6]");
        assert_eq!(value_type(&session), "List<Int>");
        assert_eq!(
            value(&mut session, "conf").to_string(),
            "{hosts = [{up = false}, {up = true}]}"
        );
        // The list an earlier variable holds is not changed.
        assert_eq!(value(&mut session, "old").to_string(), "[1, 2, 3]");
        // Nor by a later input's function, which cannot rebind it.
        assert_eq!(
            messages(&mut session, "let f = fn(): Int { xs[0] := 1; 0 }"),
            vec!["cannot rebind `xs` here: it is declared outside this function"]
        );
    }

    #[test]
    fn a_failed_index_update_leaves_the_old_list() {
        let mut session = Session::new();
        value(&mut session, "let mut xs = [1, 2]");
        value(&mut session, "let mut grid = [[0], [0]]");
        // Out of range, at the first index or a later one, or failing
        // later in the input, after the update was stored.
        for (text, message) in [
            ("xs[2] := 5", "index out of range"),
            ("xs[-1] := 5", "index out of range"),
            ("grid[0][1] := 5", "index out of range"),
            (
                "{ xs[0] := 5; grid[1][0] := 5 }\nlet z = 1 // 0",
                "division by zero",
            ),
            ("for i in [0, 1, 2] { xs[i] := 9 }", "index out of range"),
        ] {
            assert_eq!(messages(&mut session, text), vec![message], "{text}");
            assert_eq!(value(&mut session, "xs").to_string(), "[1, 2]", "{text}");
            assert_eq!(
                value(&mut session, "grid").to_string(),
                "[[0], [0]]",
                "{text}"
            );
        }
        // One that succeeds keeps what it stored.
        value(&mut session, "xs[1] := 20");
        assert_eq!(value(&mut session, "xs").to_string(), "[1, 20]");
    }

    #[test]
    fn a_mutable_variable_keeps_its_declared_type_across_inputs() {
        let mut session = Session::new();
        value(&mut session, "let mut r: Int | Future<Int> = lazy 1");
        value(&mut session, "{ r := 2 }");
        assert_eq!(value(&mut session, "r"), Value::Int(2));
        assert_eq!(value_type(&session), "Int | Future<Int>");
        assert_eq!(value(&mut session, "r + 1"), Value::Int(3));
        value(
            &mut session,
            "let mut c: {a: Int} | Future<{a: Int}> = lazy { a = 1 }",
        );
        value(&mut session, "{ c.a := 2 }");
        assert_eq!(value(&mut session, "c").to_string(), "{a = 2}");
        assert_eq!(value_type(&session), "{a: Int} | Future<{a: Int}>");
        assert_eq!(value(&mut session, "c.a"), Value::Int(2));
    }

    #[test]
    fn warnings_come_back_with_the_value() {
        let mut session = Session::new();
        let outcome = eval(&mut session, "let p: {x: Int} = { x = 1; y = 2 }\np").unwrap();
        assert_eq!(outcome.value.to_string(), "{x = 1; y = 2}");
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].severity, Severity::Warning);
        assert_eq!(
            outcome.warnings[0].message,
            "field `y` is not in `{x: Int}`"
        );
        // With a runtime error, the warnings come back with it, in source
        // order.
        let diagnostics = eval(&mut session, "let r: {} = { z = 1 // 0 }").unwrap_err();
        let shown: Vec<(Severity, &str)> = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.severity, diagnostic.message.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (Severity::Warning, "field `z` is not in `{}`"),
                (Severity::Error, "division by zero")
            ]
        );
        // No warnings, none returned.
        assert!(eval(&mut session, "p.x").unwrap().warnings.is_empty());
    }

    #[test]
    fn a_spread_at_the_top_level_defines_names_for_later_inputs() {
        let mut session = Session::new();
        value(&mut session, "let defaults = { host = \"h\"; port = 80 }");
        assert_eq!(
            value(&mut session, "...defaults\nport = 8080").to_string(),
            "{host = \"h\"; port = 8080}"
        );
        assert_eq!(value_type(&session), "{host: String, port: Int}");
        // Its fields are in scope for later inputs, as fields, the
        // overridden one as it was overridden.
        assert_eq!(
            value(&mut session, "\"{host}:{port}\""),
            Value::Str("h:8080".into())
        );
        assert_eq!(
            messages(&mut session, "host := \"x\""),
            vec!["cannot rebind `host`: it is a field, not a variable"]
        );
        // The two definitions of `port` share its slot, and the variables
        // of later inputs take slots of their own after it.
        value(&mut session, "let a = 1");
        value(&mut session, "...{ b = 2; c = 3 }\nb = \"s\"\nlet d = 4");
        value(&mut session, "let e = 5");
        assert_eq!(
            value(&mut session, "\"{host} {port} {a} {b} {c} {d} {e}\""),
            Value::Str("h 8080 1 s 3 4 5".into())
        );
    }

    #[test]
    fn a_union_variable_is_used_by_later_inputs() {
        let mut session = Session::new();
        // The warning does not stop the input.
        let outcome = eval(&mut session, "let mut v = if 1 > 2 { 1 } else { \"one\" }").unwrap();
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(value(&mut session, "v"), Value::Str("one".into()));
        assert_eq!(value_type(&session), "Int | String");
        assert_eq!(
            value(&mut session, "if v is Int { v } else { len([v, v]) }"),
            Value::Int(2)
        );
        value(&mut session, "v := 5");
        assert_eq!(value(&mut session, "(v as Int) + 1"), Value::Int(6));
        // A failed `as` leaves the session as it was, a rebind it made
        // before it failed included.
        let failed = eval(&mut session, "v := \"no\"\nlet n = v as Int").unwrap_err();
        assert_eq!(failed[0].message, "`v` is not an Int");
        assert_eq!(failed[0].severity, Severity::Error);
        assert_eq!(value(&mut session, "v"), Value::Int(5));
        assert_eq!(messages(&mut session, "n"), vec!["undefined variable `n`"]);
    }

    #[test]
    fn a_session_runs_exec_panic_and_sh() {
        let mut session = Session::new();
        // The twenty-four built-in functions' cells are its first slots.
        assert_eq!(
            &session.frame[..24],
            Builtin::ALL.map(Value::Builtin).as_slice()
        );
        // On the first input, and on later ones, through a variable.
        let shown = |value: Value| value.to_string();
        assert_eq!(
            shown(value(&mut session, "exec([\"echo\", \"hi\"])")),
            "{status = 0; stdout = \"hi\\n\"; stderr = \"\"}"
        );
        assert_eq!(
            value_type(&session),
            "{status: Int, stderr: String, stdout: String}"
        );
        value(&mut session, "let r = exec([\"cat\"])");
        value(&mut session, "let e = exec");
        assert_eq!(
            value(&mut session, "e([\"sh\", \"-c\", \"exit 4\"]).status"),
            Value::Int(4)
        );
        assert_eq!(value(&mut session, "r.stdout"), Value::Str("".into()));
        assert_eq!(
            value(&mut session, "sh(\"echo s\")"),
            Value::Str("s\n".into())
        );
        // Every record `exec` builds, in any input, has the session's one
        // layout.
        let Value::Record(record) = value(&mut session, "e([\"true\"])") else {
            panic!("expected a record");
        };
        let layout = Rc::clone(session.exec_result.as_ref().unwrap());
        assert!(Rc::ptr_eq(&record.layout, &layout));
        let Value::Record(earlier) = value(&mut session, "r") else {
            panic!("expected a record");
        };
        assert!(Rc::ptr_eq(&earlier.layout, &layout));
        // A panic fails its input, which the session forgets; a failing `sh`
        // is the prelude's panic, input 2.
        assert_eq!(messages(&mut session, "let x = panic(\"stop\")"), ["stop"]);
        assert_eq!(messages(&mut session, "x"), ["undefined variable `x`"]);
        let failed = eval(&mut session, "sh(\"exit 3\")").unwrap_err();
        assert_eq!(failed[0].message, "`exit 3` exited with status 3: ");
        assert_eq!(failed[0].input, Some(2));
        assert_eq!(
            value(&mut session, "exec([\"true\"]).status"),
            Value::Int(0)
        );
        // A later input shadows them as any earlier name.
        value(&mut session, "let exec = 2");
        assert_eq!(
            value(&mut session, "exec + e([\"true\"]).status"),
            Value::Int(2)
        );
    }

    #[test]
    fn a_session_has_the_string_built_ins_and_path() {
        let mut session = Session::new();
        // Every standard module was loaded as it started, each a scope of
        // its own, in path order.
        assert_eq!(
            session.source(1).map(SourceFile::name),
            Some("<stdlib>/path.gcsl")
        );
        // `split` on the first input.
        assert_eq!(
            value(&mut session, "split(\"a/b\", \"/\")").to_string(),
            "[\"a\", \"b\"]"
        );
        assert_eq!(value_type(&session), "List<String>");
        // `import path`, then `path.join` in that input and a later one,
        // beside the prelude's `join`.
        assert_eq!(
            value(&mut session, "import path\npath.join(\"a\", \"b\")"),
            Value::Str("a/b".into())
        );
        assert_eq!(
            value(&mut session, "path.join(\"/x\", \"../y\")"),
            Value::Str("/y".into())
        );
        assert_eq!(
            value(&mut session, "join([\"a\", \"b\"], \", \")"),
            Value::Str("a, b".into())
        );
        assert_eq!(
            value(&mut session, "trim(\"  t \")"),
            Value::Str("t".into())
        );
        // The more string built-ins and `compare_versions`, on an input and
        // through a later one's variable.
        value(
            &mut session,
            "let tags = lines(\"refs/tags/v6.6\\nrefs/tags/v6.10\\n\")",
        );
        assert_eq!(value_type(&session), "{}");
        assert_eq!(
            value(
                &mut session,
                "let vs = for t in tags { strip_prefix(t, \"refs/tags/v\") }\n\
                 compare_versions(vs[1], vs[0])"
            ),
            Value::Int(1)
        );
        assert_eq!(value_type(&session), "Int");
        assert_eq!(
            value(&mut session, "fields(strip_suffix(\" a b.gz\", \".gz\"))").to_string(),
            "[\"a\", \"b\"]"
        );
        // `sort_versions` and `show` on an input, and `show` through an alias
        // a later input calls, by the alias rule.
        assert_eq!(
            value(
                &mut session,
                "show({v = sort_versions([\"1.10\", \"1.9\"])})"
            ),
            Value::Str("{v = [\"1.9\", \"1.10\"]}".into())
        );
        assert_eq!(value_type(&session), "String");
        value(&mut session, "let render = show");
        assert_eq!(
            value(&mut session, "render(\"a\")"),
            Value::Str("\"a\"".into())
        );
        let failed = eval(&mut session, "split(\"a\", \"\")").unwrap_err();
        assert_eq!(
            failed[0].message,
            "`split` needs a separator that is not empty"
        );
        // A later input's `join` shadows the prelude's, not `path`'s.
        value(&mut session, "let join = 5");
        assert_eq!(value(&mut session, "join + 1"), Value::Int(6));
        assert_eq!(
            value(&mut session, "path.join(\"a\", \".\")"),
            Value::Str("a".into())
        );
    }

    #[test]
    fn an_input_has_the_prelude_and_its_attributes() {
        let mut session = Session::new();
        // An input is no file: it cannot turn the prelude off.
        let failed = eval(&mut session, "#![no_prelude]\nx = 1").unwrap_err();
        let shown: Vec<(&str, u32, u32)> = failed
            .iter()
            .map(|error| (error.message.as_str(), error.span.start, error.span.end))
            .collect();
        assert_eq!(shown, [("`#![no_prelude]` applies only to a file", 0, 14)]);
        assert_eq!(value(&mut session, "len([1])"), Value::Int(1));
        // `#[shadowable]` changes nothing between inputs, which shadow the
        // earlier inputs' names already; within one, it is as in a file.
        value(&mut session, "#[shadowable] a = 1");
        value(&mut session, "a = 2");
        assert_eq!(value(&mut session, "a"), Value::Int(2));
        value(&mut session, "b = 1");
        value(&mut session, "#[shadowable] b = 3");
        assert_eq!(value(&mut session, "b"), Value::Int(3));
        value(&mut session, "x = c\n#[shadowable] c = 1\nc = 4");
        assert_eq!(value(&mut session, "x + c"), Value::Int(8));
        // The attribute errors are a file's.
        assert_eq!(
            messages(&mut session, "#[shadowable] let d = 1"),
            ["`#[shadowable]` applies only to a file's top-level fields, `def`s and imports"]
        );
        // A dotted root of a prelude name shadows it, as before.
        value(&mut session, "len.x = 1");
        assert_eq!(value(&mut session, "len.x"), Value::Int(1));
        assert_eq!(
            messages(&mut session, "len([1])"),
            ["cannot call a value of type `{x: Int}`"]
        );
    }

    #[test]
    fn a_session_is_in_its_working_directory() {
        let mut session = Session::new();
        let root = Value::Str(workspace::working_directory().into());
        assert!(
            matches!(&root, Value::Str(text) if text.starts_with('/')),
            "{root}"
        );
        // Every input is in the working directory the session started in,
        // its workspace and its directory.
        value(&mut session, "import fileroot");
        assert_eq!(value(&mut session, "fileroot.workspace"), root);
        assert_eq!(value(&mut session, "fileroot.this"), root);
        assert_eq!(value(&mut session, "fileroot.caller"), root);
        // A function of an earlier input, called from a later one, through
        // the import the later input inherits.
        value(&mut session, "fn f(): String { fileroot.caller }");
        assert_eq!(value(&mut session, "f()"), root);
        value(&mut session, "fn g(): String { fileroot.helper()\nf() }");
        assert_eq!(value(&mut session, "g()"), root);
        // A memoised function reading the caller, called twice, runs once.
        value(
            &mut session,
            "fn m(x: Int) ->> {c: String} { c = fileroot.caller }",
        );
        value(&mut session, "a = m(1)\nb = m(1)");
        let (Value::Record(a), Value::Record(b)) =
            (value(&mut session, "a"), value(&mut session, "b"))
        else {
            panic!("expected records");
        };
        assert!(Rc::ptr_eq(&a, &b));
        assert_eq!(
            messages(&mut session, "import caller from fileroot"),
            ["`fileroot`'s members are read as `fileroot.caller`"]
        );
        assert_eq!(
            messages(&mut session, "x = fileroot"),
            ["`fileroot`'s members are read as `fileroot.caller`"]
        );
    }

    #[test]
    fn a_session_has_fs_and_owns_its_temporary_directories() {
        let mut session = Session::new();
        // Loaded as the session started, its first input.
        assert_eq!(
            session.source(0).map(SourceFile::name),
            Some("<stdlib>/fs.gcsl")
        );
        // `import fs` then a read, in that input and a later one.
        let dir =
            std::env::temp_dir().join(format!("gcsl-slice-42-{}-session", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.to_str().unwrap().to_owned();
        std::fs::write(format!("{dir}/x.txt"), "from a file\n").unwrap();
        assert_eq!(
            value(
                &mut session,
                &format!("import fs\nfs.read(\"{dir}/x.txt\")")
            ),
            Value::Str("from a file\n".into())
        );
        assert_eq!(value_type(&session), "String");
        assert_eq!(
            value(&mut session, &format!("fs.exists(\"{dir}/x.txt\")")),
            Value::Bool(true)
        );
        // Its errors are the input's, at the call.
        let failed = eval(&mut session, "fs.read(\"x.txt\")").unwrap_err();
        assert_eq!(
            failed[0].message,
            "`fs.read` needs an absolute path, found `x.txt`"
        );
        assert_eq!(failed[0].input, None);
        assert_eq!(failed[0].span, crate::Span::new(0, 16));
        // A temporary directory lasts as long as the session, an input that
        // failed after making one included, and a clone shares them.
        let Value::Str(first) = value(&mut session, "let t = fs.temp_dir()\nt") else {
            panic!("expected a string");
        };
        let failed = eval(&mut session, "let u = fs.temp_dir()\npanic(u)").unwrap_err();
        let second = failed[0].message.clone();
        assert_ne!(&*first, second);
        value(&mut session, "fs.mkdir(\"{t}/inner\")");
        for made in [&*first, &second] {
            assert!(std::path::Path::new(made).is_dir(), "{made}");
        }
        let clone = session.clone();
        drop(session);
        assert!(std::path::Path::new(&format!("{first}/inner")).is_dir());
        drop(clone);
        for made in [&*first, &second] {
            assert!(!std::path::Path::new(made).exists(), "{made}");
        }
        assert!(!std::path::Path::new(&*first).parent().unwrap().exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
