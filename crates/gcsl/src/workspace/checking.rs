//! Checking a workspace: each file in order, what checking one commits for the
//! next, and the indexes of the language server.

use std::mem;
use std::collections::HashMap;
use std::rc::Rc;

use crate::session;
use crate::ast::{ItemId, ItemKind, Syntax};
#[cfg(test)]
use crate::check::ViewRecord;
use crate::check::{
    Builtin, CheckOutput, Checker, KnownParams, Record, Scope, Slot, Types, VariantItem,
    check_enum_bounds, check_types, check_variant_payloads,
};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::eval::{MemoTables, TempDirs};
use crate::index::{Index, Indexed};
use crate::ir::Ir;
use crate::names::{Name, Names};
use crate::source::{SourceFile, Span};
use crate::types::{TyId, TypeTable};
use crate::value::Value;

use super::{
    CellReads, CheckedMethod, Context, File, FileRun, ForLater, Library, MethodRef, Placing, Scan,
    Workspace, builtin_cells, check_context_items, count, exec_layout, scan_into,
};

/// Scans, checks and lowers the workspace of `sources`, each with its
/// path relative to the root, in the order of [`Scan::order`].
///
/// # Errors
///
/// The diagnostics of the scan, or those of every file whose check or
/// lowering fails, with every file's syntax errors and the warnings found,
/// in path then source order; each names its file in [`Diagnostic::input`].
/// A file that needs a file that failed is not checked: its errors would
/// follow from the first. A file with syntax errors always fails, after
/// its check of what was parsed (see [Syntax errors](crate::workspace::parse#syntax-errors)).
pub fn check(sources: &[SourceFile]) -> Result<Workspace, Vec<Diagnostic>> {
    check_with(sources, &[])
}

/// [`check`], told the directories left out of the workspace as
/// workspaces of their own (see [`scan_with`](super::scan_with)). Its root, for the
/// `fileroot` module, is the process's working directory, as a session's
/// is ([`working_directory`]); [`check_with_standard`] is told it.
///
/// # Errors
///
/// As [`check`].
pub fn check_with(sources: &[SourceFile], nested: &[String]) -> Result<Workspace, Vec<Diagnostic>> {
    check_with_standard(sources, nested, &working_directory()).1
}

/// [`check_with`], with the standard files it loaded as sources, each named
/// `<stdlib>/STEM.gcsl`, which are its first files, whether the check
/// succeeds or fails: the sources the diagnostics of the standard files
/// name by input number, the workspace's files' numbers coming after them.
/// Only the prelude and the standard files some file imports are loaded,
/// so the number of the first file of the workspace is their number. `root`
/// is the absolute path of the workspace's root directory, canonical and
/// without a trailing `/` (but `/` itself), which every directory of its
/// files is found from, for the `fileroot` module (see the module
/// documentation, "The standard library").
pub fn check_with_standard(
    sources: &[SourceFile],
    nested: &[String],
    root: &str,
) -> (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>) {
    check_in((sources, Placing::Workspace(nested)), Library::NEEDED, root)
}

/// The process's working directory, canonical (symbolic links resolved),
/// as an absolute path without a trailing `/` (but `/` itself): the root a
/// session's inputs are in ([`crate::Session::new`]), and that of a
/// workspace or a lone file checked by an entry point that is given none
/// ([`check`], [`check_with`], [`check_lone`], [`crate::run`]). `/` if it
/// cannot be found, as when it has been removed; a path that is not UTF-8
/// is taken lossily, each invalid sequence replaced by U+FFFD.
#[must_use]
pub fn working_directory() -> String {
    std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_or_else(|_| "/".to_owned(), |dir| dir.to_string_lossy().into_owned())
}

/// Scans, checks and lowers `source`, a file outside any workspace (`gcsl
/// run` of a lone file), as the workspace of one directory holding just
/// it, beside the standard library (see the module documentation, "The
/// standard library"): it is file [`Scan::standard`], after the standard
/// files it loads.
///
/// # Errors
///
/// As [`check`]; an import of anything but `builtins` and the standard
/// files is the checker's error, "`lib` is not a built-in or standard
/// module, and there is no `workspace.gcsl` above this file".
pub fn check_lone(source: &SourceFile) -> Result<Workspace, Vec<Diagnostic>> {
    check_lone_with_standard(source, &working_directory()).1
}

/// [`check_lone`], with the standard files it loaded as sources, as
/// [`check_with_standard`] gives them: the file's input number is their
/// number. `dir` is the absolute path of the file's directory, canonical
/// and without a trailing `/` (but `/` itself), which is the root of the
/// workspace of one directory it is checked as, for the `fileroot` module;
/// [`check_lone`] takes the working directory.
pub fn check_lone_with_standard(
    source: &SourceFile,
    dir: &str,
) -> (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>) {
    let (loaded, checked) = check_lone_loading(source, dir);
    (Library::NEEDED.sources(&loaded), checked)
}

