//! Tests of the evaluator: top-level fields computed at their points.

use super::*;

/// Asserts the static schedule of the cells of `ir`'s top-level fields
/// (`cells`): walking the top-level block's items in order, every cell
/// is stored before any item that `Load`s it, and before the first item
/// that loads or makes a function whose body, or the body of a function
/// it reads in turn, reads the cell through the frame (`Top`). A cell
/// read before its point holds `Value::Empty`, a valid `{}`, so a
/// scheduling bug would otherwise go unseen. Iterative throughout.
pub(super) fn assert_cells_scheduled(ir: &Ir, cells: &[Slot], text: &str) {
    let cell = |slot: Slot| cells.contains(&slot) && slot != Slot::NONE;
    // What each node reads: the cells it loads or reads through the
    // frame, the functions it makes, and the top-level slots it reads
    // through the frame as functions.
    let reads = |root: IrId, top_level: bool| {
        let (mut loads, mut fns, mut tops) = (Vec::new(), Vec::new(), Vec::new());
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            match ir.get(id).kind {
                IrKind::Load(slot, _) if top_level => loads.push(slot),
                IrKind::Top(slot, _) => tops.push(slot),
                IrKind::MakeClosure { function, .. } | IrKind::MakeFuture { function, .. } => {
                    fns.push(function);
                }
                _ => {}
            }
            stack.extend(ir.children(id));
        }
        (loads, fns, tops)
    };
    // The slots a function's closure lives in, by the top-level `Store`
    // of its `MakeClosure`, so that a `Top` of a function is followed.
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        return;
    };
    let items = ir.items(items);
    let mut closure_at: Vec<(Slot, FnId)> = Vec::new();
    for &item in items {
        if let IrKind::Store(slot, value) = ir.get(item).kind
            && let IrKind::MakeClosure { function, .. } = ir.get(value).kind
        {
            closure_at.push((slot, function));
        }
    }
    let mut stored: Vec<Slot> = Vec::new();
    for &item in items {
        let (loads, fns, tops) = reads(item, true);
        let mut needed: Vec<Slot> = loads;
        // Functions this item reaches, followed through frame reads.
        let mut work: Vec<FnId> = fns;
        work.extend(
            closure_at
                .iter()
                .filter(|(s, _)| needed.contains(s) || tops.contains(s))
                .map(|&(_, f)| f),
        );
        // Making a top-level function's closure mentions nothing: its
        // body reads cells when it runs, not when it is made.
        if closure_at.iter().any(
            |&(slot, _)| matches!(ir.get(item).kind, IrKind::Store(stored, _) if stored == slot),
        ) {
            work.clear();
        }
        let mut seen: Vec<FnId> = Vec::new();
        while let Some(function) = work.pop() {
            if seen.contains(&function) {
                continue;
            }
            seen.push(function);
            let (_, inner, frame) = reads(ir.function(function).body, false);
            work.extend(inner);
            work.extend(
                closure_at
                    .iter()
                    .filter(|(s, _)| frame.contains(s))
                    .map(|&(_, f)| f),
            );
            needed.extend(frame);
        }
        for slot in needed.into_iter().filter(|&slot| cell(slot)) {
            assert!(
                stored.contains(&slot),
                "{text}: cell {slot} read before its store"
            );
        }
        if let IrKind::Store(slot, _) = ir.get(item).kind
            && cell(slot)
        {
            stored.push(slot);
        }
    }
}

#[test]
fn top_level_fields_are_computed_at_their_points() {
    let last = |text: &str| format!("{:?}", fields(text).last().cloned());
    let int = |value: i64| format!("{:?}", Some(Value::Int(value)));
    // A dotted root read between its items is the whole root.
    assert_eq!(last("a.b = 1\nc = a\na.d = 2\nr = c.d"), int(2));
    assert_eq!(last("a.b = 1\nc = a.b + a.d\na.d = 2"), int(3));
    // A field used above its line, by a `let` and through a function.
    assert_eq!(
        last("let x = y + 1\ny = g()\nfn g(): Int { 41 }\nr = x"),
        int(42)
    );
    assert_eq!(
        last("let x = f()\nfn f(): Int { k.v }\nk.v = 5\nr = x"),
        int(5)
    );
    // A function reads a field through the frame before and after it.
    assert_eq!(
        last("fn f(): Int { n }\nlet x = f()\nn = 3\nlet y = f()\nr = x + y"),
        int(6)
    );
    // A `lazy` reads a field declared below it.
    assert_eq!(last("let l = lazy { m * 2 }\nm = 4\nr = await l"), int(8));
}

