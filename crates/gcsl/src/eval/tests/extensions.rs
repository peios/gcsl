//! Tests of the evaluator: rest-fields, versions, temporary directories,
//! shorthand arguments, quantities, generics and methods.

use super::*;

/// dpkg's `verrevcmp`, transcribed from its C as it is, a byte past the
/// end read as C's terminating zero, then the byte-wise tie-break of
/// `sort -V`: what [`compare_versions`] must agree with.
pub(super) fn dpkg_verrevcmp(a: &[u8], b: &[u8]) -> Ordering {
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    let order = |c: u8| -> i32 {
        if c.is_ascii_digit() {
            0
        } else if c.is_ascii_alphabetic() {
            i32::from(c)
        } else if c == b'~' {
            -1
        } else if c != 0 {
            i32::from(c) + 256
        } else {
            0
        }
    };
    let (mut i, mut j) = (0, 0);
    while at(a, i) != 0 || at(b, j) != 0 {
        let mut first_diff = 0;
        while (at(a, i) != 0 && !at(a, i).is_ascii_digit())
            || (at(b, j) != 0 && !at(b, j).is_ascii_digit())
        {
            let (ac, bc) = (order(at(a, i)), order(at(b, j)));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        while at(a, i) == b'0' {
            i += 1;
        }
        while at(b, j) == b'0' {
            j += 1;
        }
        while at(a, i).is_ascii_digit() && at(b, j).is_ascii_digit() {
            if first_diff == 0 {
                first_diff = i32::from(at(a, i)) - i32::from(at(b, j));
            }
            i += 1;
            j += 1;
        }
        if at(a, i).is_ascii_digit() {
            return Ordering::Greater;
        }
        if at(b, j).is_ascii_digit() {
            return Ordering::Less;
        }
        if first_diff != 0 {
            return first_diff.cmp(&0);
        }
    }
    a.cmp(b)
}

#[test]
fn typed_equality_compares_the_fields_a_rest_field_covers() {
    let prefix = "let a: {*: Int} = {x = 1; y = 2}\n\
                  let b: {*: Int} = {y = 2; x = 1}\n\
                  let c: {*: Int} = {x = 1; y = 3}\n\
                  let d: {*: Int} = {x = 1}\n\
                  let e: {*: Int} = {}\n\
                  let f: {n: String, *: Int} = {n = \"a\"; x = 1}\n\
                  let g: {n: String, *: Int} = {n = \"a\"; x = 2}\n\
                  let h: {n: String, *: Int} = {n = \"a\"}\n\
                  let i: {*: {*: Int}} = {p = {q = 1}}\n\
                  let j: {*: {*: Int}} = {p = {q = 1; r = 2}}\n";
    let cases = [
        // The same fields in another order, a value differing, a field
        // missing, and the empty record on either side.
        ("a == b", true),
        ("a == c", false),
        ("a == d", false),
        ("d == a", false),
        ("e == e", true),
        ("d == e", false),
        ("e == d", false),
        // Named fields and the others together.
        ("f == g", false),
        ("f != g", true),
        ("h == h", true),
        ("f == h", false),
        // Through a nested rest-field's shape.
        ("i == j", false),
        ("i == i", true),
    ];
    for (comparison, expected) in cases {
        assert_eq!(
            boolean(&format!("{prefix}{comparison}")),
            expected,
            "{comparison}"
        );
    }
}

#[test]
fn a_deep_await_awaits_the_fields_a_rest_field_covers() {
    let text = "let l: {n: Future<Int>, *: Future<Int>} = {n = lazy 1; p = lazy 1 + 1; q = lazy 3}\n\
                let m: {n: Int, *: Int} = l\n\
                let expected: {n: Int, *: Int} = {n = 1; p = 2; q = 3}\n\
                m == expected";
    assert!(boolean(text));
    let text = "let l: {*: {*: Future<String>}} = {a = {b = lazy \"x\"}}\n\
                let m: {*: {*: String}} = l\n\
                show(m)";
    assert_eq!(string(text), "{a = {b = \"x\"}}");
}

#[test]
fn as_tests_each_field_a_rest_field_covers() {
    let text = "let x = {a = 1; b = 2}\nlet y: {} = x\ny as {*: Int}";
    assert_eq!(run(text).unwrap().to_string(), "{a = 1; b = 2}");
    booleans(&[
        (
            "let x = {a = 1; b = \"s\"}\nlet y: {} = x\ny is {*: Int}",
            false,
        ),
        (
            "let x = {a = 1; b = \"s\"}\nlet y: {} = x\ny is {b: String, *: Int}",
            true,
        ),
        ("let y: {} = {}\ny is {*: Int}", true),
    ]);
    let text = "let x = {a = 1; \"b c\" = \"s\"}\nlet y: {a: Int} = x\ny as {*: Int}";
    let diagnostic = run(text).unwrap_err();
    assert_eq!(diagnostic.message, "`y` is not a `{*: Int}`");
    assert_eq!(
        diagnostic.label.as_deref(),
        Some("it is a record, and its field `\"b c\"` is a String")
    );
}

#[test]
fn compare_versions_is_dpkg_s_order_and_total() {
    // Every string of up to three pieces of a small alphabet that has
    // each kind of byte: digits, a zero, letters, `~`, others, and a
    // character of two bytes.
    let alphabet = ["0", "1", "9", "a", "Z", "~", ".", "+", "\u{e9}"];
    let mut all = vec![String::new()];
    let mut last = vec![String::new()];
    for _ in 0..3 {
        last = last
            .iter()
            .flat_map(|s| alphabet.iter().map(move |piece| format!("{s}{piece}")))
            .collect();
        all.extend(last.iter().cloned());
    }
    for a in &all {
        for b in &all {
            let order = compare_versions(a, b);
            assert_eq!(
                order,
                dpkg_verrevcmp(a.as_bytes(), b.as_bytes()),
                "{a:?} {b:?}"
            );
            assert_eq!(order, compare_versions(b, a).reverse(), "{a:?} {b:?}");
            assert_eq!(order.is_eq(), a == b, "{a:?} {b:?}");
        }
    }
    // A total order: sorted by it, no string is after a later one.
    all.sort_by(|a, b| compare_versions(a, b));
    for (at, a) in all.iter().enumerate() {
        for b in &all[at..] {
            assert!(compare_versions(a, b).is_le(), "{a:?} {b:?}");
        }
    }
    // Digit runs far longer than any integer.
    let nines = "9".repeat(100);
    assert_eq!(
        compare_versions(&format!("1.{nines}"), &format!("1.1{nines}")),
        Ordering::Less
    );
    assert_eq!(
        compare_versions(&format!("000{nines}"), &nines),
        Ordering::Less
    );
}

#[test]
fn temporary_directories_are_made_when_asked_and_removed_when_dropped() {
    // Nothing is made until the first is asked for.
    let temp_dirs = TempDirs::new();
    assert!(temp_dirs.parent.get().is_none());
    drop(temp_dirs);
    let temp_dirs = TempDirs::new();
    let first = temp_dirs.make().unwrap();
    let parent = temp_dirs.parent.get().unwrap().clone();
    assert_eq!(Path::new(&first), parent.join("1"));
    let name = parent.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with(&format!("gcsl-{}-", std::process::id())),
        "{name}"
    );
    // A number something else took is passed over.
    fs::create_dir(parent.join("2")).unwrap();
    assert_eq!(Path::new(&temp_dirs.make().unwrap()), parent.join("3"));
    // Two owners have two parents.
    let other = TempDirs::new();
    let elsewhere = other.make().unwrap();
    assert_ne!(Path::new(&elsewhere).parent(), Some(parent.as_path()));
    fs::write(parent.join("1/file"), "x").unwrap();
    drop(temp_dirs);
    assert!(!parent.exists());
    assert!(Path::new(&elsewhere).is_dir());
    drop(other);
    assert!(!Path::new(&elsewhere).exists());
}

