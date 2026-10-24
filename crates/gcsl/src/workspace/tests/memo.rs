//! Tests of workspaces: memoised calls and the callers they read.

use super::*;

/// The library of the memoisation tests: `reads`, which reads its
/// caller; `plain`, which does not; `outer`, a helper whose callee reads
/// the caller it passes on; `outer_hit`, a helper whose memoised callee,
/// `reads`, may be found; `awaits`, which reads its caller in a `lazy`
/// it awaits while it runs; `again`, which calls itself with its own
/// arguments, from its own file, once it has read a caller of another;
/// and four that return a step: `step`, whose `lazy` reads its caller,
/// `plain_step`, whose does not, `helped`, whose is made after
/// `fileroot.helper()` and calls `inner`, `nested`, whose makes a `lazy`
/// that reads it, and `fn_step`, whose makes a function that reads its
/// own.
pub(super) const MEMO_LIB: &str = "import fileroot\n\
        def reads(x: Int) ->> {c: String} { c = fileroot.caller }\n\
        def plain(x: Int) ->> {v: Int} { v = x }\n\
        def inner(): String { fileroot.caller }\n\
        def outer(x: Int) ->> {c: String} {\n\
            fileroot.helper()\n\
            c = inner()\n\
        }\n\
        def outer_hit(x: Int) ->> {c: String} {\n\
            fileroot.helper()\n\
            reads(x)\n\
        }\n\
        def awaits(x: Int) ->> {c: String} {\n\
            let made = lazy { fileroot.caller }\n\
            c = await made\n\
        }\n\
        def again(x: Int) ->> String {\n\
            let c = fileroot.caller\n\
            if c == fileroot.this { c } else { again(x) + \" \" + c }\n\
        }\n\
        def step(x: Int) ->> {s: Future<String>} { s = lazy { fileroot.caller } }\n\
        def plain_step(x: Int) ->> {s: Future<Int>} { s = lazy { x + 1 } }\n\
        def helped(x: Int) ->> {s: Future<String>} {\n\
            fileroot.helper()\n\
            s = lazy { inner() }\n\
        }\n\
        def nested(x: Int) ->> {s: Future<String>} { s = lazy { await lazy { fileroot.caller } } }\n\
        def fn_step(x: Int) ->> {s: Future<fn(): String>} { s = lazy { fn(): String { fileroot.caller } } }";

/// Whether the values of the fields of `record` at `a` and `b` are one
/// record: one run of the memoised function that made both.
pub(super) fn one_run(record: &Value, (a, b): (usize, usize)) -> bool {
    let Value::Record(record) = record else {
        panic!("expected a record, got {record}");
    };
    match (record.values.get(a), record.values.get(b)) {
        (Some(Value::Record(a)), Some(Value::Record(b))) => Rc::ptr_eq(a, b),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn a_memoised_call_depends_on_the_caller_it_reads() {
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        (
            "p/a.gcsl",
            "import lib/m\nr = m.reads(1)\nv = m.plain(1)\no = m.outer(1)\nw = m.awaits(1)",
        ),
        (
            "p/b.gcsl",
            "import lib/m\nrb = m.reads(1)\nvb = m.plain(1)\nob = m.outer(1)\nwb = m.awaits(1)",
        ),
        (
            "q/c.gcsl",
            "import lib/m\nr = m.reads(1)\nv = m.plain(1)\no = m.outer(1)\nw = m.awaits(1)",
        ),
        (
            "z/all.gcsl",
            "import p/a\nimport p/b\nimport q/c\n\
                 ra = a.r\nrb = b.rb\nrc = c.r\n\
                 va = a.v\nvb = b.vb\nvc = c.v\n\
                 oa = a.o\nob = b.ob\noc = c.o\n\
                 wa = a.w\nwb = b.wb\nwc = c.w",
        ),
    ];
    let all = rooted_value(&files, "z/all.gcsl");
    assert_eq!(
        all.to_string().replace(SCRATCH, "<root>"),
        "{ra = {c = \"<root>/p\"}; rb = {c = \"<root>/p\"}; rc = {c = \"<root>/q\"}; \
             va = {v = 1}; vb = {v = 1}; vc = {v = 1}; \
             oa = {c = \"<root>/p\"}; ob = {c = \"<root>/p\"}; oc = {c = \"<root>/q\"}; \
             wa = {c = \"<root>/p\"}; wb = {c = \"<root>/p\"}; wc = {c = \"<root>/q\"}}"
    );
    // A `lazy` it awaits while it runs reads its caller as it would.
    assert!(one_run(&all, (9, 10)));
    assert!(!one_run(&all, (9, 11)));
    // Reading the caller: one run for two files of one directory, whose
    // callers are equal, and another for another directory.
    assert!(one_run(&all, (0, 1)));
    assert!(!one_run(&all, (0, 2)));
    // Not reading it: one run from every directory.
    assert!(one_run(&all, (3, 4)));
    assert!(one_run(&all, (3, 5)));
    // A helper whose callee reads the caller it passed on depends on it.
    assert!(one_run(&all, (6, 7)));
    assert!(!one_run(&all, (6, 8)));
}

#[test]
fn a_memoised_call_found_passes_its_dependencies_on() {
    // In `p`, `outer_hit(1)` finds `reads(1)`, which read `p`, as `first`
    // ran it: `outer_hit`'s entry depends on the caller through the hit,
    // so `q` runs it again rather than finding `p`'s.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        (
            "p/a.gcsl",
            "import lib/m\nfirst = m.reads(1)\nhit = m.outer_hit(len([first]))",
        ),
        ("q/c.gcsl", "import lib/m\nhit = m.outer_hit(1)"),
        (
            "z/all.gcsl",
            "import p/a\nimport q/c\nfirst = a.first\nhit = a.hit\nother = c.hit",
        ),
    ];
    let all = rooted_value(&files, "z/all.gcsl");
    assert_eq!(
        all.to_string().replace(SCRATCH, "<root>"),
        "{first = {c = \"<root>/p\"}; hit = {c = \"<root>/p\"}; other = {c = \"<root>/q\"}}"
    );
    // `p`'s `outer_hit` found the entry `first` made.
    assert!(one_run(&all, (0, 1)));
    assert!(!one_run(&all, (1, 2)));
}