/// [`check_lone_with_standard`], with the indices in [`crate::STANDARD`] of
/// the standard files it loaded, which are its first files: what
/// [`crate::run`] numbers the file by, with no source made of them.
pub(crate) fn check_lone_loading(
    source: &SourceFile,
    dir: &str,
) -> (Vec<usize>, Result<Workspace, Vec<Diagnostic>>) {
    let sources = std::slice::from_ref(source);
    check_loading((sources, Placing::Lone(&[], false)), Library::NEEDED, dir)
}

/// Scans `sources` placed as `placing` says, beside `library`, and checks
/// and lowers every file, as [`check_with`] does, the root being `root`;
/// with the standard files the scan loaded, as [`check_with_standard`]
/// gives them.
pub(super) fn check_in(
    placed: (&[SourceFile], Placing),
    library: Library,
    root: &str,
) -> (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>) {
    let (loaded, checked) = check_loading(placed, library, root);
    (library.sources(&loaded), checked)
}

/// [`check_in`], with the indices in the library's table of the standard
/// files the scan loaded in place of their sources.
pub(super) fn check_loading(
    placed: (&[SourceFile], Placing),
    library: Library,
    root: &str,
) -> (Vec<usize>, Result<Workspace, Vec<Diagnostic>>) {
    let mut loaded = Vec::new();
    let checked =
        scan_into(placed, library, &mut Names::new(), &mut loaded).and_then(|mut scan| {
            scan.locate(root);
            check_scan(scan)
        });
    (loaded, checked)
}

/// The tables of the cells of `scan` before any file is checked: each
/// one's type, `Error` but for the built-in functions', the first
/// ([`builtin_nodes`](super::builtin_nodes)), each built-in's (see [`Builtin::ty`]), interned
/// into `types` with `names`; each one's slot; and nothing known of any
/// `def`'s parameters.
pub(super) fn cell_tables(scan: &Scan, types: &mut TypeTable, names: &mut Names) -> CellTables {
    let mut cell_types = vec![TyId::ERROR; scan.cells()];
    for (cell, builtin) in cell_types.iter_mut().zip(Builtin::ALL) {
        *cell = builtin.ty(types, names);
    }
    let cell_slots = (0..count(scan.cells()))
        .map(|cell| scan.slot(cell))
        .collect();
    (cell_types, cell_slots, vec![None; scan.cells()])
}

/// Takes the names of the slots of `ir`, a file's: its slots' names are the
/// workspace's, `slot_names` (see [`Workspace::dump_ir`]), but for those of
/// its temporaries past the slots it committed, which are kept, with how
/// many it committed, and added to `slot_names`, so that the files checked
/// after it take slots past them. Files do not run in the order they are
/// checked in (a second invocation runs the files it needs after files
/// checked later have run), so a later file's slots, which hold its cells
/// for as long as the workspace lives, must not be a temporary of one that
/// may run after it.
pub(super) fn own_slot_names(ir: &mut Ir, slot_names: &mut Vec<Name>) -> (usize, Vec<Name>) {
    let committed = slot_names.len();
    let own = ir.replace_slot_names(Vec::new());
    let tail = own.get(committed..).unwrap_or_default().to_vec();
    slot_names.extend_from_slice(&tail);
    (committed, tail)
}

/// The tables of a workspace's cells, by number: each one's type, the slot
/// it is read from, and for a `def`'s what a call of it needs to know of its
/// parameters.
pub(super) type CellTables = (Vec<TyId>, Vec<Slot>, Vec<Option<Rc<KnownParams>>>);

/// Checks and lowers every file of `scan` in the order (see [`check`]), each
/// where the scan located it ([`Scan::locate`]).
pub(super) fn check_scan(scan: Scan) -> Result<Workspace, Vec<Diagnostic>> {
    check_scan_partial(scan).map_err(|failed| failed.diagnostics)
}

/// A check of a workspace that failed ([`check_partial`]): its diagnostics,
/// as [`check_with_standard`] gives them, and what it found of the
/// workspace, if its scan succeeded.
#[derive(Debug)]
pub struct Failed {
    /// Every diagnostic, in path then source order.
    pub diagnostics: Vec<Diagnostic>,
    /// What the check found, once the scan succeeded; `None` if it failed.
    pub partial: Option<Box<Partial>>,
}

impl Failed {
    /// The check of `scan`, which interned its types into `types` and found
    /// `checked` of its files, failed with `errors`, and each file's
    /// `warnings`, by input number.
    pub(super) fn of(
        scan: Scan,
        (types, checked): (TypeTable, Vec<Option<Types>>),
        (errors, warnings): (Vec<Diagnostic>, Vec<Vec<Diagnostic>>),
    ) -> Self {
        let warnings = warnings.into_iter().flatten().collect();
        Self {
            diagnostics: merge_files(reported(errors), warnings),
            partial: Some(Box::new(Partial {
                scan,
                types,
                checked,
            })),
        }
    }
}