#[test]
fn the_fields_of_a_package_may_be_written_in_any_order() {
    let text = "let major = 2\ncom.x.app.deps = [com.x.lib]\n\
                    com.x.app.name = \"app-{version}\"\ncom.x.lib.name = \"lib\"\n\
                    version = describe(major)\nfn describe(n: Int): String { \"v{n}\" }";
    assert_eq!(
        run(text).unwrap().to_string(),
        "{com = {x = {app = {deps = [{name = \"lib\"}]; name = \"app-v2\"}; \
             lib = {name = \"lib\"}}}; version = \"v2\"}"
    );
}

#[test]
fn long_chains_of_fields_need_no_deep_recursion() {
    // The first item needs the whole chain, written below it, and each
    // dotted field reads the one before it: neither recurses per field.
    let count = 3000;
    let plain = (1..count).map(|index| format!("x{index} = x{} + 1\n", index + 1));
    let dotted = (2..=count).map(|index| format!("a.f{index} = a.f{} + 1\n", index - 1));
    let text: String = std::iter::once("total = x1\n".to_string())
        .chain(plain)
        .chain([format!("x{count} = 0\na.f1 = 1\n")])
        .chain(dotted)
        .chain(["last = a.f3000".to_string()])
        .collect();
    let value = run(&text).unwrap().to_string();
    assert!(value.starts_with("{total = 2999; "), "{}", &value[..40]);
    assert!(value.ends_with("last = 3000}"), "{value}");
}

#[test]
fn a_top_level_field_is_computed_once_with_its_effects_at_its_point() {
    // Mentioned by several items above it, a field is stored once.
    let text = "let a = x + 1\nlet b = x * 2\nlet c = { x }\nx = 5\nr = a + b + c";
    assert_eq!(fields(text), vec![Value::Int(5), Value::Int(21)]);
    let ir = compile(text);
    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
        panic!("{text}: the top level is not a block");
    };
    let stores = ir
        .items(items)
        .iter()
        .filter(|&&item| {
            matches!(ir.get(item).kind, IrKind::Store(_, value)
                    if matches!(ir.get(value).kind, IrKind::IntConst(5)))
        })
        .count();
    assert_eq!(stores, 1, "{text}");
    // A field that reads a mutable variable is computed at its own line,
    // with the value the variable has there.
    assert_eq!(
        fields("let mut t = 1\nf = t\nt := t + 1\ng = t\nr = f * 10 + g"),
        vec![Value::Int(1), Value::Int(2), Value::Int(12)]
    );
    // A field's effects happen where it is computed: before the first
    // item that needs it, here above an item that would fail first if
    // the field were computed at its own line; through an impure
    // function too.
    let index = run("[1][5]").unwrap_err().message;
    for text in [
        "let a = b\nlet c = 1 // 0\nb = [1][5]",
        "let a = b\nlet c = 1 // 0\nb = get()\nfn get() -> Int { [1][5] }",
    ] {
        assert_eq!(run(text).unwrap_err().message, index, "{text}");
    }
}

#[test]
fn a_discarded_call_runs_with_what_it_defers() {
    // The call runs where it stands, failing here before the result.
    let index = run("[1][5]").unwrap_err().message;
    for text in [
        "fn boom() -> Int { [1][5] }\nboom()\nx = 1",
        "fn boom() -> Int { [1][5] }\nfn f() -> Int {\n    boom()\n    = 1\n}\nx = f()",
        // What it defers runs too, rather than being lost with its value.
        "fn later() -> Int { = lazy [1][5] }\nlater()\nx = 1",
    ] {
        assert_eq!(run(text).unwrap_err().message, index, "{text}");
    }
    assert_eq!(
        int("fn one() -> Int { 1 }\nfn f() -> Int {\n    one()\n    = 2\n}\nf()"),
        2
    );
}