#[test]
fn a_step_that_can_read_its_caller_is_a_dependency_of_its_maker() {
    // Each step is awaited after the memoised call that made it has
    // returned, in a third directory.
    let made = "import lib/m\nst = m.step(1)\nps = m.plain_step(1)\n\
                    hs = m.helped(1)\nns = m.nested(1)\nfs = m.fn_step(1)";
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        ("p/a.gcsl", made),
        ("q/c.gcsl", made),
        (
            "z/all.gcsl",
            "import p/a\nimport q/c\n\
                 sa = await a.st.s\nsc = await c.st.s\n\
                 pa = a.ps\npc = c.ps\n\
                 ha = await a.hs.s\nhc = await c.hs.s\n\
                 na = await a.ns.s\nnc = await c.ns.s\n\
                 fa = a.fs\nfc = c.fs",
        ),
    ];
    let all = rooted_value(&files, "z/all.gcsl");
    let Value::Record(record) = &all else {
        panic!("expected a record, got {all}");
    };
    let shown: Vec<String> = record
        .values
        .iter()
        .map(|value| value.to_string().replace(SCRATCH, "<root>"))
        .collect();
    // A step reading its caller: one run of its maker for each
    // directory, each step reading its own, as the maker's caller.
    assert_eq!(shown[0], "<root>/p");
    assert_eq!(shown[1], "<root>/q");
    // A step that cannot read it: one run from every directory.
    assert!(one_run(&all, (2, 3)));
    // Made after `fileroot.helper()`, its calls pass the caller on.
    assert_eq!(shown[4], "<root>/p");
    assert_eq!(shown[5], "<root>/q");
    // A step making a step that reads it.
    assert_eq!(shown[6], "<root>/p");
    assert_eq!(shown[7], "<root>/q");
    // A function a step makes reads its own caller, not the step's: one
    // run from every directory.
    assert!(one_run(&all, (8, 9)));
}

#[test]
fn a_running_memoised_call_is_found_as_a_lookup_finds_it() {
    // `again(1)` from `p` has read `p` when it calls `again(1)` from
    // `lib`: the running entry depends on `p`, so the call from `lib` is
    // not the call depending on its own result, and runs.
    let files = [
        ("workspace.gcsl", ""),
        ("lib/m.gcsl", MEMO_LIB),
        ("p/a.gcsl", "import lib/m\nr = m.again(1)"),
    ];
    assert_eq!(
        rooted_text(&files, "p/a.gcsl"),
        "{r = \"<root>/lib <root>/p\"}"
    );
}
