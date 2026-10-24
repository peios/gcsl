use super::*;
use super::checking::{check_in, check_scan};
use crate::ast::{self, Syntax, TypeStore};
use crate::check::Scope;
use crate::lexer;
use crate::mentions::Need;
use crate::names;
use crate::parser;
use crate::source::Span;
use crate::value::Quantity;
use crate::ir::IrKind;

use merging::{checked, shown_error};
use standard::lone;
use shell::lone_error;
use strings::{checked_beside, prelude_entry, runs_to, value_beside};
use fs::{fs_scratch, lone_error_in, lone_in};
use fileroot::{SCRATCH, rooted, rooted_messages, rooted_text, rooted_value};
use types::check_errors;
use deftypes::{BUILD, JACKS_PACKAGE};
use invoke::{
    PACKAGE, UNDER, invoke_errors, invoke_with, invoked_text, invoked_text_with, listed,
    listing_errors, request_error, request_error_with, string,
};
use metas::{INFO, TAGGED};

/// Scans the workspace of `files`, each a path and a text.
fn ws(files: &[(&str, &str)]) -> Result<Scan, Vec<Diagnostic>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    scan(&sources)
}

/// The single error of scanning `files`: its message, input, and the
/// inputs of its labels.
fn error(files: &[(&str, &str)]) -> (String, Option<u32>, Vec<Option<u32>>) {
    let errors = ws(files).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let error = &errors[0];
    let labels = error.labels.iter().map(|l| l.input).collect();
    (error.message.clone(), error.input, labels)
}

/// The files of the summary, in path order: app, common, lib,
/// workspace.
const SUMMARY: [(&str, &str); 4] = [
    ("workspace.gcsl", ""),
    (
        "common.gcsl",
        "version = \"1.2\"\ndef tagged(n: String): String { \"{n}-{version}\" }",
    ),
    (
        "app.gcsl",
        "com.x.app.name = tagged(\"app\")\ncom.x.app.deps = [com.x.lib]",
    ),
    (
        "lib.gcsl",
        "com.x.lib.name = tagged(\"lib\")\ncom.x.lib.src = \"lib-{version}.tar\"",
    ),
];

/// Checks the workspace of `files` and runs the file at `path`,
/// returning its value shown, or the messages and inputs of its
/// diagnostics.
fn run_file(files: &[(&str, &str)], path: &str) -> Result<String, Vec<(String, Option<u32>)>> {
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let shown = |errors: Vec<Diagnostic>| {
        errors
            .into_iter()
            .map(|error| (error.message, error.input))
            .collect::<Vec<_>>()
    };
    let mut workspace = check(&sources).map_err(shown)?;
    assert_imports_stored(&workspace);
    assert_assemblies_stored(&workspace);
    let index = workspace.file(path).unwrap();
    let outcome = workspace.run(index).map_err(shown)?;
    Ok(outcome.value.to_string())
}

/// Asserts that the driver wrote the cell of every built-in function
/// as it laid the frame out, before any file runs: a `Value::Builtin`,
/// never `Value::Empty`, which would read as a record-kinded `{}` that
/// nothing else can see. Returns their slots, which no file stores.
fn assert_builtins_written(workspace: &Workspace) -> Vec<Slot> {
    Builtin::ALL
        .into_iter()
        .zip(0u32..)
        .map(|(builtin, node)| {
            let slot = workspace.cell_slots[node as usize];
            assert_eq!(workspace.scan.builtin(node), Some(builtin));
            assert_eq!(
                workspace.frame.get(slot.index()),
                Some(&Value::Builtin(builtin)),
                "the cell of `{}` is not written by the driver",
                builtin.name()
            );
            slot
        })
        .collect()
}

/// Asserts that every cell an import of `workspace` reaches, directly
/// or as a field of a record read whole, is stored by a node of its
/// owner's top-level block: slice 24's schedule test sees one file
/// only, and a cell no owner writes would read as `{}`. A built-in
/// function's cell has no owner: the driver wrote it before any file
/// ran ([`assert_builtins_written`]).
fn assert_imports_stored(workspace: &Workspace) {
    let scan = &workspace.scan;
    let types = vec![TyId::ERROR; scan.cells()];
    let builtins = assert_builtins_written(workspace);
    for (index, file) in (0u32..).zip(&scan.files) {
        let reads = (&workspace.cell_slots[..], &[][..], &workspace.for_later);
        let context = Context::new(scan, index, &types, reads).unwrap();
        let mut pending: Vec<Target> = file
            .mentions
            .foreign
            .iter()
            .flatten()
            .filter_map(|&(need, _)| match need {
                Need::Field(n) => context.scope.target(n),
                Need::Function(_) => None,
            })
            .collect();
        while let Some(target) = pending.pop() {
            let cell = match target {
                Target::Node(node) => Some(context.cell_slot(node)),
                other => context.value_of(other).map(Slot::at),
            };
            let Some(slot) = cell else {
                pending.extend(context.target_fields(target).into_iter().map(|(_, t)| t));
                continue;
            };
            if builtins.contains(&slot) {
                continue;
            }
            let owners = scan.target_files(target);
            let stored = owners.iter().any(|&owner| {
                let Some(FileRun::Pending(ir)) = workspace.runs.get(owner as usize) else {
                    return false;
                };
                let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                    return false;
                };
                ir.items(items)
                    .iter()
                    .any(|&item| matches!(ir.get(item).kind, IrKind::Store(s, _) if s == slot))
            });
            assert!(
                stored,
                "{}: `{}` is not stored by {owners:?}",
                file.path,
                scan.show_target(target)
            );
        }
    }
}

