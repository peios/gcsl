//! A workspace as a session: checking and running an input beside the standard
//! library and the inputs before it.
//!
//! # A workspace as a session
//!
//! [`check`](super::check) scans the workspace and then checks and lowers every file in
//! the order, as the inputs of a [`crate::Session`]: one table of names, of
//! types and of slots, each file's diagnostics named by its input number,
//! its index among the files, the standard files it loads first and the
//! workspace's after them in path order ([`Diagnostic::input`]). After a
//! file is checked, its declarations are committed to its directory's scope
//! for the files after it; its private names (`let`, `let mut`, `fn`) are
//! not, though their slots stay taken. The scope is committed after the check,
//! not after a run, since every file is checked before any runs: any error
//! anywhere stops everything. A file that needs a file whose check failed
//! is not checked, since its errors would follow from the first.
//!
//! [`Workspace::run`] then runs the file asked for, after the files it
//! needs, transitively ([`Scan::needed`](super::Scan::needed)), each as a whole and in the
//! order, on one frame and with one memo table, as session inputs share
//! theirs: a closure or a future of one file used in another reads its
//! file's cells, and a future awaited from several files runs once. Files
//! nothing needs are checked but never run. The effects of a file all
//! happen before those of a file that needs it. A runtime error names the
//! file whose code failed.

use std::mem;
use std::rc::Rc;

use crate::session;
use crate::ast::{Ast, ItemKind, PathSegment, TypeStore};
use crate::check::{CheckOutput, Checker, KnownParams, Scope, Slot, check_types};
use crate::diagnostic::{self, Diagnostic};
use crate::eval::{MemoTables, TempDirs};
use crate::ir::Ir;
use crate::lexer::TokenKind;
use crate::names::{self, Name, Names};
use crate::source::{SourceFile, Span};
use crate::types::{TyId, TypeTable};
use crate::value::{Layout, Quantity, Value};

use super::{
    Alias, Context, File, ForLater, InheritedContext, InheritedDefType, InheritedVariant, Library,
    Placing, Workspace, being_run, check_declared_items, check_loading, count, scan_into_with,
};

/// The standard library as a session ran it once, as it started: what each
/// of its inputs is scanned and checked beside ([`check_input`]).
#[derive(Clone, Debug)]
pub(crate) struct Standard {
    /// The number of its cells, the built-in functions' and every standard
    /// file's nodes, which are the first nodes of the scan of every input,
    /// which loads every standard file too, an input's after them. (A
    /// standard file that is a value file has no cell of its value here;
    /// none is.)
    pub(super) cells: u32,
    /// The type of each of its cells.
    pub(super) cell_types: Vec<TyId>,
    /// The slot each of its cells is read from: its own, or for a `def` its
    /// function's.
    pub(super) cell_slots: Vec<Slot>,
    /// For each of its cells that is a `def`'s, what a call of it needs to
    /// know of its parameters.
    pub(super) cell_known: Vec<Option<Rc<KnownParams>>>,
    /// The methods and the contexts' defaults of the standard files,
    /// checked.
    pub(super) for_later: ForLater,
    /// The table of the directories of the run ([`Scan::locate`](super::Scan::locate)): the
    /// session's root, the working directory it started in, first, at index
    /// 0, where every input is, both its `fileroot.this` and its
    /// `fileroot.workspace`, and the standard files' `<stdlib>` after it.
    /// Every input's code names its directory by it, the root's, and its run
    /// reads it, so no input needs it extended.
    pub(crate) dirs: Vec<Rc<str>>,
}

/// What a session continues from once it has run the standard library
/// ([`start`]): the standard library's cells, and the names, types, frame,
/// memo tables, temporary directories and names of slots of the run. (The
/// standard library has no warnings to keep.)
#[derive(Debug)]
pub(crate) struct Started {
    /// The standard library's cells.
    pub(crate) standard: Standard,
    /// The names interned.
    pub(crate) names: Names,
    /// The types interned.
    pub(crate) types: TypeTable,
    /// The top level's frame, the standard library's cells and slots
    /// written.
    pub(crate) frame: Vec<Value>,
    /// The memo tables of the run.
    pub(crate) memo: MemoTables,
    /// The layout of the record `exec` returns, built once for the run.
    pub(crate) exec_result: Rc<Layout>,
    /// The temporary directories of the run, which `fs.temp_dir` makes, for
    /// the session to own from then on.
    pub(crate) temp_dirs: TempDirs,
    /// The name of each slot of the frame.
    pub(crate) slot_names: Vec<Name>,
}