/// What a check of a workspace that failed leaves of it once its scan
/// succeeded ([`check_partial`]), for a tool that shows what it can of it,
/// as a language server would: the scan, every file's tree as its parser
/// recovered it from its syntax errors ([`crate::ast::Ast::syntax`]), the
/// table its types are interned into, and the types of each file whose
/// check succeeded, by input number. A file whose check failed, one with a
/// syntax error among them, has none, nor does one not checked.
#[derive(Debug)]
pub struct Partial {
    /// The scanned workspace, its names in [`Scan::names`].
    pub scan: Scan,
    /// The table the files' types are interned into.
    pub types: TypeTable,
    /// The type of every expression of each file whose check succeeded, by
    /// input number.
    pub checked: Vec<Option<Types>>,
}

/// [`check_with_standard`], with what the check found of the workspace if
/// it failed after its scan, as [`Partial`] gives it.
pub fn check_partial(
    sources: &[SourceFile],
    nested: &[String],
    root: &str,
) -> (Vec<SourceFile>, Result<Workspace, Failed>) {
    let placed = (sources, Placing::Workspace(nested));
    let mut loaded = Vec::new();
    let checked = match scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded) {
        Ok(mut scan) => {
            scan.locate(root);
            check_scan_partial(scan)
        }
        Err(diagnostics) => Err(Failed {
            diagnostics,
            partial: None,
        }),
    };
    (Library::NEEDED.sources(&loaded), checked)
}

/// [`check_scan`], with what the check found if it fails ([`Partial`]).
pub(super) fn check_scan_partial(scan: Scan) -> Result<Workspace, Failed> {
    check_scan_recording(scan, false).0
}

/// What a check of a workspace that recorded found for its index, besides
/// the workspace or what it found of it (see [`crate::index`]).
#[derive(Debug, Default)]
pub(crate) struct Recorded {
    /// What each file's check recorded, by input number; `None` for a file
    /// not checked, whose tree is unbalanced. (A file that needs one whose
    /// check failed is checked for this alone: `record_blocked`.)
    pub(crate) files: Vec<Option<Box<Record>>>,
    /// The type of each cell, by its number, as the check of the file that
    /// computes it found it, whether that check succeeded or failed: for
    /// what is shown, never for what is checked.
    pub(crate) cells: Vec<TyId>,
}

impl Recorded {
    /// What a check of a workspace of `files` files records, if it
    /// `records`, before any file is checked, the cells' types being
    /// `cells`: nothing of any file yet.
    pub(super) fn new(records: bool, files: usize, cells: &[TyId]) -> Self {
        if !records {
            return Self::default();
        }
        Self {
            files: (0..files).map(|_| None).collect(),
            cells: cells.to_vec(),
        }
    }

    /// Keeps `found`, what the check of file `index` of `scan` recorded, if
    /// it recorded, and the types of the cells the file computes, as its
    /// check found them, failed or not.
    pub(super) fn keep(&mut self, scan: &Scan, index: u32, found: Option<Box<Record>>) {
        let Some(found) = found else {
            return;
        };
        let global = scan
            .files
            .get(index as usize)
            .map_or(&[][..], |f| &f.global);
        for (&global, &ty) in global.iter().zip(&found.cells) {
            if ty != TyId::ERROR
                && scan.computed_by(global) == Some(index)
                && !scan.is_poisoned(global)
            {
                set(&mut self.cells, global, ty);
            }
        }
        if let (Some(cell), Some(ty)) = (scan.value_cell(index), found.value) {
            set(&mut self.cells, cell, ty);
        }
        set(&mut self.files, index, Some(found));
    }
}

