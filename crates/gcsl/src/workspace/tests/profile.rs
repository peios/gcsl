//! Tests of workspaces: profiling their runs.

use super::*;
use crate::eval::profile::{Profiled, ProfileReport};
use crate::ir::FnKind;

/// The workspace of the profiling tests: `twice`, a plain function; `memo`,
/// a memoised one that calls it and the built-ins `trim` and `split`; and
/// `down`, which calls itself.
const PROFILED: [(&str, &str); 2] = [
    ("workspace.gcsl", ""),
    (
        "app/main.gcsl",
        "fn twice(x: Int): Int { x * 2 }\n\
         fn memo(x: Int) ->> Int { twice(x) + len(split(trim(\" a b \"), \" \")) }\n\
         fn down(n: Int): Int { if n == 0 { 0 } else { down(n - 1) } }\n\
         value = memo(1) + memo(1) + memo(2) + down(3) + twice(twice(1))",
    ),
];

/// The profile of running `app/main.gcsl` of [`PROFILED`], profiled from
/// the start if `profiled`, and the file's value.
fn profiled_run(profiled: bool) -> (Option<ProfileReport>, String) {
    let sources: Vec<SourceFile> = PROFILED
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
    let (_, checked) = check_with_standard(&sources, &[], "/w");
    let mut workspace = checked.unwrap();
    if profiled {
        workspace.start_profiling();
    }
    let index = workspace.file("app/main.gcsl").unwrap();
    let value = workspace.run(index).unwrap().value.to_string();
    (workspace.profile_report(), value)
}

/// The row of `report` of the function named `name`: its calls, how many
/// found a memo entry, and its kind.
fn function_row(report: &ProfileReport, name: &str) -> (u64, u64, FnKind) {
    let rows: Vec<(u64, u64, FnKind)> = report
        .rows
        .iter()
        .filter_map(|row| match &row.site {
            Profiled::Function {
                name: Some(named),
                kind,
                ..
            } if named == name => Some((row.calls, row.found, *kind)),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 1, "one row of `{name}`: {:?}", report.rows);
    rows[0]
}

#[test]
fn a_profile_counts_each_function_s_calls_and_its_memo_entries_found() {
    let (report, value) = profiled_run(true);
    assert_eq!(value, "{value = 18}");
    let report = report.unwrap();
    // `memo(1)`'s and `memo(2)`'s bodies, and two in the file.
    assert_eq!(function_row(&report, "twice"), (4, 0, FnKind::Function));
    // The second `memo(1)` finds the first's entry.
    assert_eq!(function_row(&report, "memo"), (3, 1, FnKind::Memo));
    // `down(3)` and the three calls it makes inside itself.
    assert_eq!(function_row(&report, "down"), (4, 0, FnKind::Function));
    let builtins: Vec<(&str, u64)> = report
        .rows
        .iter()
        .filter_map(|row| match row.site {
            Profiled::Builtin(name) => Some((name, row.calls)),
            Profiled::Function { .. } => None,
        })
        .collect();
    assert_eq!(builtins.len(), 2, "{builtins:?}");
    assert!(builtins.contains(&("trim", 2)) && builtins.contains(&("split", 2)));
}

#[test]
fn a_profile_s_own_times_add_up_to_its_outermost_time() {
    let (report, _) = profiled_run(true);
    let report = report.unwrap();
    let own: std::time::Duration = report.rows.iter().map(|row| row.own).sum();
    assert_eq!(own, report.outermost);
    // A function's total is never less than its own time, and a recursive
    // one's counts its outermost calls only, so no more than the run's.
    for row in &report.rows {
        assert!(row.total >= row.own, "{row:?}");
        assert!(row.total <= report.outermost, "{row:?}");
    }
    // The rows come longest own time first.
    assert!(
        report
            .rows
            .windows(2)
            .all(|pair| pair[0].own >= pair[1].own)
    );
}

#[test]
fn a_workspace_not_profiled_has_no_profile() {
    let (report, value) = profiled_run(false);
    assert!(report.is_none());
    assert_eq!(value, "{value = 18}");
}

#[test]
fn a_profile_of_graph_mode_counts_each_call_once_however_often_its_unit_stops() {
    let (mut workspace, file) = super::graph::lone_workspace(
        "fn plan(n: Int) ->> String {\n\
             let a = exec([\"echo\", \"a\"]).stdout.trim()\n\
             let b = exec([\"echo\", \"b\"]).stdout.trim()\n\
             \"{a}{b}{n}\"\n\
         }\n\
         = [compile(plan(1)), compile(plan(1))]",
    );
    workspace.start_profiling();
    super::graph::drive(&mut workspace, file);
    let report = workspace.profile_report().unwrap();
    // `plan`'s body stops at each `exec`, a node, and takes up where it
    // stopped once the node's result is given, never running again: one
    // call that ran, and the second, which finds the entry it made.
    assert_eq!(function_row(&report, "plan"), (2, 1, FnKind::Memo));
    // Each effect's node runs its built-in once, as the host asks.
    let exec: Vec<u64> = report
        .rows
        .iter()
        .filter(|row| row.site == Profiled::Builtin("exec"))
        .map(|row| row.calls)
        .collect();
    assert_eq!(exec, [2]);
    // The two calls of `compile` are one step, whose run is one call.
    assert_eq!(function_row(&report, "compile"), (1, 0, FnKind::Function));
    let own: std::time::Duration = report.rows.iter().map(|row| row.own).sum();
    assert_eq!(own, report.outermost);
}