/// Checks and runs the standard library as a session starts: through the
/// driver of a workspace with no file of its own, whose root is `root`, the
/// session's, that loads every standard file, each a scope of its own, the
/// built-in functions' cells written as the frame is laid out and every
/// standard file run.
///
/// # Errors
///
/// The diagnostics of the standard library, which are a bug.
pub(crate) fn start(root: &str) -> Result<Started, Vec<Diagnostic>> {
    let mut workspace = check_loading((&[], Placing::Workspace(&[])), Library::EVERY, root).1?;
    for file in 0..workspace.scan.standard {
        workspace.run(file)?;
    }
    let Workspace {
        scan,
        frame,
        memo,
        exec_result,
        temp_dirs,
        types,
        slot_names,
        cell_types,
        cell_slots,
        cell_known,
        for_later,
        ..
    } = workspace;
    Ok(Started {
        standard: Standard {
            cells: count(scan.cells()),
            cell_types,
            cell_slots,
            cell_known,
            for_later,
            dirs: scan.dir_texts,
        },
        names: scan.names,
        types,
        frame,
        memo,
        exec_result,
        temp_dirs,
        slot_names,
    })
}

/// Checks and lowers `source`, session input number `input`, beside the
/// standard library `standard`, from `scope`, the session's: scanned as a lone
/// file is ([`check_lone`](super::check_lone)), its names interned into `names`
/// and its types into `types`, so that it may import `builtins` and the
/// standard files and has the prelude's names as the yielding last source of
/// its view; its cells, after the standard library's, take the slots after the
/// scope's ([`Scan::slot`](super::Scan::slot)), and its check is a session
/// input's, which reads the scope's variables as a session's input does (see
/// [`crate::session`]). It inherits what `earlier` holds of the session's
/// earlier inputs: their imports, as the imports of its own that follow its
/// `import`s ([`inherit_imports`](super::imports::inherit_imports)), and their
/// types, as the last source of its view of types, its own types defined in
/// their store after theirs (see [Types](crate::workspace::declared#types)).
/// Its diagnostics name it as the input being run, with no input number; a
/// label in a standard file names that file's. Returns, with its IR and its
/// check's output, what a later input inherits, once it has run: the imports it
/// has, its own and those it inherited and did not shadow, each with no item or
/// span of this input's, and its view of types, each type with its
/// declaration's number in the store.
///
/// # Errors
///
/// The diagnostics of the scan, the check or the lowering.
pub(crate) fn check_input(
    standard: &Standard,
    (source, earlier): (&SourceFile, Earlier),
    (names, types): (&mut Names, &mut TypeTable),
    scope: Scope,
    input: u32,
) -> Result<(Ir, CheckOutput, Inherited), Vec<Diagnostic>> {
    let (lowered, inherited) = check_input_then(
        standard,
        (source, earlier, input),
        (names, types),
        scope,
        |ast, checked, types| {
            checked.and_then(|output| session::lower_checked(ast, output, types, input))
        },
    )?;
    let (ir, output) = lowered?;
    Ok((ir, output, inherited))
}

/// What a session's input inherits from the session's earlier inputs (see
/// [`check_input`]).
#[derive(Debug)]
pub(crate) struct Earlier<'a> {
    /// Their imports that no later input shadowed.
    pub(crate) imports: &'a [Alias],
    /// Their types that no later input shadowed, each with its
    /// declaration's number in `store`.
    pub(crate) types: &'a [(Name, u32)],
    /// The definitions of their types, which the input's are added to.
    pub(crate) store: &'a mut TypeStore,
    /// Their def-types that no later input shadowed, each by its path, with
    /// its namespace's key, and each name a later input bound to something
    /// else, with none (see [Def-types](crate::workspace::declared#def-types)).
    pub(crate) def_types: &'a [InheritedDefType],
    /// Their variants that no later input shadowed, each by its enum's key
    /// and its name, with its own key (see [Enums](crate::workspace::declared#enums)).
    pub(crate) variants: &'a [(Name, Name, Name)],
    /// Their contexts that no later input shadowed (see
    /// [Context](crate::workspace::declared#context)).
    pub(crate) contexts: &'a [InheritedContext],
}

