use super::*;

fn ints(range: std::ops::Range<i64>) -> List {
    List::from_vec(range.map(Value::Int).collect())
}

fn as_ints(list: &List) -> Vec<i64> {
    list.values()
        .iter()
        .map(|value| match value {
            Value::Int(n) => *n,
            other => panic!("not an int: {other}"),
        })
        .collect()
}

/// Whether every join of `list` is balanced, its children of heights at
/// most one apart, with its height and length right; its height.
fn check_balanced(list: &List) -> u32 {
    match list.0.as_deref() {
        None | Some(Node::Leaf(_)) => 0,
        Some(Node::Join(join)) => {
            let (left, right) = (check_balanced(&join.left), check_balanced(&join.right));
            assert!(left.abs_diff(right) <= 1, "unbalanced: {left} and {right}");
            assert_eq!(join.height, left.max(right) + 1);
            assert_eq!(join.len, join.left.len() + join.right.len());
            assert!(!join.left.is_empty() && !join.right.is_empty());
            join.height
        }
    }
}

#[test]
fn joins_keep_the_elements_in_order_and_stay_balanced() {
    // Appending one element at a time, prepending, and joining lists of
    // every mix of lengths.
    let mut appended = List::EMPTY;
    let mut prepended = List::EMPTY;
    for n in 0..2000 {
        appended = appended.concat(&ints(n..n + 1));
        prepended = ints(1999 - n..2000 - n).concat(&prepended);
        check_balanced(&appended);
        check_balanced(&prepended);
    }
    assert_eq!(as_ints(&appended), (0..2000).collect::<Vec<_>>());
    assert_eq!(as_ints(&prepended), (0..2000).collect::<Vec<_>>());
    let height = check_balanced(&appended);
    assert!(
        height <= 12,
        "height {height} for 2000 elements in short leaves"
    );
    for (a, b) in [(0, 40), (40, 0), (3, 500), (500, 3), (100, 100), (33, 34)] {
        let joined = appended
            .concat(&ints(0..a))
            .concat(&ints(0..b))
            .concat(&prepended);
        check_balanced(&joined);
        assert_eq!(joined.len(), 4000 + usize::try_from(a + b).unwrap());
    }
}

#[test]
fn short_lists_join_into_one_leaf_and_long_ones_share_their_elements() {
    let short = ints(0..10).concat(&ints(10..20));
    assert!(matches!(short.0.as_deref(), Some(Node::Leaf(values)) if values.len() == 20));
    let long = ints(0..100);
    let joined = long.concat(&ints(100..200));
    let Some(Node::Join(join)) = joined.0.as_deref() else {
        panic!("not a join");
    };
    assert!(join.left.shares(&long));
    assert_eq!(as_ints(&joined), (0..200).collect::<Vec<_>>());
    assert!(List::EMPTY.concat(&long).shares(&long));
    assert!(long.concat(&List::EMPTY).shares(&long));
}

#[test]
fn an_element_is_found_by_its_position_without_a_slice() {
    let mut list = List::EMPTY;
    for n in 0..300 {
        list = list.concat(&ints(n * 7..n * 7 + 7));
    }
    for at in [0_usize, 6, 7, 500, 2099] {
        assert_eq!(list.get(at), Some(&Value::Int(i64::try_from(at).unwrap())));
    }
    assert_eq!(list.get(2100), None);
    let Some(Node::Join(join)) = list.0.as_deref() else {
        panic!("not a join");
    };
    assert!(
        join.flat.get().is_none(),
        "reading by position made a slice"
    );
    assert_eq!(list.values().len(), 2100);
    assert!(join.flat.get().is_some());
}

#[test]
fn replacing_an_element_copies_its_path_and_leaves_the_old_list() {
    let long = ints(0..1000);
    let replaced = long.with(500, Value::Int(-1)).unwrap();
    check_balanced(&replaced);
    assert_eq!(long.get(500), Some(&Value::Int(500)));
    assert_eq!(replaced.get(500), Some(&Value::Int(-1)));
    assert_eq!(replaced.len(), 1000);
    // The leaf was split, so the next replacement copies one short leaf.
    let again = replaced.with(10, Value::Int(-2)).unwrap();
    check_balanced(&again);
    let expected: Vec<i64> = (0..1000)
        .map(|n| match n {
            10 => -2,
            500 => -1,
            n => n,
        })
        .collect();
    assert_eq!(as_ints(&again), expected);
    assert_eq!(long.with(1000, Value::Int(0)), None);
}

#[test]
fn a_list_nested_in_lists_deeply_drops_without_recursion() {
    let mut list = ints(0..40);
    for _ in 0..200_000 {
        list = List::from_vec(vec![Value::List(list)]).concat(&ints(0..40));
    }
    drop(list);
}