/// The slots the top-level block of file `file` of `workspace` stores,
/// in order.
fn top_stores(workspace: &Workspace, file: u32) -> Vec<Slot> {
    let Some(FileRun::Pending(ir)) = workspace.runs.get(file as usize) else {
        return Vec::new();
    };
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        return Vec::new();
    };
    ir.items(items)
        .iter()
        .filter_map(|&item| match ir.get(item).kind {
            IrKind::Store(slot, _) => Some(slot),
            _ => None,
        })
        .collect()
}

/// Asserts that every record a file of `workspace` assembles of a node
/// of its view with a leaf of the file under it reads only cells stored
/// before it: the file's own by an earlier node of its top-level block,
/// every other one by the top-level block of a file it needs, which runs
/// whole before it. And that every record any file assembles at its top
/// level (an interior node's, a shared node's, an imported one) reads
/// only cells stored earlier in its block or by a file earlier in the
/// order: slice 25's shared nodes read the cells of the files that
/// contribute to them, which come earlier but need not be needed. A cell
/// nobody stored first would read as `{}`. A built-in function's cell
/// counts as stored before any file: the driver wrote it as it laid the
/// frame out ([`assert_builtins_written`]).
fn assert_assemblies_stored(workspace: &Workspace) {
    let scan = &workspace.scan;
    let builtins = assert_builtins_written(workspace);
    for (index, file) in (0u32..).zip(&scan.files) {
        let own = top_stores(workspace, index);
        let needed: Vec<Slot> = scan
            .needed(index)
            .into_iter()
            .filter(|&other| other != index)
            .flat_map(|other| top_stores(workspace, other))
            .chain(builtins.iter().copied())
            .collect();
        let rank = scan.order.iter().position(|&f| f == index).unwrap_or(0);
        let earlier: Vec<Slot> = scan.order[..rank]
            .iter()
            .flat_map(|&other| top_stores(workspace, other))
            .chain(builtins.iter().copied())
            .collect();
        let records = workspace.view_records.get(index as usize);
        for (slot, _, fields) in records.into_iter().flatten() {
            let Some(at) = own.iter().position(|s| s == slot) else {
                let path = &file.path;
                assert_eq!(*slot, Slot::NONE, "{path}: a view record is not stored");
                continue;
            };
            for &(name, cell, _) in fields {
                assert!(
                    own[..at].contains(&cell) || needed.contains(&cell),
                    "{}: the view record in {slot:?} reads `{}` from {cell:?}, stored by \
                         nothing before it",
                    file.path,
                    scan.names.get(name)
                );
            }
        }
        let Some(FileRun::Pending(ir)) = workspace.runs.get(index as usize) else {
            continue;
        };
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            continue;
        };
        let mut stored: Vec<Slot> = Vec::new();
        for &item in ir.items(items) {
            let IrKind::Store(slot, value) = ir.get(item).kind else {
                continue;
            };
            if let IrKind::MakeRecord { values, .. } = ir.get(value).kind {
                for &field in ir.items(values) {
                    if let IrKind::Load(cell, _) = ir.get(field).kind {
                        assert!(
                            stored.contains(&cell) || earlier.contains(&cell),
                            "{}: a record stored in {slot:?} reads {cell:?}, stored by \
                                 nothing before it",
                            file.path
                        );
                    }
                }
            }
            stored.push(slot);
        }
    }
}

/// The value of the file at `path` of the workspace of `files`.
fn value(files: &[(&str, &str)], path: &str) -> String {
    run_file(files, path).unwrap_or_else(|errors| panic!("{path}: {errors:?}"))
}

/// The messages of the diagnostics of running `path` in `files`.
fn messages(files: &[(&str, &str)], path: &str) -> Vec<String> {
    match run_file(files, path) {
        Ok(value) => panic!("{path}: unexpected value {value}"),
        Err(errors) => errors.into_iter().map(|(message, _)| message).collect(),
    }
}

// ----- The standard library -----

// ----- `exec`, `panic` and `sh` -----

// ----- `exec_live` and `sh_live` -----
//
// What `exec_live` copies goes to this process's standard error, which
// the test harness does not capture: these programs say little. The
// copy itself, and output larger than a pipe holds, are the CLI's tests.

// ----- Strings, `path`, and standard modules as scopes of their own -----

// ----- More string built-ins, and `compare_versions` -----

// ----- The `fs` module -----

// ----- `fs.write`, `sort_versions` and `show` -----

// ----- Syntax errors -----

// ----- Scan errors -----

mod arguments;
mod defers;
mod deftypes;
mod effects;
mod errors;
mod fileroot;
mod files;
mod fs;
mod imports;
mod invoke;
mod language;
mod locks;
mod memo;
mod merging;
mod metas;
mod needs;
mod shadowing;
mod shell;
mod standard;
mod strings;
mod summary;
mod toplevel;
mod types;