/// What a later input inherits of a session's input once it has run (see
/// [`check_input`]).
#[derive(Debug, Default)]
pub(crate) struct Inherited {
    /// Its imports.
    pub(crate) imports: Vec<Alias>,
    /// Its view of types.
    pub(crate) types: Vec<(Name, u32)>,
    /// The def-types it declares, each by its path with its namespace's key.
    pub(crate) def_types: Vec<(Vec<Name>, Name)>,
    /// The variants it declares, each by its enum's key and its name, with
    /// its own key.
    pub(crate) variants: Vec<InheritedVariant>,
    /// The contexts it declares.
    pub(crate) contexts: Vec<InheritedContext>,
}

/// [`check_input`] up to its check, whose outcome `then` is given with the
/// input's tree, its names in it, and the types: what lowering is done by,
/// or a test looks at. Returns what `then` does, with what a later input
/// inherits as [`check_input`] does. The input's `type` items are checked
/// before it, as a workspace's are ([`check_types`]), what is wrong in
/// them among the check's errors.
///
/// # Errors
///
/// The diagnostics of the scan.
pub(super) fn check_input_then<R>(
    standard: &Standard,
    (source, earlier, input): (&SourceFile, Earlier, u32),
    (names, types): (&mut Names, &mut TypeTable),
    scope: Scope,
    then: impl FnOnce(&Ast, Result<CheckOutput, Vec<Diagnostic>>, &mut TypeTable) -> R,
) -> Result<(R, Inherited), Vec<Diagnostic>> {
    // Every standard file, as the session's start loaded them, so that its
    // cells are numbered as they were there.
    let mut loaded = Vec::new();
    let placed = (
        std::slice::from_ref(source),
        Placing::Lone(earlier.imports, true),
    );
    let store = earlier.store;
    let mut scan = scan_into_with(
        placed,
        Library::EVERY,
        (names, &mut loaded),
        (store, (earlier.types, earlier.variants, earlier.contexts)),
        (earlier.def_types, Some(input)),
    )
    .map_err(|errors| being_run(errors, count(loaded.len())))?;
    // The input's enums and variants, the standard files' being known since
    // the session started.
    let mut lent = mem::take(&mut scan.names);
    let standard_files = scan.standard;
    let enum_errors = check_declared_items((&mut scan, standard_files), &mut lent, types);
    scan.names = lent;
    // In the session's root, as its start located the standard files.
    scan.locate(standard.dirs.first().map_or("/", |root| root));
    let index = scan.standard;
    // The input's cells, but its `def`s', take the slots past the scope's,
    // in order.
    let cells = scan.cells();
    let cell_names = scan.cell_names();
    let mut next = count(scope.slot_count());
    let mut taken = Vec::new();
    let relocated = (standard.cells..count(cells))
        .map(|cell| {
            if scan.is_def(cell) {
                return Slot::NONE;
            }
            if let Some(&name) = cell_names.get(cell as usize) {
                taken.push(name);
            }
            next = next.saturating_add(1);
            Slot::at(next - 1)
        })
        .collect();
    scan.relocation = Some((standard.cells, relocated));
    let mut cell_types = standard.cell_types.clone();
    cell_types.resize(cells, TyId::ERROR);
    let mut cell_slots = standard.cell_slots.clone();
    cell_slots.extend((count(cell_slots.len())..count(cells)).map(|cell| scan.slot(cell)));
    let mut cell_known = standard.cell_known.clone();
    cell_known.resize(cells, None);
    let scope = scope.with_slots(taken);
    let file_names = mem::take(&mut scan.names);
    let Some(file) = scan.files.get_mut(index as usize) else {
        *names = file_names;
        *store = mem::take(&mut scan.type_store);
        return Err(Vec::new());
    };
    file.ast.replace_names(file_names);
    let finished = scan.files.get(index as usize).and_then(|file| {
        let reads = (&cell_slots[..], &cell_known[..], &standard.for_later);
        let context = Context::new(&scan, index, &cell_types, reads)?;
        let context = Context {
            session: true,
            ..context
        };
        let mut declared = if file.mentions.types.is_empty() {
            Vec::new()
        } else {
            // A session's input with a syntax error is not checked.
            check_types(&file.ast, types, false)
        };
        declared.extend(enum_errors.iter().cloned());
        let checked = Checker::in_workspace(&file.ast, scope, types, &context).check();
        let checked = match (declared.is_empty(), checked) {
            (true, checked) => checked,
            (false, Ok(output)) => Err(diagnostic::merge(declared, output.warnings)),
            (false, Err(found)) => Err(diagnostic::merge(declared, found)),
        };
        Some(then(&file.ast, checked, types))
    });
    let mut inherited = Inherited::default();
    if let Some(file) = scan.files.get_mut(index as usize) {
        *names = file.ast.replace_names(Names::new());
        inherited
            .imports
            .extend(file.aliases.iter().filter(|a| !a.implicit).map(|a| Alias {
                span: Span::new(0, 0),
                item: None,
                inherited: true,
                ..*a
            }));
        inherited
            .types
            .extend(file.type_view.iter().map(|(&name, &decl)| (name, decl)));
        inherited.types.sort_unstable();
        inherited.def_types = declared_def_types(file, names, input);
    }
    inherited.variants = scan
        .variants
        .iter()
        .filter(|variant| variant.file == index)
        .filter_map(|variant| Some((variant.enum_of?.0, variant.name, variant.key)))
        .collect();
    inherited.contexts = scan.own_contexts(index);
    *store = mem::take(&mut scan.type_store);
    finished.map(|r| (r, inherited)).ok_or_else(Vec::new)
}