#[test]
fn temporary_directories_are_made_in_the_temp_root() {
    let scratch = std::env::temp_dir().join(format!("gcsl-slice-45-{}-unit", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    // A missing root is made, with its parents, as the first is asked
    // for, never before; the parent is in it, and goes, the root stays.
    let root = scratch.join("a/b/root");
    let mut temp_dirs = TempDirs::new();
    temp_dirs.set_root(root.clone());
    assert!(!scratch.exists());
    let first = temp_dirs.make().unwrap();
    let parent = temp_dirs.parent.get().unwrap().clone();
    assert_eq!(parent.parent(), Some(root.as_path()));
    assert_eq!(Path::new(&first), parent.join("1"));
    let name = parent.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with(&format!("gcsl-{}-", std::process::id())),
        "{name}"
    );
    drop(temp_dirs);
    assert!(!parent.exists());
    assert!(root.is_dir());
    // Set after one was made: the parent made stays in use.
    let mut temp_dirs = TempDirs::new();
    let first = temp_dirs.make().unwrap();
    let parent = temp_dirs.parent.get().unwrap().clone();
    temp_dirs.set_root(root.clone());
    let second = temp_dirs.make().unwrap();
    assert_eq!(Path::new(&first).parent(), Some(parent.as_path()));
    assert_eq!(Path::new(&second), parent.join("2"));
    assert_eq!(parent.parent(), Some(std::env::temp_dir().as_path()));
    drop(temp_dirs);
    assert!(!parent.exists());
    // A root that is not absolute is every call's error.
    let mut temp_dirs = TempDirs::new();
    temp_dirs.set_root(PathBuf::from("rel/tmp"));
    for _ in 0..2 {
        assert_eq!(
            temp_dirs.make(),
            Err("the temporary directory root `rel/tmp` is not an absolute path".to_owned())
        );
    }
    assert!(temp_dirs.parent.get().is_none());
    assert!(!Path::new("rel").exists());
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn a_shorthand_argument_passes_its_variable_to_its_parameter() {
    // `1` fills `a`, `c = 3` fills `c`, and `b` alone, after a named
    // argument, is `b = b`.
    let value = run(
        "fn f(a: Int, b: Int, c: Int): Int { a * 100 + b * 10 + c }\n\
         let b = 2\n= f(1, c = 3, b)",
    )
    .unwrap();
    assert_eq!(value.to_string(), "123");
}

#[test]
fn quantities_compute_exactly() {
    let cases = [
        ("30s + 1m", "1m30s"),
        ("1h - 1ns", "59m59s999ms999us999ns"),
        ("3 * 20s", "1m"),
        ("1m * 3", "3m"),
        ("1h // 7", "8m34s285ms714us285ns"),
        ("-1h // 7", "-8m34s285ms714us286ns"),
        ("1h // 7m", "8"),
        ("1h % 7m", "4m"),
        ("-(1h30m)", "-1h30m"),
        ("2GiB + 512MiB", "2560MiB"),
        ("1GiB // 3", "357913941B"),
        ("30s < 1m", "true"),
        ("1000ms == 1s", "true"),
        ("2KiB > 2KB", "true"),
        ("\"in {1m30s}, {2GiB}\"", "in 1m30s, 2GiB"),
        ("[1s, 1KB]", "[1s, 1KB]"),
        ("{ t = 1h }", "{t = 1h}"),
    ];
    for (text, shown) in cases {
        assert_eq!(run(text).unwrap().to_string(), shown, "{text}");
    }
}

#[test]
fn a_quantity_that_overflows_says_so_with_its_operands_as_literals() {
    let labels = |text: &str| -> (String, Vec<String>) {
        let error = run(text).unwrap_err();
        let labels = error
            .labels
            .iter()
            .map(|label| label.text.clone())
            .collect();
        (error.message, labels)
    };
    assert_eq!(
        labels("106751d * 2"),
        (
            "duration overflow".to_owned(),
            vec!["106751d".to_owned(), "2".to_owned()]
        )
    );
    assert_eq!(
        labels("8191PiB + 8191PiB"),
        (
            "size overflow".to_owned(),
            vec!["8191PiB".to_owned(), "8191PiB".to_owned()]
        )
    );
    assert_eq!(
        labels("let least = -106751d23h47m16s854ms775us807ns - 1ns\n-least"),
        (
            "duration overflow".to_owned(),
            vec!["-106751d23h47m16s854ms775us808ns".to_owned()]
        )
    );
    assert_eq!(
        labels("30s // 0"),
        (
            "division by zero".to_owned(),
            vec!["30s".to_owned(), "0".to_owned()]
        )
    );
    // An `Int`'s is as ever.
    assert_eq!(labels(&format!("{MAX} * 2")).0, "integer overflow");
}

#[test]
fn quantities_are_memo_keys_and_tell_apart_with_is() {
    let text = "fn f(t: Duration) ->> {v: Duration} { v = t }\n\
                    a = f(1s)\nb = f(1000ms)\nc = f(2s)";
    let values = fields(text);
    assert!(same(&values[0], &values[1]));
    assert!(!same(&values[0], &values[2]));
    let text = "fn pick(x: Duration | Size | Int): String {\n\
                    \x20   if x is Duration { \"time {x}\" } else if x is Size { \"data {x}\" } else { \"{x}\" }\n\
                    }\n\
                    r = [pick(1s), pick(1B), pick(1)]";
    assert_eq!(
        fields(text)[0].to_string(),
        r#"["time 1s", "data 1B", "1"]"#
    );
}

#[test]
fn a_generic_function_passes_any_value_through() {
    let text = "fn id<T>(x: T): T { x }\n\
                fn wrap<T>(x: T): {v: T} { v = x }\n\
                fn twice<T>(x: T): List<T> { [x, x] }\n\
                a = id(1) + 1\nb = id(\"s\")\nc = wrap([1, 2]).v\nd = twice(2.5)\n\
                e = id({ n = 3 }).n";
    let shown: Vec<String> = fields(text).iter().map(ToString::to_string).collect();
    assert_eq!(shown, ["2", "s", "[1, 2]", "[2.5, 2.5]", "3"]);
    // A lazy value too, which the caller awaits.
    let text = "fn id<T>(x: T) -> T { x }\nfn later() -> Int { 5 }\nr = await id(lazy later())";
    assert_eq!(fields(text)[0].to_string(), "5");
    // Recursion binds its own parameters.
    let text = "fn last<T>(xs: List<T>, at: Int, fallback: T): T {\n\
                \x20   if at >= len(xs) { fallback } else if at == len(xs) - 1 { xs[at] } else { last(xs, at + 1, fallback) }\n\
                }\nr = last([1, 2, 3], 0, 0)";
    assert_eq!(fields(text)[0].to_string(), "3");
}

#[test]
fn a_method_call_passes_its_receiver_first() {
    let text = "fn (x: Int) add(y: Int = 10, z: Int = x * 2): Int { x + y + z }\n\
                fn (x: Int) fact(): Int { if x <= 1 { 1 } else { x * (x - 1).fact() } }\n\
                fn (xs: List<T>) first<T>(fallback: T): T { if len(xs) > 0 { xs[0] } else { fallback } }\n\
                fn later() -> Int { 5 }\n\
                def (x: Int) inc() -> Int { x + 1 }\n\
                fn outer(n: Int): Int { n.add(0, 0) }\n\
                a = 3.add()\nb = 3.add(1)\nc = 3.add(y = 5)\nd = 5.fact()\n\
                e = [4, 5].first(0)\nf = [\"x\"].first(\"none\")\ng = await (lazy later()).inc()\nh = outer(7)";
    let shown: Vec<String> = fields(text).iter().map(ToString::to_string).collect();
    assert_eq!(shown, ["19", "10", "14", "120", "4", "x", "6", "7"]);
}