/// The check of file `index` of `scan` in `context`, from `scope`,
/// interning into `types`, and its lowering if it succeeds: what it records
/// of its names kept in `recorded`, if the check of the workspace records
/// (see [`crate::check`], "Recording").
pub(super) fn check_file<'a>(
    (scan, index): (&'a Scan, u32),
    context: &'a Context<'a>,
    (scope, types): (Scope, &'a mut TypeTable),
    recorded: &mut Recorded,
) -> Result<(Ir, CheckOutput), Vec<Diagnostic>> {
    let ast = &context.file().ast;
    let checking = Checker::in_workspace(ast, scope, types, context);
    let records = !recorded.files.is_empty();
    let checking = if records {
        checking.recording()
    } else {
        checking
    };
    let (output, found) = checking.check_recorded();
    recorded.keep(scan, index, found);
    output.and_then(|output| session::lower_checked(ast, output, types, index))
}

/// If the check of `scan` records (see [`crate::check`], "Recording"),
/// once every file that could be is checked, checks each file that needs
/// a file whose check failed, and so was not, for its index alone (see
/// [`crate::index`]), in the order: from the declarations `scopes` hold,
/// with the cells' slots and parameters as the checks left them, and their
/// types as `recorded` has them (those the failed checks found among
/// them), lent the workspace's `names`, interning into `types`. It keeps
/// what each records in `recorded`, throws away what each finds, and
/// commits nothing, a name that names nothing taken as no error, since it
/// may be declared in the file that failed. A file whose tree is
/// unbalanced is still not checked.
pub(super) fn record_blocked(
    scan: &mut Scan,
    (scopes, reads): (&[Committed], CellReads<'_>),
    (names, types, recorded): (&mut Names, &mut TypeTable, &mut Recorded),
) {
    if recorded.files.is_empty() {
        return;
    }
    let cell_types = recorded.cells.clone();
    for index in scan.order.clone() {
        let checked = recorded
            .files
            .get(index as usize)
            .is_some_and(Option::is_some);
        if checked || scan.is_unbalanced(index) {
            continue;
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            file.ast.replace_names(mem::take(names));
        }
        if let Some(file) = scan.files.get(index as usize)
            && let Some(context) = Context::new(scan, index, &cell_types, reads)
        {
            let scope = Committed::scope(scan, scopes, file, scan.cell_names());
            let checking = Checker::in_workspace(&file.ast, scope, types, &context);
            let (_, found) = checking.recording().blind().check_recorded();
            recorded.keep(scan, index, found);
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            *names = file.ast.replace_names(Names::new());
        }
    }
}

/// [`check_scan_partial`], each file's check recording what its names refer
/// to if `record` (see [`crate::check`], "Recording"), with what it
/// recorded.
pub(super) fn check_scan_recording(
    mut scan: Scan,
    record: bool,
) -> (Result<Workspace, Failed>, Recorded) {
    let mut names = mem::take(&mut scan.names);
    let mut types = TypeTable::new();
    let (mut cell_types, mut cell_slots, mut cell_known) =
        cell_tables(&scan, &mut types, &mut names);
    let mut slot_names: Vec<Name> = scan.cell_names();
    let mut scopes = vec![Committed::default(); scan.dirs.len()];
    let mut runs: Vec<FileRun> = (0..scan.files.len()).map(|_| FileRun::Succeeded).collect();
    let mut tails: Vec<(usize, Vec<Name>)> = vec![(0, Vec::new()); scan.files.len()];
    let mut checked: Vec<Option<Types>> = (0..scan.files.len()).map(|_| None).collect();
    let mut warnings = vec![Vec::new(); scan.files.len()];
    let mut errors = check_type_items(&mut scan, &mut names, &mut types);
    let mut for_later = ForLater::default();
    // The files whose check failed, or that need one that did: those are
    // not checked, since their errors would follow from the first; and
    // those with syntax errors (see [Syntax errors](crate::workspace::parse#syntax-errors)).
    let mut failed = scan.broken_files();
    let mut recorded = Recorded::new(record, scan.files.len(), &cell_types);
    #[cfg(test)]
    let mut view_records: Vec<Vec<ViewRecord>> = vec![Vec::new(); scan.files.len()];
    for index in scan.order.clone() {
        if blocked(&scan, &mut failed, index) || scan.unchecked(index) {
            continue;
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            file.ast.replace_names(mem::take(&mut names));
        }
        // Every file of the order is a file of the workspace.
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        // The slots' names are moved, not copied: the check gives them back
        // with the file's own slots added.
        let scope = Committed::scope(&scan, &scopes, file, mem::take(&mut slot_names));
        let start = scope.len();
        let reads = (&cell_slots[..], &cell_known[..], &for_later);
        let compiled = Context::new(&scan, index, &cell_types, reads).map(|context| {
            check_file((&scan, index), &context, (scope, &mut types), &mut recorded)
        });
        // What the file declares, committed for the files after it.
        let cells = match &compiled {
            Some(Ok((_, output))) => computed_cells(&scan, index, output),
            _ => Vec::new(),
        };
        let keep = |name: Name| commits(&scan, index, name);
        let poison = |name: Name| poisoned_root(&scan, index, name);
        #[cfg(test)]
        keep_view_records(&mut view_records, index, compiled.as_ref());
        let result = match compiled {
            Some(Ok((ir, output))) => {
                for_later.extend(&scan, index, &output);
                if let Some(dir) = scopes.get_mut(file.dir as usize) {
                    dir.commit(&output.scope, start, (keep, poison));
                }
                slot_names = output.scope.into_slot_names();
                Ok((ir, output.checked.types, output.warnings))
            }
            failed_check => {
                // The slots' names went to the failed check. Nothing runs
                // once a file has failed, so the files checked after it
                // need only the merged nodes' slots kept apart from theirs.
                slot_names = scan.cell_names();
                // Every file of the order is a file of the workspace.
                Err(failed_check.and_then(Result::err).unwrap_or_default())
            }
        };
        if let Some(file) = scan.files.get_mut(index as usize) {
            names = file.ast.replace_names(Names::new());
        }
        set_cells(&mut cell_types, &mut cell_slots, &mut cell_known, cells);
        match result {
            Ok((mut ir, file_types, found)) => {
                set(&mut tails, index, own_slot_names(&mut ir, &mut slot_names));
                set(&mut runs, index, FileRun::Pending(Box::new(ir)));
                set(&mut checked, index, Some(file_types));
                set(&mut warnings, index, in_file(found, index));
            }
            Err(found) => {
                errors.extend(scan.failure(index, found));
                set(&mut failed, index, true);
            }
        }
    }
    let tables = (&scopes[..], (&cell_slots[..], &cell_known[..], &for_later));
    let lent = (&mut names, &mut types, &mut recorded);
    record_blocked(&mut scan, tables, lent);
    scan.names = names;
    if !errors.is_empty() || failed.contains(&true) {
        let failed = Failed::of(scan, (types, checked), (errors, warnings));
        return (Err(failed), recorded);
    }
    let cells = (cell_types, cell_slots, cell_known);
    let workspace = assembled(
        scan,
        (runs, checked, warnings.into_iter().flatten().collect()),
        (slot_names, tails),
        (types, cells, for_later),
    );
    #[cfg(test)]
    let workspace = Workspace {
        view_records,
        ..workspace
    };
    (Ok(workspace), recorded)
}

/// Each file's run and types, and the warnings, of a check (see
/// [`assembled`]).
type FileResults = (Vec<FileRun>, Vec<Option<Types>>, Vec<Diagnostic>);

/// The workspace a check that found nothing wrong made, of the scan
/// `scan`, each file's run, types and the warnings, the slots' names and
/// each file's own, the type table, the cells' tables, and what later
/// files read of the earlier ones.
fn assembled(
    mut scan: Scan,
    (runs, checked, warnings): FileResults,
    (slot_names, slot_tails): (Vec<Name>, Vec<(usize, Vec<Name>)>),
    (types, (cell_types, cell_slots, cell_known), for_later): (TypeTable, CellTables, ForLater),
) -> Workspace {
    Workspace {
        values: vec![Value::Empty; scan.files.len()],
        // Built before `scan` moves; its names are interned already.
        exec_result: exec_layout(&mut scan.names),
        temp_dirs: TempDirs::new(),
        calls_prepared: 0,
        suspended: std::collections::HashMap::new(),
        profile: None,
        scan,
        runs,
        checked,
        warnings,
        // The driver writes the built-in functions' cells, the first slots,
        // as it lays the frame out, before any file runs.
        frame: builtin_cells(),
        slot_names,
        slot_tails,
        memo: MemoTables::new(),
        types,
        cell_types,
        cell_slots,
        cell_known,
        for_later,
        #[cfg(test)]
        view_records: Vec::new(),
    }
}

/// Scans `sources` placed as `placing` says, beside the standard library,
/// and checks every file as [`check_with_standard`] does, the root being
/// `root`, each file's check recording what its names refer to (see
/// [`crate::check`], "Recording"), and indexes what it found, whether it
/// succeeded or failed: what [`index_with_standard`] and
/// [`index_lone_with_standard`] do.
pub(super) fn index_in(placed: (&[SourceFile], Placing), root: &str) -> Indexed {
    let given = placed.0;
    let mut loaded = Vec::new();
    let scanned = scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded);
    let standard = Library::NEEDED.sources(&loaded);
    let mut scan = match scanned {
        Ok(scan) => scan,
        Err(diagnostics) => {
            return Indexed {
                standard,
                diagnostics,
                succeeded: false,
                index: None,
            };
        }
    };
    scan.locate(root);
    let (checked, recorded) = check_scan_recording(scan, true);
    let (diagnostics, succeeded, parts) = match checked {
        Ok(workspace) => {
            let Workspace {
                scan,
                types,
                warnings,
                ..
            } = workspace;
            (warnings, true, Some((scan, types)))
        }
        Err(Failed {
            diagnostics,
            partial,
        }) => (diagnostics, false, partial.map(|p| (p.scan, p.types))),
    };
    let index = parts.map(|(scan, types)| {
        let mut sources = standard.clone();
        sources.extend(given.iter().cloned());
        Index::new(scan, types, recorded, (sources, &diagnostics))
    });
    Indexed {
        standard,
        diagnostics,
        succeeded,
        index,
    }
}

/// Scans, checks and indexes the workspace of `sources`, as
/// [`check_with_standard`] checks it, with the same diagnostics, running
/// nothing, for a language server (see [`crate::index`]): each file's check
/// records what its names
/// refer to (see [`crate::check`], "Recording"), and the index answers what
/// a name at a place is and where it is declared, whether the check
/// succeeded or failed, in every file it checked. `root` is as for
/// [`check_with_standard`].
#[must_use]
pub fn index_with_standard(sources: &[SourceFile], nested: &[String], root: &str) -> Indexed {
    index_in((sources, Placing::Workspace(nested)), root)
}

/// [`index_with_standard`] of `source`, a file outside any workspace,
/// checked as [`check_lone_with_standard`] checks it, in the directory
/// `dir`.
#[must_use]
pub fn index_lone_with_standard(source: &SourceFile, dir: &str) -> Indexed {
    let sources = std::slice::from_ref(source);
    index_in((sources, Placing::Lone(&[], false)), dir)
}

/// Checks the `type` items of every file of `scan` that has any, before any
/// file is checked, since a file may use a type of a file checked after it
/// (see [Types](crate::workspace::declared#types)): what is wrong in each declaration is reported
/// here, once, and every file that uses it fails quietly. Lends each file
/// `names`, and interns into `types`. Returns the errors, each naming its
/// file, after the files' syntax errors and the errors of the scan: the
/// errors found before any file is checked. A file whose tree is
/// unbalanced is not checked; in one whose
/// view sees a file with syntax errors, an unknown type is no error (see
/// [Syntax errors](crate::workspace::parse#syntax-errors)).
pub(super) fn check_type_items(
    scan: &mut Scan,
    names: &mut Names,
    types: &mut TypeTable,
) -> Vec<Diagnostic> {
    let mut errors = scan.syntax_errors.clone();
    errors.extend(scan.recovered.iter().cloned());
    let broken: Vec<bool> = (0..count(scan.files.len()))
        .map(|index| scan.sees_broken(index))
        .collect();
    for ((index, file), sees_broken) in (0u32..).zip(&mut scan.files).zip(broken) {
        if file.mentions.types.is_empty() || file.ast.syntax() == Syntax::Unbalanced {
            continue;
        }
        file.ast.replace_names(mem::take(names));
        let found = check_types(&file.ast, types, sees_broken);
        *names = file.ast.replace_names(Names::new());
        errors.extend(in_file(found, index));
    }
    errors.extend(check_declared_items((scan, 0), names, types));
    errors
}

/// The enums and variants ([`check_enum_items`]) and the contexts
/// ([`check_context_items`]) of the scan's files from number `from` on,
/// checked once their `type` items are. Returns the errors found, each in
/// its file.
pub(super) fn check_declared_items(
    (scan, from): (&mut Scan, u32),
    names: &mut Names,
    types: &mut TypeTable,
) -> Vec<Diagnostic> {
    let mut errors = check_enum_items((scan, from), names, types);
    errors.extend(check_context_items((scan, from), names, types));
    errors
}

/// Resolves the bounds of every enum of the scan's files from number `from` on,
/// and then every variant's payload, once their `type` items are checked (see
/// [Enums](crate::workspace::declared#enums)), lent the workspace's `names`,
/// interning into `types`, which records them. Returns the errors found, each
/// in its file.
pub(super) fn check_enum_items(
    (scan, from): (&mut Scan, u32),
    names: &mut Names,
    types: &mut TypeTable,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let enums: Vec<Vec<(ItemId, Name)>> = (0..count(scan.files.len()))
        .map(|index| scan.file_enums(index))
        .collect();
    let mut variants: Vec<Vec<VariantItem>> = vec![Vec::new(); scan.files.len()];
    for variant in &scan.variants {
        if let Some(entry) = variants.get_mut(variant.file as usize) {
            entry.push((variant.item, variant.enum_of, variant.key));
        }
    }
    for pass in [false, true] {
        for (index, file) in (0u32..).zip(&mut scan.files).skip(from as usize) {
            let (Some(enums), Some(variants)) =
                (enums.get(index as usize), variants.get(index as usize))
            else {
                continue;
            };
            let empty = if pass {
                variants.is_empty()
            } else {
                enums.is_empty()
            };
            if empty || file.ast.syntax() == Syntax::Unbalanced {
                continue;
            }
            file.ast.replace_names(mem::take(names));
            let found = if pass {
                check_variant_payloads(&file.ast, types, variants)
            } else {
                check_enum_bounds(&file.ast, types, enums)
            };
            *names = file.ast.replace_names(Names::new());
            errors.extend(in_file(found, index));
        }
    }
    errors
}

/// `errors`, the errors of a workspace some file of which failed its check,
/// or, if there are none, the internal error that one failed with nothing
/// reported: a check fails so quietly only for an erroneous type, which
/// [`check_type_items`] reports where it is declared.
pub(super) fn reported(errors: Vec<Diagnostic>) -> Vec<Diagnostic> {
    if !errors.is_empty() {
        return errors;
    }
    vec![
        InternalError {
            message: "a file's check failed with nothing reported",
            span: Span::new(0, 0),
        }
        .into(),
    ]
}

/// Whether file `index` commits its declaration `name` for the files
/// checked after it: a root it computes, being its last contributor, or a
/// `def` of that name. (A `def` named by a dotted path is a leaf under its
/// root, committed with the root.)
pub(super) fn commits(scan: &Scan, index: u32, name: Name) -> bool {
    let Some(file) = scan.files.get(index as usize) else {
        return false;
    };
    file.mentions.tree.root(name).is_some_and(|root| {
        let global = file.global.get(root as usize).copied();
        global.and_then(|global| scan.computed_by(global)) == Some(index)
    }) || file.mentions.functions.iter().any(|top| {
        top.name == Some(name) && matches!(file.ast.item(top.item).kind, ItemKind::Field { .. })
    })
}

/// Whether file `index`'s root `name` is poisoned, a conflicted node at or
/// under it (see [Scan errors](crate::workspace::scanning#scan-errors)): it is
/// committed as `Error`.
pub(super) fn poisoned_root(scan: &Scan, index: u32, name: Name) -> bool {
    if scan.poisoned_nodes.is_empty() {
        return false;
    }
    let Some(file) = scan.files.get(index as usize) else {
        return false;
    };
    file.mentions
        .tree
        .root(name)
        .and_then(|root| file.global.get(root as usize))
        .is_some_and(|&global| scan.is_poisoned(global))
}

/// The declarations the files of one directory have committed, for the
/// files checked after them: a scope, with its variables indexed by name,
/// so that a file is checked from the declarations of the names it spells
/// alone, never a copy of every declaration of its directory chain (which
/// would make checking a workspace quadratic in its files).
#[derive(Clone, Debug, Default)]
pub(super) struct Committed {
    /// The variables, in the order they were committed.
    pub(super) scope: Scope,
    /// The indices in `scope` of the variables of each name, in order.
    pub(super) by_name: HashMap<Name, Vec<usize>>,
}

impl Committed {
    /// The scope `file` is checked from, given the committed declarations
    /// of each directory of `scan` and the names of the frame's slots so
    /// far: those of its directory chain, from the root down, of the names
    /// it spells.
    pub(super) fn scope(
        scan: &Scan,
        committed: &[Self],
        file: &File,
        slot_names: Vec<Name>,
    ) -> Scope {
        let chain: Vec<u32> = scan.chain(file.dir).collect();
        Scope::joined(
            chain.iter().rev().filter_map(|&dir| {
                let committed = committed.get(dir as usize)?;
                Some((&committed.scope, committed.picked(&file.spelled)))
            }),
            slot_names,
        )
    }

    /// The indices in `scope` of the variables named in `spelled`, in
    /// commit order, so that of two variables of one name the later is
    /// still the later.
    pub(super) fn picked(&self, spelled: &[Name]) -> Vec<usize> {
        let mut picked: Vec<usize> = spelled
            .iter()
            .filter_map(|name| self.by_name.get(name))
            .flatten()
            .copied()
            .collect();
        picked.sort_unstable();
        picked
    }

    /// Commits the variables of `from` after its first `start` whose names
    /// `keep` accepts, as [`Scope::commit`] does, and indexes them; each
    /// whose name `poisoned` accepts as `Error` (see [Scan
    /// errors](crate::workspace::scanning#scan-errors)).
    pub(super) fn commit(
        &mut self,
        from: &Scope,
        start: usize,
        (keep, poisoned): (impl Fn(Name) -> bool, impl Fn(Name) -> bool),
    ) {
        let before = self.scope.len();
        self.scope.commit(from, start, keep);
        self.scope.poison_from(before, poisoned);
        for (at, name) in self.scope.names().enumerate().skip(before) {
            self.by_name.entry(name).or_default().push(at);
        }
    }
}

/// The cells file `index` computes, now that its check gave `output`: each
/// one's number, the slot it is read from (a `def`'s is its function's) and
/// its type, with the cell of its value if it is an imported value file.
pub(super) fn computed_cells(scan: &Scan, index: u32, output: &CheckOutput) -> Vec<ComputedCell> {
    let file = scan.files.get(index as usize);
    let global = file.map_or(&[][..], |file| &file.global);
    let slots = output.checked.resolution.field_cells();
    let known_by_slot = output.scope.known_by_slot();
    let nodes = file.map_or(&[][..], |file| &file.mentions.tree.nodes);
    let mut cells = Vec::new();
    for (own, (&global, &ty)) in global.iter().zip(&output.cell_types).enumerate() {
        if scan.computed_by(global) == Some(index) {
            // A `def`'s cell is read from its function's slot.
            let slot = slots.get(own).copied().unwrap_or_else(|| scan.slot(global));
            let mut known = known_by_slot.get(&slot).copied().flatten().cloned();
            // A `def` named by a dotted path has no binding: what a call of
            // it needs to know is its function's, which the cell's type is.
            if known.is_none()
                && ty != TyId::ERROR
                && let Some(function) = nodes
                    .get(own)
                    .and_then(|node| node.leaf)
                    .and_then(|leaf| leaf.function)
                    .and_then(|function| file?.mentions.functions.get(function as usize))
                    .filter(|function| function.name.is_none())
            {
                let resolution = &output.checked.resolution;
                let attributes = resolution.attributes(function.function);
                known = Some(KnownParams::of(
                    resolution.function(function.function),
                    attributes,
                ));
            }
            // A poisoned cell is read as `Error` (see [Scan
            // errors](crate::workspace::scanning#scan-errors)).
            if scan.is_poisoned(global) {
                cells.push((global, slot, TyId::ERROR, None));
            } else {
                cells.push((global, slot, ty, known));
            }
        }
    }
    // An imported value file's value is a cell of the workspace.
    if let Some(cell) = scan.value_cell(index) {
        cells.push((cell, scan.slot(cell), output.ty, None));
    }
    cells
}

/// Keeps in `view_records` the records file `index`'s check, `compiled`,
/// made of the nodes of its view with a leaf of it under them, if it
/// succeeded: for the tests' check that each reads only cells stored before
/// it.
#[cfg(test)]
pub(super) fn keep_view_records(
    view_records: &mut [Vec<ViewRecord>],
    index: u32,
    compiled: Option<&Result<(Ir, CheckOutput), Vec<Diagnostic>>>,
) {
    if let Some(Ok((_, output))) = compiled {
        let records = output.checked.resolution.view_records();
        set(view_records, index, records.to_vec());
    }
}

/// The methods of file `index` of `scan` as its check `output` found them
/// (see [Methods](crate::workspace::declared#methods)): each by its binding in the scope it leaves.
pub(super) fn checked_methods(
    scan: &Scan,
    index: u32,
    output: &CheckOutput,
) -> Vec<(MethodRef, CheckedMethod)> {
    let Some(file) = scan.files.get(index as usize) else {
        return Vec::new();
    };
    (0u32..)
        .zip(file.ast.methods())
        .filter_map(|(method, decl)| {
            let (slot, ty, known) = output.scope.function_binding(decl.binding)?;
            let at = MethodRef {
                file: index,
                method,
            };
            Some((at, CheckedMethod { slot, ty, known }))
        })
        .collect()
}

/// The defaults of the contexts of file `index` of `scan` as its check
/// `output` found them (see [Context](crate::workspace::declared#context)): each by its binding in
/// the scope it leaves, by its declaration's key.
pub(super) fn checked_contexts(scan: &Scan, index: u32, output: &CheckOutput) -> Vec<(Name, Slot)> {
    scan.contexts
        .iter()
        .filter(|context| context.file == index)
        .filter_map(|context| {
            let (slot, ..) = output.scope.function_binding(context.binding?)?;
            Some((context.key, slot))
        })
        .collect()
}

/// A cell a file computes, as [`computed_cells`] gives it: its number, the
/// slot it is read from, its type, and for a `def` what a call of it needs to
/// know of its parameters.
pub(super) type ComputedCell = (u32, Slot, TyId, Option<Rc<KnownParams>>);

/// Sets the type, the slot and the parameters of each of `cells` in the
/// cells' tables, `types`, `slots` and `known`.
pub(super) fn set_cells(
    types: &mut [TyId],
    slots: &mut [Slot],
    known: &mut [Option<Rc<KnownParams>>],
    cells: Vec<ComputedCell>,
) {
    for (global, slot, ty, params) in cells {
        set(types, global, ty);
        set(slots, global, slot);
        set(known, global, params);
    }
}

/// Sets the entry `index` of `entries` to `value`, if it has one.
pub(super) fn set<T>(entries: &mut [T], index: u32, value: T) {
    if let Some(entry) = entries.get_mut(index as usize) {
        *entry = value;
    }
}

/// Whether file `index` needs a file in `failed` (whose check failed, or
/// that was not checked for this reason), marking it failed if it does.
pub(super) fn blocked(scan: &Scan, failed: &mut [bool], index: u32) -> bool {
    let blocked = scan.needs.get(index as usize).is_some_and(|needs| {
        needs
            .iter()
            .any(|&need| failed.get(need as usize).copied().unwrap_or(false))
    });
    if blocked && let Some(entry) = failed.get_mut(index as usize) {
        *entry = true;
    }
    blocked
}

/// Merges `errors` and `warnings` of several files into one vector, as
/// [`crate::diagnostic::merge`] does for one: in path order of their files (their
/// [`Diagnostic::input`]), then in source order, errors first among those
/// that start at one place.
pub(super) fn merge_files(errors: Vec<Diagnostic>, warnings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut all = errors;
    all.extend(warnings);
    all.sort_by_key(|diagnostic| (diagnostic.input, diagnostic.span.start));
    all
}

/// `diagnostics`, each named as of file `index` unless it names one.
pub(super) fn in_file(mut diagnostics: Vec<Diagnostic>, index: u32) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        diagnostic.input.get_or_insert(index);
    }
    diagnostics
}