/// The def-types `file`, session input number `input` whose names are `names`,
/// declares: each `def` marked `#[def_type]` that stands, by its path, with its
/// namespace's key, as [`name_namespaces`](super::scanning::name_namespaces)
/// makes it (see [Def-types](crate::workspace::declared#def-types)).
pub(super) fn declared_def_types(
    file: &File,
    names: &mut Names,
    input: u32,
) -> Vec<(Vec<Name>, Name)> {
    let mut found = Vec::new();
    for node in &file.mentions.tree.nodes {
        let Some(leaf) = node.leaf else {
            continue;
        };
        let ItemKind::Field {
            path,
            function: true,
            ..
        } = file.ast.item(leaf.item).kind
        else {
            continue;
        };
        if !file.ast.is_def_type(leaf.item) {
            continue;
        }
        let path: Vec<Name> = file
            .ast
            .path(path)
            .iter()
            .filter_map(PathSegment::name)
            .collect();
        let shown: Vec<&str> = path.iter().map(|&name| names.get(name)).collect();
        let item = u32::try_from(leaf.item.index()).unwrap_or(u32::MAX);
        let text = names::namespace_key(&shown.join("."), input, item);
        found.push((path, names.intern(&text)));
    }
    found
}

/// For the tests of the other modules, which check a text alone: `text`
/// checked as a session's first input is, beside the standard library, so
/// that the prelude's names, `len` among them, are its, with the tree it
/// was parsed to, which holds every name, and the types. Its top-level
/// slots come after the standard library's.
#[cfg(test)]
pub(crate) fn check_beside_prelude(
    text: &str,
) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    let started = start("/").unwrap();
    let (mut names, mut types) = (started.names, started.types);
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let scope = Scope::of_slots(started.slot_names);
    let earlier = Earlier {
        imports: &[],
        types: &[],
        store: &mut TypeStore::new(),
        def_types: &[],
        variants: &[],
        contexts: &[],
    };
    // A session's first input, after the standard files.
    let input = count(crate::STANDARD.len());
    let checked = check_input_then(
        &started.standard,
        (&source, earlier, input),
        (&mut names, &mut types),
        scope,
        |ast, checked, _| (ast.clone(), checked),
    );
    match checked {
        Ok(((ast, checked), _)) => (ast, types, checked),
        Err(errors) => panic!("{text}: {errors:?}"),
    }
}

/// The duration or the size `text` is, if it is one: one of its literals,
/// read by the lexer as a program's is, with `-` before it if negative, and
/// nothing else, not even a space.
pub(super) fn quantity_literal(text: &str) -> Option<(Quantity, i64)> {
    let (negative, literal) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let source = SourceFile::new("<argument>", literal).ok()?;
    let (tokens, _, _) = crate::lexer::tokenize(&source).ok()?;
    let (quantity, amount) = match tokens.as_slice() {
        [first, end] if end.kind == TokenKind::Eof && first.span.start == 0 => match first.kind {
            TokenKind::Duration(amount) => (Quantity::Duration, amount),
            TokenKind::Size(amount) => (Quantity::Size, amount),
            _ => return None,
        },
        _ => return None,
    };
    let amount = if negative {
        amount.checked_neg()?
    } else {
        amount
    };
    Some((quantity, amount))
}
