use super::*;

/// A table and names with `a`, `b`, `c` interned in that order.
fn setup() -> (TypeTable, Names, [Name; 3]) {
    let mut names = Names::new();
    let abc = [names.intern("a"), names.intern("b"), names.intern("c")];
    (TypeTable::new(), names, abc)
}

#[test]
fn the_constants_are_pre_interned_in_order() {
    let mut table = TypeTable::new();
    assert_eq!(table.len(), 12);
    assert!(!table.is_empty());
    assert_eq!(table.intern(TyKind::Int), TyId::INT);
    assert_eq!(table.intern(TyKind::Float), TyId::FLOAT);
    assert_eq!(table.intern(TyKind::Bool), TyId::BOOL);
    assert_eq!(table.intern(TyKind::String), TyId::STRING);
    assert_eq!(table.record([]), TyId::EMPTY);
    assert_eq!(table.intern(TyKind::UntypedInt), TyId::UNTYPED_INT);
    assert_eq!(table.intern(TyKind::Error), TyId::ERROR);
    assert_eq!(
        table.intern(TyKind::Quantity(Quantity::Duration)),
        TyId::DURATION
    );
    assert_eq!(table.intern(TyKind::Quantity(Quantity::Size)), TyId::SIZE);
    assert_eq!(table.intern(TyKind::File), TyId::FILE);
    assert_eq!(table.intern(TyKind::Dir), TyId::DIR);
    assert_eq!(table.intern(TyKind::Sandbox), TyId::SANDBOX);
    assert_eq!(table.len(), 12);
    assert_eq!(TypeTable::default().len(), 12);
    // Each quantity's type, and back.
    for quantity in [Quantity::Duration, Quantity::Size] {
        assert_eq!(TyId::of_quantity(quantity).quantity(), Some(quantity));
    }
    assert_eq!(TyId::INT.quantity(), None);
}

#[test]
fn a_kind_is_small() {
    // A record type's defaults and rest-field are behind a thin
    // pointer, which a record type with neither, nearly every one,
    // leaves null: so a record type is 24 bytes. A function type's
    // signature is the largest kind: its parameters' box, its result, its
    // meta, its effect and its generic's 4-byte id, which a 16-bit id
    // would fit in 32 bytes only by limiting a table to 65535 generic
    // functions.
    assert_eq!(size_of::<RecordTy>(), 24);
    assert_eq!(size_of::<TyKind>(), 40);
}

#[test]
fn a_record_types_defaults_change_nothing_it_admits() {
    let (mut table, _, [a, b, c]) = setup();
    let one = || Constant::Int(1);
    let plain = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    let defaulted =
        table.record_with_defaults([(a, TyId::INT), (b, TyId::STRING)], vec![(a, one())]);
    // A type of its own, interned by its defaults too, in any order.
    assert_ne!(defaulted, plain);
    assert_eq!(
        table.record_with_defaults([(b, TyId::STRING), (a, TyId::INT)], vec![(a, one())]),
        defaulted
    );
    let other = table.record_with_defaults(
        [(a, TyId::INT), (b, TyId::STRING)],
        vec![(a, Constant::Int(2))],
    );
    assert_ne!(other, defaulted);
    // No default is none.
    assert_eq!(
        table.record_with_defaults([(a, TyId::INT), (b, TyId::STRING)], Vec::new()),
        plain
    );
    assert_eq!(table.defaults(defaulted), &[(a, one())][..]);
    assert_eq!(table.field_default(defaulted, a), Some(&one()));
    assert_eq!(table.field_default(defaulted, b), None);
    assert!(table.defaults(plain).is_empty());
    assert!(table.defaults(TyId::INT).is_empty());
    // Each a subtype of the other, and one type once they are
    // forgotten, at any depth.
    assert!(table.is_subtype(plain, defaulted));
    assert!(table.is_subtype(defaulted, plain));
    assert!(table.same(plain, defaulted));
    assert!(table.same(defaulted, other));
    let (plain_list, defaulted_list) = (table.list(plain), table.list(defaulted));
    assert!(table.same(plain_list, defaulted_list));
    let plain_fn = table.fn_type([(Some(c), plain)], plain_list, Effect::Pure);
    let defaulted_fn = table.fn_type([(Some(c), defaulted)], defaulted_list, Effect::Pure);
    assert!(table.same(plain_fn, defaulted_fn));
    let (plain_either, defaulted_either) = (table.deferrable(plain), table.deferrable(defaulted));
    assert!(table.same(plain_either, defaulted_either));
    let plain_union = table.union([plain_list, TyId::INT]);
    let defaulted_union = table.union([defaulted_list, TyId::INT]);
    assert!(table.same(plain_union, defaulted_union));
    // But not where anything else differs.
    let wider = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
    assert!(!table.same(plain, wider));
    let unnamed = table.fn_type([(None, plain)], plain_list, Effect::Pure);
    assert!(!table.same(plain_fn, unnamed));
    let impure = table.fn_type([(Some(c), defaulted)], defaulted_list, Effect::Impure);
    assert!(!table.same(plain_fn, impure));
    assert!(!table.same(plain_list, plain));
    // The join has none, and is not widened; a union keeps, of the two,
    // the one interned first.
    assert_eq!(table.join(defaulted, plain), (plain, false));
    assert_eq!(table.join(defaulted, defaulted), (defaulted, false));
    assert_eq!(table.union([defaulted, plain]), plain);
    let union = table.union([defaulted, TyId::BOOL]);
    assert_eq!(
        table.union_members(union),
        Some(&[TyId::BOOL, defaulted][..])
    );
    let kept = table.union([TyId::BOOL, plain]);
    assert_eq!(table.union([union, plain]), kept);
}

#[test]
fn a_record_types_defaults_are_kept_and_shown() {
    let (mut table, names, [a, b, c]) = setup();
    let one = || Constant::Int(1);
    let defaulted =
        table.record_with_defaults([(a, TyId::INT), (b, TyId::STRING)], vec![(a, one())]);
    let plain = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    let plain_list = table.list(plain);
    // A record type rebuilt from its fields keeps theirs; a field a
    // functional update sets loses its own.
    let lazy_int = table.deferrable(TyId::INT);
    let lazy = table.record_with_defaults([(a, lazy_int), (b, TyId::STRING)], vec![(a, one())]);
    assert_eq!(table.present(lazy), defaulted);
    assert_eq!(
        table
            .with_field(defaulted, c, TyId::BOOL)
            .map(|set| table.defaults(set).len()),
        Some(1)
    );
    let set = table.with_field(defaulted, a, TyId::FLOAT);
    assert_eq!(set.map(|set| table.defaults(set).is_empty()), Some(true));
    // Shown after their fields' types, each as a value's `repr` shows
    // it, a record's fields in the order they are written.
    let every = table.record_with_defaults(
        [(a, defaulted), (b, TyId::FLOAT), (c, TyId::STRING)],
        vec![
            (
                a,
                Constant::Record(Box::new([(b, Constant::Str("x\"y".into())), (a, one())])),
            ),
            (b, Constant::Float(2.0f64.to_bits())),
            (c, Constant::Str("{z}".into())),
        ],
    );
    assert_eq!(
        table.display(every, &names).to_string(),
        "{a: {a: Int = 1, b: String} = {b = \"x\\\"y\"; a = 1}, b: Float = 2.0, \
             c: String = \"{{z}}\"}"
    );
    let lists = table.list(TyId::FLOAT);
    let more = table.record_with_defaults(
        [(a, lists), (b, TyId::FLOAT), (c, TyId::EMPTY)],
        vec![
            (
                a,
                Constant::List(Box::new([
                    Constant::Float(f64::NAN.to_bits()),
                    Constant::Float(f64::NEG_INFINITY.to_bits()),
                ])),
            ),
            (b, Constant::Float((-0.5f64).to_bits())),
            (c, Constant::Empty),
        ],
    );
    assert_eq!(
        table.display(more, &names).to_string(),
        "{a: List<Float> = [nan, -inf], b: Float = -0.5, c: {} = {}}"
    );
    let bools = table.record_with_defaults(
        [(a, TyId::BOOL), (b, plain_list)],
        vec![
            (a, Constant::Bool(false)),
            (b, Constant::List(Box::new([]))),
        ],
    );
    assert_eq!(
        table.display(bools, &names).to_string(),
        "{a: Bool = false, b: List<{a: Int, b: String}> = []}"
    );
}

#[test]
fn interning_is_by_structure() {
    let (mut table, _, [a, b, _]) = setup();
    let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    // The same fields in another order are the same type.
    assert_eq!(table.record([(b, TyId::STRING), (a, TyId::INT)]), ab);
    assert_eq!(
        table.intern(TyKind::Record(RecordTy {
            fields: Box::new([(b, TyId::STRING), (a, TyId::INT)]),
            extras: None,
        })),
        ab
    );
    // A different field type, a different field name, or a missing
    // field is a different type.
    assert_ne!(table.record([(a, TyId::INT), (b, TyId::INT)]), ab);
    assert_ne!(table.record([(a, TyId::INT)]), ab);
    assert_ne!(table.record([(b, TyId::INT), (a, TyId::STRING)]), ab);
    let before = table.len();
    let nested = table.record([(a, ab)]);
    assert_eq!(table.record([(a, ab)]), nested);
    assert_eq!(table.len(), before + 1);
}

#[test]
fn record_fields_are_sorted_by_name() {
    let (mut table, _, [a, b, c]) = setup();
    let id = table.record([(c, TyId::BOOL), (a, TyId::INT), (b, TyId::FLOAT)]);
    assert_eq!(
        table.record_fields(id),
        Some(&[(a, TyId::INT), (b, TyId::FLOAT), (c, TyId::BOOL)][..])
    );
    assert_eq!(table.field(id, b), Some(TyId::FLOAT));
    assert_eq!(table.field(TyId::EMPTY, b), None);
    assert_eq!(table.field(TyId::INT, b), None);
    assert_eq!(table.record_fields(TyId::STRING), None);
    assert_eq!(table.record_fields(TyId::EMPTY), Some(&[][..]));
    assert!(table.is_record(id));
    assert!(table.is_record(TyId::EMPTY));
    assert!(!table.is_record(TyId::INT));
    assert!(table.is_error(TyId::ERROR));
    assert!(!table.is_error(TyId::EMPTY));
}

#[test]
fn setting_a_field_replaces_or_adds_it() {
    let (mut table, _, [a, b, c]) = setup();
    let ac = table.record([(a, TyId::INT), (c, TyId::BOOL)]);
    // Added in its place among the sorted fields, at the start, middle
    // or end.
    let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
    assert_eq!(table.with_field(ac, b, TyId::STRING), Some(abc));
    let bc = table.record([(b, TyId::FLOAT), (c, TyId::BOOL)]);
    let c_only = table.record([(c, TyId::BOOL)]);
    assert_eq!(table.with_field(c_only, b, TyId::FLOAT), Some(bc));
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    assert_eq!(table.with_field(a_only, b, TyId::INT), Some(ab));
    assert_eq!(table.with_field(TyId::EMPTY, a, TyId::INT), Some(a_only));
    // Replaced, the type changing.
    let ac_float = table.record([(a, TyId::INT), (c, TyId::FLOAT)]);
    assert_eq!(table.with_field(ac, c, TyId::FLOAT), Some(ac_float));
    assert_eq!(table.with_field(ac, a, TyId::INT), Some(ac));
    // Nested records are types like any other.
    let nested = table.record([(a, ac)]);
    assert_eq!(table.with_field(TyId::EMPTY, a, ac), Some(nested));
    // Only records have fields to set.
    assert_eq!(table.with_field(TyId::INT, a, TyId::INT), None);
}

#[test]
fn display_every_kind() {
    let mut names = Names::new();
    let (y, x, spaced, keyword, dashed) = (
        names.intern("y"),
        names.intern("x"),
        names.intern("a b"),
        names.intern("let"),
        names.intern("hello-world"),
    );
    let mut table = TypeTable::new();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    assert_eq!(show(&table, TyId::INT), "Int");
    assert_eq!(show(&table, TyId::FLOAT), "Float");
    assert_eq!(show(&table, TyId::BOOL), "Bool");
    assert_eq!(show(&table, TyId::STRING), "String");
    assert_eq!(show(&table, TyId::EMPTY), "{}");
    assert_eq!(show(&table, TyId::UNTYPED_INT), "Int");
    assert_eq!(show(&table, TyId::ERROR), "<error>");
    // Alphabetical by text, although `y` was interned first.
    let xy = table.record([(y, TyId::STRING), (x, TyId::INT)]);
    assert_eq!(show(&table, xy), "{x: Int, y: String}");
    // Names that are not plain identifiers are quoted; keywords are not
    // identifiers.
    let quoted = table.record([(spaced, TyId::INT), (keyword, TyId::BOOL), (dashed, xy)]);
    assert_eq!(
        show(&table, quoted),
        "{\"a b\": Int, hello-world: {x: Int, y: String}, \"let\": Bool}"
    );
    let nested = table.record([(x, TyId::EMPTY)]);
    assert_eq!(show(&table, nested), "{x: {}}");
    let union = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::STRING])));
    assert_eq!(show(&table, union), "Int | String");
}

#[test]
fn subtyping_is_width_and_depth() {
    let (mut table, _, [a, b, c]) = setup();
    let a_int = table.record([(a, TyId::INT)]);
    let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
    // Width: more fields is a subtype of fewer.
    assert!(table.is_subtype(ab, a_int));
    assert!(table.is_subtype(abc, a_int));
    assert!(table.is_subtype(abc, ab));
    assert!(!table.is_subtype(a_int, ab));
    // Reflexive.
    assert!(table.is_subtype(ab, ab));
    // A field of the wrong type, or a missing field, is not.
    let a_float = table.record([(a, TyId::FLOAT)]);
    assert!(!table.is_subtype(a_int, a_float));
    assert!(!table.is_subtype(ab, a_float));
    let c_bool = table.record([(c, TyId::BOOL)]);
    assert!(!table.is_subtype(ab, c_bool));
    assert!(table.is_subtype(abc, c_bool));
    let b_only = table.record([(b, TyId::STRING)]);
    assert!(table.is_subtype(abc, b_only));
    assert!(!table.is_subtype(a_int, b_only));
    // Depth, nested.
    let outer_ab = table.record([(c, ab)]);
    let outer_a = table.record([(c, a_int)]);
    assert!(table.is_subtype(outer_ab, outer_a));
    assert!(!table.is_subtype(outer_a, outer_ab));
    let deeper = table.record([(b, outer_ab), (a, TyId::INT)]);
    let shallower = table.record([(b, outer_a)]);
    assert!(table.is_subtype(deeper, shallower));
    assert!(!table.is_subtype(shallower, deeper));
    // Everything record is a subtype of `{}`; nothing else is.
    for id in [a_int, ab, abc, outer_ab, deeper, TyId::EMPTY] {
        assert!(table.is_subtype(id, TyId::EMPTY));
    }
    assert!(!table.is_subtype(TyId::EMPTY, a_int));
    for id in [TyId::INT, TyId::FLOAT, TyId::BOOL, TyId::STRING] {
        assert!(!table.is_subtype(id, TyId::EMPTY));
        assert!(!table.is_subtype(TyId::EMPTY, id));
        assert!(table.is_subtype(id, id));
    }
    // The scalar types are subtypes only of themselves.
    assert!(!table.is_subtype(TyId::INT, TyId::FLOAT));
    assert!(!table.is_subtype(TyId::FLOAT, TyId::INT));
    assert!(!table.is_subtype(TyId::UNTYPED_INT, TyId::INT));
}

#[test]
fn joins() {
    let (mut table, _, [a, b, c]) = setup();
    // Identical types join to themselves, records or not.
    assert_eq!(table.join(TyId::INT, TyId::INT), (TyId::INT, false));
    assert_eq!(table.join(TyId::EMPTY, TyId::EMPTY), (TyId::EMPTY, false));
    // Different non-record types, or a record and a non-record, join
    // to their union, widened.
    let int_float = table.union([TyId::INT, TyId::FLOAT]);
    assert_eq!(table.join(TyId::INT, TyId::FLOAT), (int_float, true));
    assert_eq!(table.join(TyId::INT, TyId::UNTYPED_INT), (TyId::INT, false));
    let empty_string = table.union([TyId::EMPTY, TyId::STRING]);
    assert_eq!(table.join(TyId::EMPTY, TyId::STRING), (empty_string, true));
    // Shared fields.
    let ab = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    let a_int = table.record([(a, TyId::INT)]);
    assert_eq!(table.join(ab, a_int), (a_int, false));
    assert_eq!(table.join(a_int, ab), (a_int, false));
    let bc = table.record([(b, TyId::STRING), (c, TyId::BOOL)]);
    let b_string = table.record([(b, TyId::STRING)]);
    assert_eq!(table.join(ab, bc), (b_string, false));
    assert_eq!(table.join(bc, ab), (b_string, false));
    // No shared fields: the empty record.
    let c_bool = table.record([(c, TyId::BOOL)]);
    assert_eq!(table.join(a_int, c_bool), (TyId::EMPTY, false));
    assert_eq!(table.join(a_int, TyId::EMPTY), (TyId::EMPTY, false));
    // A shared field whose types do not agree is kept, with their
    // union, widened.
    let a_float = table.record([(a, TyId::FLOAT)]);
    let a_either_number = table.record([(a, int_float)]);
    assert_eq!(table.join(ab, a_float), (a_either_number, true));
    // Recursive.
    let outer_ab = table.record([(c, ab), (a, TyId::BOOL)]);
    let outer_bc = table.record([(c, bc)]);
    let joined = table.record([(c, b_string)]);
    assert_eq!(table.join(outer_ab, outer_bc), (joined, false));
    let outer_int = table.record([(c, TyId::INT)]);
    let ab_or_int = table.union([ab, TyId::INT]);
    let outer_union = table.record([(c, ab_or_int)]);
    assert_eq!(table.join(outer_ab, outer_int), (outer_union, true));
    // A field matched out of step: `b` of the second record lies
    // between fields of the first.
    let ac = table.record([(a, TyId::INT), (c, TyId::BOOL)]);
    let bc2 = table.record([(b, TyId::FLOAT), (c, TyId::BOOL)]);
    assert_eq!(table.join(ac, bc2), (c_bool, false));
    let abc = table.record([(a, TyId::INT), (b, TyId::STRING), (c, TyId::BOOL)]);
    assert_eq!(table.join(abc, bc), (bc, false));
    assert_eq!(table.join(bc, abc), (bc, false));
    assert_eq!(table.join(ac, c_bool), (c_bool, false));
}

#[test]
fn function_types_intern_with_their_names() {
    let (mut table, _, [a, b, _]) = setup();
    let named = table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure);
    let unnamed = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    assert_ne!(named, unnamed);
    assert_eq!(
        table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure),
        named
    );
    // Names, types, arity, return type and purity all distinguish.
    assert_ne!(
        table.fn_type([(Some(b), TyId::INT)], TyId::INT, Effect::Pure),
        named
    );
    assert_ne!(
        table.fn_type([(Some(a), TyId::FLOAT)], TyId::INT, Effect::Pure),
        named
    );
    assert_ne!(table.fn_type([], TyId::INT, Effect::Pure), named);
    assert_ne!(
        table.fn_type([(Some(a), TyId::INT)], TyId::BOOL, Effect::Pure),
        named
    );
    assert_ne!(
        table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Impure),
        named
    );
    let sig = table.fn_sig(named).unwrap();
    assert_eq!(&*sig.params, &[(Some(a), TyId::INT)]);
    assert_eq!(sig.ret, TyId::INT);
    assert_eq!(sig.effect, Effect::Pure);
    assert!(table.fn_sig(TyId::INT).is_none());
    assert!(table.fn_sig(TyId::EMPTY).is_none());
    assert!(!table.is_record(named));
}

#[test]
fn display_function_types() {
    let mut names = Names::new();
    let (name, times, x) = (
        names.intern("name"),
        names.intern("times"),
        names.intern("x"),
    );
    let mut table = TypeTable::new();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    let plain = table.fn_type(
        [(None, TyId::INT), (None, TyId::STRING)],
        TyId::BOOL,
        Effect::Pure,
    );
    assert_eq!(show(&table, plain), "fn(Int, String): Bool");
    let with_names = table.fn_type(
        [(Some(name), TyId::STRING), (Some(times), TyId::INT)],
        TyId::STRING,
        Effect::Impure,
    );
    assert_eq!(
        show(&table, with_names),
        "fn(name: String, times: Int) -> String"
    );
    let memo = table.fn_type([(Some(x), TyId::STRING)], TyId::STRING, Effect::Memo);
    assert_eq!(show(&table, memo), "fn(x: String) ->> String");
    let returns_memo = table.fn_type([(None, memo)], memo, Effect::Memo);
    assert_eq!(
        show(&table, returns_memo),
        "fn(fn(x: String) ->> String) ->> fn(x: String) ->> String"
    );
    let record = table.record([(x, TyId::INT)]);
    let returns_record = table.fn_type([], record, Effect::Pure);
    assert_eq!(show(&table, returns_record), "fn(): {x: Int}");
    let int_to_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    let higher = table.fn_type([(None, int_to_int)], TyId::INT, Effect::Pure);
    assert_eq!(show(&table, higher), "fn(fn(Int): Int): Int");
    let returns_fn = table.fn_type([], int_to_int, Effect::Impure);
    assert_eq!(show(&table, returns_fn), "fn() -> fn(Int): Int");
    let in_record = table.record([(x, int_to_int)]);
    assert_eq!(show(&table, in_record), "{x: fn(Int): Int}");
    let mixed = table.fn_type(
        [(Some(x), TyId::INT), (None, TyId::EMPTY)],
        TyId::EMPTY,
        Effect::Pure,
    );
    assert_eq!(show(&table, mixed), "fn(x: Int, {}): {}");
}

#[test]
fn function_subtyping() {
    let (mut table, _, [a, b, c]) = setup();
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    let f = |table: &mut TypeTable, params: &[TyId], ret, effect| {
        table.fn_type(params.iter().map(|&ty| (None, ty)), ret, effect)
    };
    // Arity must match.
    let unary = f(&mut table, &[TyId::INT], TyId::INT, Effect::Pure);
    let binary = f(&mut table, &[TyId::INT, TyId::INT], TyId::INT, Effect::Pure);
    assert!(!table.is_subtype(unary, binary));
    assert!(!table.is_subtype(binary, unary));
    // Parameters are contravariant: accepting `{a}` is more than
    // accepting `{a, b}`.
    let takes_a = f(&mut table, &[a_only], TyId::INT, Effect::Pure);
    let takes_ab = f(&mut table, &[ab], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(takes_a, takes_ab));
    assert!(!table.is_subtype(takes_ab, takes_a));
    // The return type is covariant.
    let gives_ab = f(&mut table, &[], ab, Effect::Pure);
    let gives_a = f(&mut table, &[], a_only, Effect::Pure);
    assert!(table.is_subtype(gives_ab, gives_a));
    assert!(!table.is_subtype(gives_a, gives_ab));
    // Pure may stand in for impure, not the reverse.
    let impure = f(&mut table, &[TyId::INT], TyId::INT, Effect::Impure);
    assert!(table.is_subtype(unary, impure));
    assert!(!table.is_subtype(impure, unary));
    // Names are ignored.
    let named = table.fn_type([(Some(c), TyId::INT)], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(named, unary));
    assert!(table.is_subtype(unary, named));
    // Different parameter types, or a function and anything else.
    let takes_float = f(&mut table, &[TyId::FLOAT], TyId::INT, Effect::Pure);
    assert!(!table.is_subtype(unary, takes_float));
    assert!(!table.is_subtype(unary, TyId::INT));
    assert!(!table.is_subtype(unary, TyId::EMPTY));
    assert!(!table.is_subtype(TyId::EMPTY, unary));
    // Nested: a function of functions, and a record of functions.
    let takes_takes_a = f(&mut table, &[takes_a], TyId::INT, Effect::Pure);
    let takes_takes_ab = f(&mut table, &[takes_ab], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(takes_takes_ab, takes_takes_a));
    assert!(!table.is_subtype(takes_takes_a, takes_takes_ab));
    let record_pure = table.record([(a, unary), (b, TyId::INT)]);
    let record_impure = table.record([(a, impure)]);
    assert!(table.is_subtype(record_pure, record_impure));
    assert!(!table.is_subtype(record_impure, record_pure));
}

#[test]
fn the_effect_lattice() {
    let (mut table, _, _) = setup();
    let effects = [Effect::Pure, Effect::Memo, Effect::Impure];
    let of = |table: &mut TypeTable, effect| table.fn_type([(None, TyId::INT)], TyId::INT, effect);
    // All nine pairs: the offered effect must be at most the required.
    for offered in effects {
        for required in effects {
            let (sub, sup) = (of(&mut table, offered), of(&mut table, required));
            assert_eq!(
                table.is_subtype(sub, sup),
                offered <= required,
                "{offered:?} against {required:?}"
            );
        }
    }
    assert!(Effect::Pure < Effect::Memo && Effect::Memo < Effect::Impure);
    // Joins take the greater effect, each way round.
    for a in effects {
        for b in effects {
            let (left, right) = (of(&mut table, a), of(&mut table, b));
            let joined = table.join(left, right);
            assert_eq!(joined, (of(&mut table, a.max(b)), false), "{a:?} and {b:?}");
        }
    }
    // A memoised function's bare parameter is present, which the
    // checker decides; with one, it is not a subtype of an impure
    // function type whose caller may pass a future, but is one of a
    // function type that promises a present argument.
    let either = table.deferrable(TyId::INT);
    let memo = table.fn_type([(None, TyId::INT)], either, Effect::Memo);
    let impure_either = table.fn_type([(None, either)], either, Effect::Impure);
    let impure_present = table.fn_type([(None, TyId::INT)], either, Effect::Impure);
    assert!(!table.is_subtype(memo, impure_either));
    assert!(table.is_subtype(memo, impure_present));
    // And a memoised function type never stands in for a pure one.
    let pure = table.fn_type([(None, TyId::INT)], either, Effect::Pure);
    assert!(!table.is_subtype(memo, pure));
    assert!(table.is_subtype(pure, memo));
}

#[test]
fn function_joins() {
    let (mut table, _, [a, b, _]) = setup();
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    let b_only = table.record([(b, TyId::INT)]);
    // Names kept where both sides agree, dropped otherwise; the return
    // types' join; the greater effect, so pure only if both are.
    let left = table.fn_type(
        [(Some(a), TyId::INT), (Some(a), TyId::BOOL)],
        ab,
        Effect::Pure,
    );
    let right = table.fn_type(
        [(Some(a), TyId::INT), (Some(b), TyId::BOOL)],
        a_only,
        Effect::Impure,
    );
    let joined = table.fn_type(
        [(Some(a), TyId::INT), (None, TyId::BOOL)],
        a_only,
        Effect::Impure,
    );
    assert_eq!(table.join(left, right), (joined, false));
    assert_eq!(table.join(right, left), (joined, false));
    let pure_right = table.fn_type(
        [(Some(a), TyId::INT), (None, TyId::BOOL)],
        b_only,
        Effect::Pure,
    );
    let pure_join = table.fn_type(
        [(Some(a), TyId::INT), (None, TyId::BOOL)],
        b_only,
        Effect::Pure,
    );
    assert_eq!(table.join(left, pure_right), (pure_join, false));
    let gives_a = table.fn_type(
        [(Some(a), TyId::INT), (None, TyId::BOOL)],
        a_only,
        Effect::Pure,
    );
    let gives_empty = table.fn_type(
        [(Some(a), TyId::INT), (None, TyId::BOOL)],
        TyId::EMPTY,
        Effect::Pure,
    );
    assert_eq!(table.join(pure_right, gives_a), (gives_empty, false));
    // A parameter is the narrower of the two types, which both
    // functions accept, if one is a subtype of the other.
    let takes_a = table.fn_type([(None, a_only)], TyId::INT, Effect::Pure);
    let takes_ab = table.fn_type([(None, ab)], TyId::INT, Effect::Pure);
    assert_eq!(table.join(takes_a, takes_ab), (takes_ab, false));
    assert_eq!(table.join(takes_ab, takes_a), (takes_ab, false));
    let either = table.deferrable(TyId::INT);
    let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Pure);
    let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    assert_eq!(table.join(takes_either, takes_int), (takes_int, false));
    assert_eq!(table.join(takes_int, takes_either), (takes_int, false));
    // Each parameter on its own.
    let mixed = table.fn_type([(None, either), (None, ab)], TyId::INT, Effect::Pure);
    let other = table.fn_type([(None, TyId::INT), (None, a_only)], TyId::INT, Effect::Pure);
    let narrow = table.fn_type([(None, TyId::INT), (None, ab)], TyId::INT, Effect::Pure);
    assert_eq!(table.join(mixed, other), (narrow, false));
    // Unrelated parameter types join to the union of the two, widened.
    let unrelated = table.fn_type([(None, b_only)], TyId::INT, Effect::Pure);
    let either_taker = table.union([takes_a, unrelated]);
    assert_eq!(table.join(takes_a, unrelated), (either_taker, true));
    let takes_bool = table.fn_type([(None, TyId::BOOL)], TyId::INT, Effect::Pure);
    let int_or_bool_taker = table.union([takes_int, takes_bool]);
    assert_eq!(table.join(takes_int, takes_bool), (int_or_bool_taker, true));
    // So do different arities; return types that do not agree join to
    // their union, widened.
    let nullary = table.fn_type([], TyId::INT, Effect::Pure);
    let unary = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    let arities = table.union([nullary, unary]);
    assert_eq!(table.join(nullary, unary), (arities, true));
    let gives_float = table.fn_type([], TyId::FLOAT, Effect::Pure);
    let int_float = table.union([TyId::INT, TyId::FLOAT]);
    let gives_number = table.fn_type([], int_float, Effect::Pure);
    assert_eq!(table.join(nullary, gives_float), (gives_number, true));
    // A function and anything else join to their union; a function and
    // itself is itself.
    let with_int = table.union([nullary, TyId::INT]);
    assert_eq!(table.join(nullary, TyId::INT), (with_int, true));
    let with_empty = table.union([nullary, TyId::EMPTY]);
    assert_eq!(table.join(nullary, TyId::EMPTY), (with_empty, true));
    assert_eq!(table.join(nullary, nullary), (nullary, false));
    // Through records: a field whose function types do not join is
    // kept, with their union.
    let with_nullary = table.record([(a, nullary), (b, TyId::INT)]);
    let with_unary = table.record([(a, unary), (b, TyId::INT)]);
    let with_either = table.record([(a, arities), (b, TyId::INT)]);
    assert_eq!(table.join(with_nullary, with_unary), (with_either, true));
}

#[test]
fn a_special_built_in_has_an_opaque_type() {
    let mut table = TypeTable::new();
    let mut names = Names::new();
    let field = names.intern("f");
    let len = table.builtin(Builtin::Len);
    assert_eq!(table.builtin(Builtin::Len), len);
    assert_eq!(table.display(len, &names).to_string(), "builtin len");
    assert_eq!(table.value_kind(len), Some(ValueKind::Fn));
    assert!(table.is_builtin(len));
    assert!(table.contains_fn(len));
    assert!(table.is_subtype(len, len));
    assert!(!table.is_subtype(len, TyId::INT));
    // Found wherever a type holds it, and nowhere else.
    let list = table.list(len);
    let record = table.record([(field, list)]);
    assert_eq!(table.special_builtin(record), Some(Builtin::Len));
    assert!(!table.is_builtin(record));
    assert_eq!(table.special_builtin(TyId::INT), None);
    let function = table.fn_type([(None, TyId::INT)], TyId::FLOAT, Effect::Pure);
    assert_eq!(table.special_builtin(function), None);
}

#[test]
fn containing_a_function() {
    let (mut table, _, [a, b, c]) = setup();
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    assert!(table.contains_fn(function));
    for id in [
        TyId::INT,
        TyId::FLOAT,
        TyId::BOOL,
        TyId::STRING,
        TyId::EMPTY,
    ] {
        assert!(!table.contains_fn(id));
    }
    let flat = table.record([(a, TyId::INT)]);
    assert!(!table.contains_fn(flat));
    let direct = table.record([(a, TyId::INT), (b, function)]);
    assert!(table.contains_fn(direct));
    let deep = table.record([(c, direct)]);
    assert!(table.contains_fn(deep));
    let deeper = table.record([(a, deep), (b, flat)]);
    assert!(table.contains_fn(deeper));
    // A function taking or returning records is a function.
    let takes_flat = table.fn_type([(None, flat)], flat, Effect::Pure);
    assert!(table.contains_fn(takes_flat));
    // An id from a bigger table contains nothing.
    assert!(!TypeTable::new().contains_fn(deep));
}

#[test]
fn value_kinds() {
    let (mut table, _, [a, _, _]) = setup();
    assert_eq!(table.value_kind(TyId::INT), Some(ValueKind::Int));
    assert_eq!(table.value_kind(TyId::FLOAT), Some(ValueKind::Float));
    assert_eq!(table.value_kind(TyId::BOOL), Some(ValueKind::Bool));
    assert_eq!(table.value_kind(TyId::STRING), Some(ValueKind::String));
    assert_eq!(table.value_kind(TyId::EMPTY), Some(ValueKind::Record));
    let record = table.record([(a, TyId::INT)]);
    assert_eq!(table.value_kind(record), Some(ValueKind::Record));
    assert_eq!(table.value_kind(TyId::UNTYPED_INT), None);
    assert_eq!(table.value_kind(TyId::ERROR), None);
    let union = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::BOOL])));
    assert_eq!(table.value_kind(union), Some(ValueKind::Mixed));
    let function = table.fn_type([(None, record)], TyId::INT, Effect::Impure);
    assert_eq!(table.value_kind(function), Some(ValueKind::Fn));
    // An id from a bigger table.
    assert_eq!(TypeTable::new().value_kind(record), None);
    // A future, and the either shape, may be futures.
    let future = table.future(TyId::INT);
    assert_eq!(table.value_kind(future), Some(ValueKind::Deferred));
    let either = table.deferrable(record);
    assert_eq!(table.value_kind(either), Some(ValueKind::Deferred));
}

// ----- Futures -----

#[test]
fn futures_and_the_either_shape_intern_normalised() {
    let (mut table, _, [a, _, _]) = setup();
    let future = table.future(TyId::INT);
    assert_eq!(table.future(TyId::INT), future);
    assert_eq!(table.kind(future), &TyKind::Future(TyId::INT));
    let either = table.deferrable(TyId::INT);
    assert_eq!(table.kind(either), &TyKind::Either(TyId::INT));
    // A future of a future, or of an either, is a future of what it
    // yields in the end.
    assert_eq!(table.future(future), future);
    assert_eq!(table.future(either), future);
    // But the fields of a record it yields are untouched.
    let lazy_fields = table.record([(a, future)]);
    let future_record = table.future(lazy_fields);
    assert_eq!(table.kind(future_record), &TyKind::Future(lazy_fields));
    assert_eq!(table.future(future_record), future_record);
    // Whatever it is given, stripped: a future or the either itself.
    assert_eq!(table.deferrable(future), either);
    assert_eq!(table.deferrable(either), either);
    // Interning is by structure.
    assert_eq!(table.intern(TyKind::Either(TyId::INT)), either);
    let record = table.record([(a, TyId::INT)]);
    let record_either = table.deferrable(record);
    assert_ne!(record_either, either);
    assert!(table.is_deferrable(either));
    assert!(table.is_deferrable(record_either));
    assert!(!table.is_deferrable(future));
    assert!(!table.is_deferrable(TyId::INT));
    let foreign = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::BOOL])));
    assert!(!table.is_deferrable(foreign));
    // An error has no either shape.
    assert_eq!(table.deferrable(TyId::ERROR), TyId::ERROR);
}

#[test]
fn strip_present_and_laziness() {
    let (mut table, _, [a, b, c]) = setup();
    let future = table.future(TyId::INT);
    let nested = table.future(future);
    let either = table.deferrable(TyId::INT);
    for id in [TyId::INT, future, nested, either] {
        assert_eq!(table.strip(id), TyId::INT);
    }
    assert_eq!(table.laziness(TyId::INT), Laziness::Present);
    assert_eq!(table.laziness(future), Laziness::Future);
    assert_eq!(table.laziness(nested), Laziness::Future);
    assert_eq!(table.laziness(either), Laziness::Either);
    let foreign = table.intern(TyKind::Union(Box::new([TyId::INT, TyId::BOOL])));
    assert_eq!(table.laziness(foreign), Laziness::Present);
    assert_eq!(table.strip(foreign), foreign);
    // Flat: a record's fields are left alone by `strip`, and presented
    // by `present`.
    let flat = table.record([(a, future), (b, either), (c, TyId::BOOL)]);
    let flat_present = table.record([(a, TyId::INT), (b, TyId::INT), (c, TyId::BOOL)]);
    assert_eq!(table.laziness(flat), Laziness::Present);
    assert_eq!(table.strip(flat), flat);
    assert_eq!(table.present(flat), flat_present);
    // Nested, and through the top.
    let inner = table.record([(a, nested)]);
    let inner_present = table.record([(a, TyId::INT)]);
    let outer = table.record([(b, inner), (c, either)]);
    let outer_present = table.record([(b, inner_present), (c, TyId::INT)]);
    assert_eq!(table.present(outer), outer_present);
    let outer_future = table.future(outer);
    assert_eq!(table.strip(outer_future), outer);
    assert_eq!(table.present(outer_future), outer_present);
    // Through an either of a record.
    let inner_either = table.deferrable(inner);
    let holder = table.record([(a, inner_either)]);
    let holder_present = table.record([(a, inner_present)]);
    assert_eq!(table.present(holder), holder_present);
    // Not into functions.
    let function = table.fn_type([(None, either)], future, Effect::Impure);
    let with_fn = table.record([(a, function), (b, future)]);
    let with_fn_present = table.record([(a, function), (b, TyId::INT)]);
    assert_eq!(table.present(with_fn), with_fn_present);
    assert_eq!(table.present(function), function);
    // A present type presents as itself, interning nothing.
    let before = table.len();
    assert_eq!(table.present(outer_present), outer_present);
    assert_eq!(table.present(TyId::STRING), TyId::STRING);
    assert_eq!(table.len(), before);
}

#[test]
fn what_needs_an_await() {
    let (mut table, _, [a, b, _]) = setup();
    let future = table.future(TyId::INT);
    let nested = table.future(future);
    let either = table.deferrable(TyId::INT);
    // Fits as it is.
    assert_eq!(table.needs_await(TyId::INT, TyId::INT), None);
    assert_eq!(table.needs_await(future, either), None);
    assert_eq!(table.needs_await(nested, either), None);
    assert_eq!(table.needs_await(TyId::INT, either), None);
    // At the top.
    assert_eq!(table.needs_await(future, TyId::INT), Some(AwaitKind::Top));
    assert_eq!(table.needs_await(either, TyId::INT), Some(AwaitKind::Top));
    assert_eq!(table.needs_await(nested, TyId::INT), Some(AwaitKind::Top));
    // Deeply: a record with a future field against a present record.
    let lazy_record = table.record([(a, future), (b, TyId::STRING)]);
    let present_record = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    assert_eq!(
        table.needs_await(lazy_record, present_record),
        Some(AwaitKind::Deep)
    );
    assert_eq!(table.needs_await(lazy_record, lazy_record), None);
    let future_record = table.future(lazy_record);
    assert_eq!(
        table.needs_await(future_record, lazy_record),
        Some(AwaitKind::Top)
    );
    assert_eq!(
        table.needs_await(future_record, present_record),
        Some(AwaitKind::Deep)
    );
    // Against a wider present record, deeply too.
    let a_only = table.record([(a, TyId::INT)]);
    assert_eq!(
        table.needs_await(lazy_record, a_only),
        Some(AwaitKind::Deep)
    );
    // Never: a present value against a future, or a mismatch whatever
    // is awaited.
    assert_eq!(table.needs_await(TyId::INT, future), None);
    assert!(!table.is_subtype(TyId::INT, future));
    assert_eq!(table.needs_await(either, future), None);
    assert_eq!(table.needs_await(future, TyId::FLOAT), None);
    assert_eq!(table.needs_await(lazy_record, TyId::INT), None);
}

#[test]
fn subtyping_with_futures() {
    let (mut table, _, [a, b, _]) = setup();
    let future = table.future(TyId::INT);
    let nested = table.future(future);
    let either = table.deferrable(TyId::INT);
    let float_future = table.future(TyId::FLOAT);
    // `Future<S> <: Future<T>` iff `S <: T`.
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    let future_ab = table.future(ab);
    let future_a = table.future(a_only);
    assert!(table.is_subtype(future_ab, future_a));
    assert!(!table.is_subtype(future_a, future_ab));
    assert!(!table.is_subtype(future, float_future));
    // A future of a future is interned as the future itself.
    assert_eq!(nested, future);
    // A present value satisfies the either shape, and not a future.
    let either_a = table.deferrable(a_only);
    assert!(table.is_subtype(TyId::INT, either));
    assert!(!table.is_subtype(TyId::INT, future));
    assert!(table.is_subtype(ab, either_a));
    // A future of any depth satisfies the either shape of what it
    // yields once awaited fully.
    assert!(table.is_subtype(future, either));
    assert!(table.is_subtype(nested, either));
    assert!(!table.is_subtype(float_future, either));
    assert!(table.is_subtype(future_ab, either_a));
    // One either shape of another.
    let either_ab = table.deferrable(ab);
    assert!(table.is_subtype(either_ab, either_a));
    assert!(!table.is_subtype(either_a, either_ab));
    // Nothing lazy is a subtype of a present type, and the either
    // shape is not a future.
    assert!(!table.is_subtype(future, TyId::INT));
    assert!(!table.is_subtype(either, TyId::INT));
    assert!(!table.is_subtype(either, future));
    assert!(!table.is_subtype(future_ab, ab));
    // Through record fields.
    let with_future = table.record([(a, future)]);
    let with_either = table.record([(a, either)]);
    let with_int = table.record([(a, TyId::INT)]);
    assert!(table.is_subtype(with_future, with_either));
    assert!(table.is_subtype(with_int, with_either));
    assert!(!table.is_subtype(with_either, with_int));
    assert!(!table.is_subtype(with_future, with_int));
    assert!(!table.is_subtype(with_int, with_future));
    // Through function parameters, contravariant, and results,
    // covariant.
    let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Impure);
    let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Impure);
    assert!(table.is_subtype(takes_either, takes_int));
    assert!(!table.is_subtype(takes_int, takes_either));
    let gives_future = table.fn_type([], future, Effect::Impure);
    let gives_either = table.fn_type([], either, Effect::Impure);
    assert!(table.is_subtype(gives_future, gives_either));
    assert!(!table.is_subtype(gives_either, gives_future));
    // The two callback cases: a pure function whose parameter is
    // present fits `fn(Present<Int>) -> Int`, whose parameter is
    // present and result either, and not `fn(Int) -> Int`, whose
    // parameter is the either too; one whose parameter is the either
    // fits both.
    let double = table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure);
    let present_callback = table.fn_type([(None, TyId::INT)], either, Effect::Impure);
    let either_callback = table.fn_type([(None, either)], either, Effect::Impure);
    assert!(table.is_subtype(double, present_callback));
    assert!(!table.is_subtype(double, either_callback));
    let passes = table.fn_type([(Some(a), either)], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(passes, present_callback));
    assert!(table.is_subtype(passes, either_callback));
}

#[test]
fn joins_with_futures() {
    let (mut table, _, [a, b, _]) = setup();
    let future = table.future(TyId::INT);
    let either = table.deferrable(TyId::INT);
    // Two futures: the future of the join.
    assert_eq!(table.join(future, future), (future, false));
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    let future_ab = table.future(ab);
    let future_a = table.future(a_only);
    assert_eq!(table.join(future_ab, future_a), (future_a, false));
    // A present value and a future, or an either and anything: the
    // either of the join, stripped.
    assert_eq!(table.join(TyId::INT, future), (either, false));
    assert_eq!(table.join(future, TyId::INT), (either, false));
    assert_eq!(table.join(either, future), (either, false));
    assert_eq!(table.join(either, TyId::INT), (either, false));
    let nested = table.future(future);
    assert_eq!(table.join(nested, TyId::INT), (either, false));
    let either_a = table.deferrable(a_only);
    assert_eq!(table.join(ab, future_a), (either_a, false));
    // Each widens iff the inner join does: the future, or the either, of
    // the union.
    let float_future = table.future(TyId::FLOAT);
    let int_float = table.union([TyId::INT, TyId::FLOAT]);
    let future_number = table.future(int_float);
    assert_eq!(table.join(future, float_future), (future_number, true));
    let either_number = table.deferrable(int_float);
    assert_eq!(table.join(TyId::FLOAT, future), (either_number, true));
    // Records join field by field, with these rules per field.
    let with_future = table.record([(a, future), (b, TyId::INT)]);
    let with_int = table.record([(a, TyId::INT)]);
    let with_either = table.record([(a, either)]);
    assert_eq!(table.join(with_future, with_int), (with_either, false));
}

#[test]
fn containing_a_future() {
    let (mut table, _, [a, b, _]) = setup();
    let future = table.future(TyId::INT);
    let either = table.deferrable(TyId::INT);
    assert!(table.contains_future(future));
    assert!(!table.contains_future(either));
    assert!(!table.contains_future(TyId::INT));
    let with_future = table.record([(a, future)]);
    let with_either = table.record([(a, either)]);
    assert!(table.contains_future(with_future));
    assert!(!table.contains_future(with_either));
    let deep = table.record([(b, with_future)]);
    assert!(table.contains_future(deep));
    // An either counts through its present member.
    let either_deep = table.deferrable(deep);
    let holder = table.record([(a, either_deep)]);
    assert!(table.contains_future(holder));
    // Functions are not looked into.
    let function = table.fn_type([(None, future)], future, Effect::Pure);
    assert!(!table.contains_future(function));
    // But `contains_fn` looks through futures and unions.
    let future_fn = table.future(function);
    assert!(table.contains_fn(future_fn));
    let fn_record = table.record([(a, future_fn)]);
    assert!(table.contains_fn(fn_record));
    let either_fn = table.deferrable(function);
    assert!(table.contains_fn(either_fn));
}

#[test]
fn display_futures() {
    let mut names = Names::new();
    let (build, step, x) = (
        names.intern("build"),
        names.intern("step"),
        names.intern("x"),
    );
    let mut table = TypeTable::new();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    let future = table.future(TyId::INT);
    assert_eq!(show(&table, future), "Future<Int>");
    let either = table.deferrable(TyId::INT);
    assert_eq!(show(&table, either), "Int | Future<Int>");
    let future_empty = table.future(TyId::EMPTY);
    let record = table.record([(build, future_empty)]);
    assert_eq!(show(&table, record), "{build: Future<{}>}");
    let takes_step = table.fn_type([(Some(step), future_empty)], TyId::EMPTY, Effect::Pure);
    assert_eq!(show(&table, takes_step), "fn(step: Future<{}>): {}");
    // A future of a future is interned, and so shown, as a future.
    let nested = table.future(future);
    assert_eq!(show(&table, nested), "Future<Int>");
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    let future_fn = table.future(function);
    assert_eq!(show(&table, future_fn), "Future<fn(): Int>");
    // The either of a function type parenthesises the function type,
    // whose return type would otherwise take the union; not inside
    // `Future<...>`.
    let either_fn = table.deferrable(function);
    assert_eq!(show(&table, either_fn), "(fn(): Int) | Future<fn(): Int>");
    let impure = table.fn_type([], either, Effect::Impure);
    let either_impure = table.deferrable(impure);
    assert_eq!(
        show(&table, either_impure),
        "(fn() -> Int | Future<Int>) | Future<fn() -> Int | Future<Int>>"
    );
    // A function returning an either shows it unparenthesised.
    let callback = table.fn_type([(None, either)], either, Effect::Impure);
    assert_eq!(
        show(&table, callback),
        "fn(Int | Future<Int>) -> Int | Future<Int>"
    );
    let gives_either_fn = table.fn_type([], either_fn, Effect::Impure);
    assert_eq!(
        show(&table, gives_either_fn),
        "fn() -> (fn(): Int) | Future<fn(): Int>"
    );
    let with_either = table.record([(x, either)]);
    let record_either = table.deferrable(with_either);
    assert_eq!(
        show(&table, record_either),
        "{x: Int | Future<Int>} | Future<{x: Int | Future<Int>}>"
    );
}

// ----- Provisional types -----

const F: FnId = FnId::new(0);
const G: FnId = FnId::new(1);

#[test]
fn provisional_types_intern_by_function_parameter_and_type() {
    let (mut table, _, [a, _, _]) = setup();
    let x = table.provisional(F, 0, TyId::INT);
    assert_eq!(table.provisional(F, 0, TyId::INT), x);
    assert_eq!(
        table.kind(x),
        &TyKind::Provisional {
            function: F,
            param: 0,
            inner: TyId::INT
        }
    );
    // Another parameter, function or type is another type.
    assert_ne!(table.provisional(F, 1, TyId::INT), x);
    assert_ne!(table.provisional(G, 0, TyId::INT), x);
    assert_ne!(table.provisional(F, 0, TyId::FLOAT), x);
    // No values have one: it never survives a successful check.
    assert_eq!(table.value_kind(x), None);
    let record = table.record([(a, TyId::INT)]);
    let r = table.provisional(F, 1, record);
    assert_eq!(table.value_kind(r), None);
}

#[test]
fn strip_present_and_laziness_of_provisional_types() {
    let (mut table, _, [a, b, _]) = setup();
    let int = table.provisional(F, 0, TyId::INT);
    assert_eq!(table.laziness(int), Laziness::Provisional);
    assert_eq!(table.strip(int), TyId::INT);
    assert_eq!(table.present(int), TyId::INT);
    // In a record's fields, stripped by `present` only.
    let holder = table.record([(a, int), (b, TyId::STRING)]);
    assert_eq!(table.laziness(holder), Laziness::Present);
    assert_eq!(table.strip(holder), holder);
    let holder_present = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    assert_eq!(table.present(holder), holder_present);
    // Of a record type, stripped to it.
    let record = table.record([(a, TyId::INT)]);
    let of_record = table.provisional(F, 1, record);
    assert_eq!(table.strip(of_record), record);
    // A future or an either of one is of its present type.
    assert_eq!(table.future(int), table.future(TyId::INT));
    assert_eq!(table.deferrable(int), table.deferrable(TyId::INT));
    // Looked through as a union is by `contains_fn`, `contains_future`
    // and `needs_await`.
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    let with_fn = table.record([(a, function)]);
    let of_fn_record = table.provisional(F, 2, with_fn);
    assert!(table.contains_fn(of_fn_record));
    assert!(!table.contains_fn(int));
    let future = table.future(TyId::INT);
    let with_future = table.record([(a, future)]);
    let of_future_record = table.provisional(F, 3, with_future);
    assert!(table.contains_future(of_future_record));
    assert!(!table.contains_future(int));
    assert_eq!(table.needs_await(int, TyId::INT), Some(AwaitKind::Top));
    let either = table.deferrable(TyId::INT);
    assert_eq!(table.needs_await(int, either), None);
    assert!(table.is_subtype(int, either));
    let present_record = table.record([(a, TyId::INT)]);
    assert_eq!(
        table.needs_await(of_future_record, present_record),
        Some(AwaitKind::Deep)
    );
}

#[test]
fn provisional_types_are_their_either_shape_as_subtypes_and_present_as_supertypes() {
    let (mut table, _, [a, _, _]) = setup();
    let x = table.provisional(F, 0, TyId::INT);
    let y = table.provisional(F, 1, TyId::INT);
    let either = table.deferrable(TyId::INT);
    let future = table.future(TyId::INT);
    // As a subtype, what the either shape is a subtype of.
    assert!(table.is_subtype(x, x));
    assert!(table.is_subtype(x, either));
    assert!(!table.is_subtype(x, TyId::INT));
    assert!(!table.is_subtype(x, future));
    assert!(!table.is_subtype(x, TyId::FLOAT));
    // As a supertype, its present type, unless it is the same one.
    assert!(table.is_subtype(TyId::INT, x));
    assert!(!table.is_subtype(either, x));
    assert!(!table.is_subtype(future, x));
    assert!(!table.is_subtype(y, x));
    // Through records and function types: a parameter is contravariant.
    let holder = table.record([(a, x)]);
    let holds_either = table.record([(a, either)]);
    let holds_int = table.record([(a, TyId::INT)]);
    assert!(table.is_subtype(holder, holds_either));
    assert!(!table.is_subtype(holder, holds_int));
    let takes_x = table.fn_type([(None, x)], TyId::INT, Effect::Pure);
    let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    let takes_either = table.fn_type([(None, either)], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(takes_x, takes_int));
    assert!(!table.is_subtype(takes_x, takes_either));
    let gives_x = table.fn_type([], x, Effect::Pure);
    let gives_either = table.fn_type([], either, Effect::Pure);
    let gives_int = table.fn_type([], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(gives_x, gives_either));
    assert!(!table.is_subtype(gives_x, gives_int));
}

#[test]
fn provisional_types_join_as_their_either_shape() {
    let (mut table, _, [a, _, _]) = setup();
    let x = table.provisional(F, 0, TyId::INT);
    let y = table.provisional(F, 1, TyId::INT);
    let other = table.provisional(G, 0, TyId::INT);
    let either = table.deferrable(TyId::INT);
    let future = table.future(TyId::INT);
    // With itself, itself; with anything else, the either shape of the
    // join.
    assert_eq!(table.join(x, x), (x, false));
    assert_eq!(table.join(x, y), (either, false));
    assert_eq!(table.join(x, other), (either, false));
    assert_eq!(table.join(x, TyId::INT), (either, false));
    assert_eq!(table.join(TyId::INT, x), (either, false));
    assert_eq!(table.join(x, future), (either, false));
    assert_eq!(table.join(x, either), (either, false));
    let int_float = table.union([TyId::INT, TyId::FLOAT]);
    let either_number = table.deferrable(int_float);
    assert_eq!(table.join(x, TyId::FLOAT), (either_number, true));
    // Through records, field by field.
    let holder = table.record([(a, x)]);
    let holds_int = table.record([(a, TyId::INT)]);
    let holds_either = table.record([(a, either)]);
    assert_eq!(table.join(holder, holder), (holder, false));
    assert_eq!(table.join(holder, holds_int), (holds_either, false));
}

#[test]
fn settling_replaces_a_functions_provisional_types_everywhere() {
    let (mut table, _, [a, b, _]) = setup();
    let x = table.provisional(F, 0, TyId::INT);
    let y = table.provisional(F, 1, TyId::STRING);
    let other = table.provisional(G, 0, TyId::INT);
    let int_either = table.deferrable(TyId::INT);
    let string_either = table.deferrable(TyId::STRING);
    // At the top, by the decision for its parameter.
    assert_eq!(table.settle(x, F, &[true, false]), TyId::INT);
    assert_eq!(table.settle(x, F, &[false, false]), int_either);
    assert_eq!(table.settle(y, F, &[true, false]), string_either);
    // Another function's are left as they are, and so is one with no
    // decision.
    assert_eq!(table.settle(other, F, &[true, true]), other);
    assert_eq!(table.settle(y, F, &[true]), y);
    // A type with none is itself, interning nothing.
    let plain = table.record([(a, TyId::INT)]);
    let before = table.len();
    assert_eq!(table.settle(plain, F, &[true]), plain);
    assert_eq!(table.len(), before);
    // Through records, futures, the either shape and function types,
    // parameters and results.
    let holder = table.record([(a, x), (b, other)]);
    let settled = table.record([(a, TyId::INT), (b, other)]);
    assert_eq!(table.settle(holder, F, &[true, true]), settled);
    let future = table.future(holder);
    let settled_future = table.future(settled);
    assert_eq!(table.settle(future, F, &[true, true]), settled_future);
    let either = table.deferrable(holder);
    let settled_either = table.deferrable(settled);
    assert_eq!(table.settle(either, F, &[true, true]), settled_either);
    let function = table.fn_type([(Some(a), x), (Some(b), y)], holder, Effect::Pure);
    let settled_function = {
        let lazy_holder = table.record([(a, int_either), (b, other)]);
        table.fn_type(
            [(Some(a), int_either), (Some(b), TyId::STRING)],
            lazy_holder,
            Effect::Pure,
        )
    };
    assert_eq!(table.settle(function, F, &[false, true]), settled_function);
    // Only `other` is left, until its own function settles it.
    assert!(table.contains_provisional(settled_function, None));
    assert!(!table.contains_provisional(settled_function, Some(F)));
    let fully = table.settle(settled_function, G, &[true]);
    assert!(!table.contains_provisional(fully, None));
}

#[test]
fn finding_provisional_types() {
    let (mut table, _, [a, b, c]) = setup();
    let first = table.provisional(F, 0, TyId::INT);
    let second = table.provisional(F, 1, TyId::INT);
    let other = table.provisional(G, 0, TyId::INT);
    let either = table.deferrable(TyId::INT);
    let future = table.future(TyId::INT);
    // Anywhere, through every kind of type.
    for id in [first, other] {
        assert!(table.contains_provisional(id, None));
    }
    assert!(table.contains_provisional(first, Some(F)));
    assert!(!table.contains_provisional(other, Some(F)));
    assert!(!table.contains_provisional(TyId::INT, None));
    let holder = table.record([(a, first)]);
    let in_future = table.future(holder);
    let in_either = table.deferrable(holder);
    let in_param = table.fn_type([(None, first)], TyId::INT, Effect::Pure);
    let in_result = table.fn_type([], holder, Effect::Pure);
    for id in [holder, in_future, in_either, in_param, in_result] {
        assert!(table.contains_provisional(id, Some(F)), "{id:?}");
        assert!(!table.contains_provisional(id, Some(G)), "{id:?}");
    }
    // Their parameters, anywhere.
    let both = table.record([(a, first), (b, in_param), (c, other)]);
    let mut found = Vec::new();
    table.provisional_params(both, F, &mut found);
    found.sort_unstable();
    assert_eq!(found, vec![0, 0]);
    // Those at positions where a type expected is present: not where it
    // is the either shape or a future, nor in a function's parameters,
    // nor in a field it lacks.
    let at_present = |table: &TypeTable, found: TyId, expected: TyId| {
        let mut out = Vec::new();
        table.provisionals_at_present(found, expected, F, &mut out);
        out.sort_unstable();
        out
    };
    assert_eq!(at_present(&table, first, TyId::INT), vec![0]);
    assert_eq!(at_present(&table, first, second), vec![0]);
    assert_eq!(at_present(&table, first, either), Vec::<u32>::new());
    assert_eq!(at_present(&table, first, future), Vec::<u32>::new());
    assert_eq!(at_present(&table, other, TyId::INT), Vec::<u32>::new());
    let pair = table.record([(a, first), (b, second)]);
    let wants_a = table.record([(a, TyId::INT), (b, either)]);
    assert_eq!(at_present(&table, pair, wants_a), vec![0]);
    let wants_both = table.record([(a, TyId::INT), (b, TyId::INT)]);
    assert_eq!(at_present(&table, pair, wants_both), vec![0, 1]);
    let wants_b_only = table.record([(b, TyId::INT)]);
    assert_eq!(at_present(&table, pair, wants_b_only), vec![1]);
    // Through the either shape and futures of records.
    let either_wants_both = table.deferrable(wants_both);
    assert_eq!(at_present(&table, pair, either_wants_both), vec![0, 1]);
    let future_pair = table.future(pair);
    assert_eq!(at_present(&table, future_pair, wants_both), vec![0, 1]);
    let future_wants_both = table.future(wants_both);
    assert_eq!(
        at_present(&table, pair, future_wants_both),
        Vec::<u32>::new()
    );
    // A function's result, not its parameters.
    let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    assert_eq!(at_present(&table, in_param, takes_int), Vec::<u32>::new());
    let gives_first = table.fn_type([], first, Effect::Pure);
    let gives_int = table.fn_type([], TyId::INT, Effect::Pure);
    assert_eq!(at_present(&table, gives_first, gives_int), vec![0]);
}

#[test]
fn display_provisional_types() {
    let mut names = Names::new();
    let x = names.intern("x");
    let mut table = TypeTable::new();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    let int = table.provisional(F, 0, TyId::INT);
    assert_eq!(show(&table, int), "Int?");
    let record = table.record([(x, TyId::INT)]);
    let r = table.provisional(F, 1, record);
    assert_eq!(show(&table, r), "{x: Int}?");
    let holder = table.record([(x, int)]);
    assert_eq!(show(&table, holder), "{x: Int?}");
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    let f = table.provisional(F, 2, function);
    assert_eq!(show(&table, f), "(fn(): Int)?");
    let takes = table.fn_type([(Some(x), int)], int, Effect::Pure);
    assert_eq!(show(&table, takes), "fn(x: Int?): Int?");
}

// ----- Lists -----

#[test]
fn list_types_intern_by_element_type() {
    let (mut table, _, [a, ..]) = setup();
    let ints = table.list(TyId::INT);
    assert_eq!(table.list(TyId::INT), ints);
    assert_ne!(table.list(TyId::FLOAT), ints);
    assert_eq!(table.list_element(ints), Some(TyId::INT));
    assert_eq!(table.list_element(TyId::INT), None);
    let nested = table.list(ints);
    assert_eq!(table.list_element(nested), Some(ints));
    let record = table.record([(a, TyId::INT)]);
    let records = table.list(record);
    assert_eq!(table.record([(a, TyId::INT)]), record);
    assert_eq!(table.list(record), records);
    assert_eq!(table.value_kind(ints), Some(ValueKind::List));
    let either = table.deferrable(ints);
    assert_eq!(table.value_kind(either), Some(ValueKind::Deferred));
    assert_eq!(table.laziness(ints), Laziness::Present);
    assert!(!table.contains_fn(ints));
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    let functions = table.list(function);
    assert!(table.contains_fn(functions));
    let in_record = table.record([(a, functions)]);
    assert!(table.contains_fn(in_record));
}

#[test]
fn display_list_types() {
    let (mut table, names, [a, ..]) = setup();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    let ints = table.list(TyId::INT);
    assert_eq!(show(&table, ints), "List<Int>");
    let nested = table.list(ints);
    assert_eq!(show(&table, nested), "List<List<Int>>");
    let either = table.deferrable(TyId::INT);
    let eithers = table.list(either);
    assert_eq!(show(&table, eithers), "List<Int | Future<Int>>");
    let outer = table.deferrable(eithers);
    assert_eq!(
        show(&table, outer),
        "List<Int | Future<Int>> | Future<List<Int | Future<Int>>>"
    );
    let record = table.record([(a, ints)]);
    let records = table.list(record);
    assert_eq!(show(&table, records), "List<{a: List<Int>}>");
    let function = table.fn_type([], TyId::INT, Effect::Impure);
    let functions = table.list(function);
    assert_eq!(show(&table, functions), "List<fn() -> Int>");
    let provisional = table.provisional(F, 0, ints);
    assert_eq!(show(&table, provisional), "List<Int>?");
}

#[test]
fn list_subtyping_is_covariant() {
    let (mut table, _, [a, b, _]) = setup();
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let just_a = table.record([(a, TyId::INT)]);
    let abs = table.list(ab);
    let just_as = table.list(just_a);
    assert!(table.is_subtype(abs, just_as));
    assert!(!table.is_subtype(just_as, abs));
    let ints = table.list(TyId::INT);
    let floats = table.list(TyId::FLOAT);
    assert!(!table.is_subtype(ints, floats));
    assert!(!table.is_subtype(ints, TyId::EMPTY));
    assert!(!table.is_subtype(TyId::EMPTY, ints));
    // With futures inside: a present element satisfies the either
    // shape, and nothing but a future a future.
    let either = table.deferrable(TyId::INT);
    let future = table.future(TyId::INT);
    let eithers = table.list(either);
    let futures = table.list(future);
    assert!(table.is_subtype(ints, eithers));
    assert!(table.is_subtype(futures, eithers));
    assert!(!table.is_subtype(eithers, ints));
    assert!(!table.is_subtype(ints, futures));
    // A list, or its future, satisfies the either shape of a list type.
    let outer = table.deferrable(eithers);
    let future_ints = table.future(ints);
    assert!(table.is_subtype(ints, outer));
    assert!(table.is_subtype(future_ints, outer));
    assert!(!table.is_subtype(outer, eithers));
    let nested = table.list(ints);
    let nested_eithers = table.list(eithers);
    assert!(table.is_subtype(nested, nested_eithers));
}

#[test]
fn list_joins() {
    let (mut table, _, [a, b, _]) = setup();
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_string = table.record([(a, TyId::INT), (b, TyId::STRING)]);
    let just_a = table.record([(a, TyId::INT)]);
    let abs = table.list(ab);
    let a_strings = table.list(a_string);
    let just_as = table.list(just_a);
    assert_eq!(table.join(abs, just_as), (just_as, false));
    let int_string = table.union([TyId::INT, TyId::STRING]);
    let a_either = table.record([(a, TyId::INT), (b, int_string)]);
    let a_eithers = table.list(a_either);
    assert_eq!(table.join(abs, a_strings), (a_eithers, true));
    let ints = table.list(TyId::INT);
    let floats = table.list(TyId::FLOAT);
    let int_float = table.union([TyId::INT, TyId::FLOAT]);
    let numbers = table.list(int_float);
    assert_eq!(table.join(ints, floats), (numbers, true));
    let ints_or_int = table.union([ints, TyId::INT]);
    assert_eq!(table.join(ints, TyId::INT), (ints_or_int, true));
    let ints_or_empty = table.union([ints, TyId::EMPTY]);
    assert_eq!(table.join(ints, TyId::EMPTY), (ints_or_empty, true));
    let future = table.future(TyId::INT);
    let futures = table.list(future);
    let either = table.deferrable(TyId::INT);
    let eithers = table.list(either);
    assert_eq!(table.join(ints, futures), (eithers, false));
    // A list and a future of one join to the either shape.
    let future_ints = table.future(ints);
    let outer = table.deferrable(ints);
    assert_eq!(table.join(ints, future_ints), (outer, false));
}

#[test]
fn a_deep_await_toward_a_type_leaves_what_it_admits_lazy() {
    let (mut table, _, [a, b, c]) = setup();
    let either = table.deferrable(TyId::INT);
    let future = table.future(TyId::INT);
    // A field the expected type admits lazy is left as it is; one it needs
    // present is presented, within a list too.
    let found = table.record([(a, future), (b, future)]);
    let expected = table.record([(a, either), (b, TyId::INT)]);
    assert_eq!(table.needs_await(found, expected), Some(AwaitKind::Deep));
    let toward = table.record([(a, future), (b, TyId::INT)]);
    assert_eq!(table.present_toward(found, expected), toward);
    let found_list = table.list(found);
    let expected_list = table.list(expected);
    let toward_list = table.list(toward);
    assert_eq!(table.present_toward(found_list, expected_list), toward_list);
    // Within a future at the top, which is awaited whole.
    let future_record = table.future(found);
    assert_eq!(table.present_toward(future_record, expected), toward);
    // What fits as it is is left whole; what the expected type does not
    // line up with field by field, here through a union, is presented
    // whole.
    assert_eq!(table.present_toward(toward, expected), toward);
    let other = table.record([(c, TyId::STRING)]);
    let union = table.union(vec![expected, other]);
    let present = table.present(found);
    assert_eq!(table.present_toward(found, union), present);
}

#[test]
fn strip_present_and_futures_through_lists() {
    let (mut table, _, [a, ..]) = setup();
    let either = table.deferrable(TyId::INT);
    let future = table.future(TyId::INT);
    let eithers = table.list(either);
    let futures = table.list(future);
    let ints = table.list(TyId::INT);
    // A list is present: stripping leaves its element type, and
    // presenting makes it present too.
    let outer = table.deferrable(eithers);
    assert_eq!(table.strip(outer), eithers);
    assert_eq!(table.present(outer), ints);
    assert_eq!(table.present(futures), ints);
    assert_eq!(table.present(ints), ints);
    let nested = table.list(eithers);
    let nested_ints = table.list(ints);
    assert_eq!(table.present(nested), nested_ints);
    let record = table.record([(a, eithers)]);
    let present_record = table.record([(a, ints)]);
    let records = table.list(record);
    let present_records = table.list(present_record);
    assert_eq!(table.present(records), present_records);
    // Awaits: at the top for a future of a list with present elements,
    // deeply for one with lazy elements.
    let future_ints = table.future(ints);
    assert_eq!(table.needs_await(future_ints, ints), Some(AwaitKind::Top));
    assert_eq!(table.needs_await(eithers, ints), Some(AwaitKind::Deep));
    assert_eq!(table.needs_await(outer, ints), Some(AwaitKind::Deep));
    assert_eq!(table.needs_await(ints, eithers), None);
    assert_eq!(table.needs_await(futures, TyId::INT), None);
    // A `Future` element is a future inside; the either shape is not.
    assert!(table.contains_future(futures));
    assert!(!table.contains_future(eithers));
    let nested_futures = table.list(futures);
    assert!(table.contains_future(nested_futures));
    let holder = table.record([(a, futures)]);
    assert!(table.contains_future(holder));
    assert!(!table.contains_future(ints));
}

#[test]
fn provisional_types_through_lists() {
    let (mut table, _, _) = setup();
    let first = table.provisional(F, 0, TyId::INT);
    let firsts = table.list(first);
    assert!(table.contains_provisional(firsts, Some(F)));
    assert!(!table.contains_provisional(firsts, Some(G)));
    let mut found = Vec::new();
    table.provisional_params(firsts, F, &mut found);
    assert_eq!(found, vec![0]);
    // Settled, element and all.
    let ints = table.list(TyId::INT);
    let either = table.deferrable(TyId::INT);
    let eithers = table.list(either);
    assert_eq!(table.settle(firsts, F, &[true]), ints);
    assert_eq!(table.settle(firsts, F, &[false]), eithers);
    // Stripped and presented as the either shape it may become.
    assert_eq!(table.present(firsts), ints);
    // Those where a present element is expected.
    let at_present = |table: &TypeTable, found: TyId, expected: TyId| {
        let mut out = Vec::new();
        table.provisionals_at_present(found, expected, F, &mut out);
        out
    };
    assert_eq!(at_present(&table, firsts, ints), vec![0]);
    assert_eq!(at_present(&table, firsts, eithers), Vec::<u32>::new());
    let outer = table.deferrable(ints);
    assert_eq!(at_present(&table, firsts, outer), vec![0]);
    let future_ints = table.future(ints);
    assert_eq!(at_present(&table, firsts, future_ints), Vec::<u32>::new());
    // A provisional list is present as a supertype, its either shape as
    // a subtype.
    let list_param = table.provisional(F, 1, ints);
    assert!(table.is_subtype(ints, list_param));
    assert!(!table.is_subtype(list_param, ints));
    let outer_eithers = table.deferrable(ints);
    assert!(table.is_subtype(list_param, outer_eithers));
}

// ----- Unions -----

#[test]
fn unions_intern_normalised() {
    let (mut table, _, [a, b, _]) = setup();
    let int_string = table.union([TyId::INT, TyId::STRING]);
    assert_eq!(
        table.kind(int_string),
        &TyKind::Union(Box::new([TyId::INT, TyId::STRING]))
    );
    // Any order, duplicates and nested unions intern to the same union.
    assert_eq!(table.union([TyId::STRING, TyId::INT]), int_string);
    assert_eq!(
        table.union([TyId::STRING, TyId::INT, TyId::STRING]),
        int_string
    );
    let three = table.union([int_string, TyId::BOOL]);
    assert_eq!(table.union([TyId::BOOL, TyId::STRING, TyId::INT]), three);
    assert_eq!(
        table.union_members(three),
        Some(&[TyId::INT, TyId::BOOL, TyId::STRING][..])
    );
    // One member is that member; a subtype of another member is dropped.
    assert_eq!(table.union([TyId::INT, TyId::INT]), TyId::INT);
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    assert_eq!(table.union([ab, a_only]), a_only);
    let with_int = table.union([ab, TyId::INT, a_only]);
    assert_eq!(
        table.union_members(with_int),
        Some(&[TyId::INT, a_only][..])
    );
    // Of two function types each a subtype of the other, the one
    // interned first stays.
    let named = table.fn_type([(Some(a), TyId::INT)], TyId::INT, Effect::Pure);
    let unnamed = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    assert_eq!(table.union([unnamed, named]), named);
    // An error, or nothing, is an error.
    assert_eq!(table.union([TyId::INT, TyId::ERROR]), TyId::ERROR);
    assert_eq!(table.union([]), TyId::ERROR);
    assert_eq!(table.union_members(TyId::INT), None);
}

#[test]
fn the_either_and_the_future_of_a_union() {
    let mut table = TypeTable::new();
    let int_string = table.union([TyId::INT, TyId::STRING]);
    let either = table.deferrable(int_string);
    assert_eq!(table.kind(either), &TyKind::Either(int_string));
    assert_eq!(table.laziness(either), Laziness::Either);
    assert_eq!(table.strip(either), int_string);
    let future = table.future(int_string);
    assert_eq!(table.kind(future), &TyKind::Future(int_string));
    assert_eq!(table.future(either), future);
    assert_eq!(table.deferrable(future), either);
    // A union is present.
    assert_eq!(table.laziness(int_string), Laziness::Present);
    assert_eq!(table.strip(int_string), int_string);
    assert_eq!(table.value_kind(either), Some(ValueKind::Deferred));
}

#[test]
fn subtyping_with_unions() {
    let (mut table, _, [a, b, _]) = setup();
    let int_string = table.union([TyId::INT, TyId::STRING]);
    let three = table.union([TyId::INT, TyId::STRING, TyId::BOOL]);
    // A member, or a subtype of one, is a subtype of the union.
    assert!(table.is_subtype(TyId::INT, int_string));
    assert!(!table.is_subtype(TyId::FLOAT, int_string));
    // A union is a subtype of a type if each of its members is.
    assert!(table.is_subtype(int_string, three));
    assert!(!table.is_subtype(three, int_string));
    assert!(!table.is_subtype(int_string, TyId::INT));
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_only = table.record([(a, TyId::INT)]);
    let b_only = table.record([(b, TyId::INT)]);
    let records = table.union([a_only, b_only]);
    assert!(table.is_subtype(ab, records));
    assert!(table.is_subtype(records, TyId::EMPTY));
    // Through records, lists, functions and futures.
    let holds = table.record([(a, int_string)]);
    let holds_int = table.record([(a, TyId::INT)]);
    assert!(table.is_subtype(holds_int, holds));
    assert!(!table.is_subtype(holds, holds_int));
    let ints = table.list(TyId::INT);
    let either_ints = table.list(int_string);
    assert!(table.is_subtype(ints, either_ints));
    let strings = table.list(TyId::STRING);
    let lists = table.union([ints, strings]);
    assert!(!table.is_subtype(either_ints, lists));
    assert!(table.is_subtype(lists, either_ints));
    let takes_union = table.fn_type([(None, int_string)], TyId::INT, Effect::Pure);
    let takes_int = table.fn_type([(None, TyId::INT)], TyId::INT, Effect::Pure);
    assert!(table.is_subtype(takes_union, takes_int));
    assert!(!table.is_subtype(takes_int, takes_union));
    let future = table.future(TyId::INT);
    let future_union = table.future(int_string);
    assert!(table.is_subtype(future, future_union));
    // The either of a union takes a member, a future of one, and the
    // either of a narrower union; nothing lazy is a subtype of a union.
    let either = table.deferrable(int_string);
    assert!(table.is_subtype(TyId::STRING, either));
    assert!(table.is_subtype(future, either));
    assert!(table.is_subtype(int_string, either));
    assert!(!table.is_subtype(future, int_string));
    assert!(!table.is_subtype(either, int_string));
}

#[test]
fn total_joins_report_widening() {
    let (mut table, _, [a, _, _]) = setup();
    let int_string = table.union([TyId::INT, TyId::STRING]);
    // At the top.
    assert_eq!(table.join(TyId::INT, TyId::STRING), (int_string, true));
    let a_int = table.record([(a, TyId::INT)]);
    let ints = table.list(TyId::INT);
    let record_or_list = table.union([a_int, ints]);
    assert_eq!(table.join(a_int, ints), (record_or_list, true));
    // A union with a type it is a supertype of is itself, not widened;
    // with any other, the union of both sides' members, widened.
    assert_eq!(table.join(int_string, TyId::INT), (int_string, false));
    assert_eq!(table.join(TyId::STRING, int_string), (int_string, false));
    let three = table.union([TyId::INT, TyId::STRING, TyId::BOOL]);
    assert_eq!(table.join(int_string, TyId::BOOL), (three, true));
    assert_eq!(table.join(three, int_string), (three, false));
    // Inside: list elements and record fields.
    let strings = table.list(TyId::STRING);
    let either_list = table.list(int_string);
    assert_eq!(table.join(ints, strings), (either_list, true));
    let a_string = table.record([(a, TyId::STRING)]);
    let a_union = table.record([(a, int_string)]);
    assert_eq!(table.join(a_int, a_string), (a_union, true));
    // Futures and either shapes, on what they yield.
    let future_int = table.future(TyId::INT);
    let future_string = table.future(TyId::STRING);
    let future_union = table.future(int_string);
    assert_eq!(table.join(future_int, future_string), (future_union, true));
    let either_union = table.deferrable(int_string);
    assert_eq!(table.join(future_int, TyId::STRING), (either_union, true));
    assert_eq!(table.join(either_union, TyId::INT), (either_union, false));
    // An error joins to an error, unwidened.
    assert_eq!(table.join(TyId::ERROR, TyId::INT), (TyId::ERROR, false));
}

#[test]
fn strip_present_laziness_and_futures_through_unions() {
    let (mut table, _, [a, _, _]) = setup();
    let future = table.future(TyId::INT);
    let either = table.deferrable(TyId::INT);
    let lazy_record = table.record([(a, either)]);
    let futures = table.list(future);
    let union = table.union([lazy_record, futures]);
    assert_eq!(table.laziness(union), Laziness::Present);
    assert_eq!(table.strip(union), union);
    let present_record = table.record([(a, TyId::INT)]);
    let ints = table.list(TyId::INT);
    let presented = table.union([present_record, ints]);
    assert_eq!(table.present(union), presented);
    assert!(table.contains_future(union));
    let quiet = table.union([lazy_record, TyId::INT]);
    assert!(!table.contains_future(quiet));
    assert_eq!(table.needs_await(union, presented), Some(AwaitKind::Deep));
    // Presenting may make one member a subtype of another.
    let both = table.union([lazy_record, present_record]);
    assert_eq!(table.present(both), present_record);
    // Functions inside.
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    let with_fn = table.union([function, TyId::INT]);
    assert!(table.contains_fn(with_fn));
    assert!(!table.contains_fn(quiet));
}

#[test]
fn provisional_types_through_unions() {
    let (mut table, _, [a, _, _]) = setup();
    let int_string = table.union([TyId::INT, TyId::STRING]);
    let x = table.provisional(F, 0, int_string);
    assert_eq!(table.strip(x), int_string);
    let holder = table.record([(a, x)]);
    let with_holder = table.union([holder, TyId::BOOL]);
    assert!(table.contains_provisional(with_holder, Some(F)));
    let mut found = Vec::new();
    table.provisional_params(with_holder, F, &mut found);
    assert_eq!(found, vec![0]);
    let present_holder = table.record([(a, int_string)]);
    let settled = table.union([present_holder, TyId::BOOL]);
    assert_eq!(table.settle(with_holder, F, &[true]), settled);
    // A provisional type where a member of an expected union is present
    // is one to measure, and not where it is the either shape.
    let expected = table.union([present_holder, TyId::STRING]);
    let mut out = Vec::new();
    table.provisionals_at_present(holder, expected, F, &mut out);
    assert_eq!(out, vec![0]);
    let either = table.deferrable(int_string);
    let lazy_holder = table.record([(a, either)]);
    let lazy_expected = table.union([lazy_holder, TyId::STRING]);
    let mut out = Vec::new();
    table.provisionals_at_present(holder, lazy_expected, F, &mut out);
    assert!(out.is_empty());
}

#[test]
fn display_unions() {
    let mut names = Names::new();
    let (b, a) = (names.intern("b"), names.intern("a"));
    let mut table = TypeTable::new();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    // By kind, in the order of `TyKind`, then by text.
    let a_int = table.record([(a, TyId::INT)]);
    let b_int = table.record([(b, TyId::INT)]);
    let ints = table.list(TyId::INT);
    let function = table.fn_type([], TyId::INT, Effect::Pure);
    let union = table.union([function, ints, b_int, a_int, TyId::STRING, TyId::FLOAT]);
    assert_eq!(
        show(&table, union),
        "Float | String | {a: Int} | {b: Int} | List<Int> | (fn(): Int)"
    );
    // Inside a list, a parameter and a field it needs no parentheses;
    // a function type's return type takes it.
    let int_string = table.union([TyId::STRING, TyId::INT]);
    assert_eq!(show(&table, int_string), "Int | String");
    let list = table.list(int_string);
    assert_eq!(show(&table, list), "List<Int | String>");
    let takes = table.fn_type([(Some(a), int_string)], int_string, Effect::Impure);
    assert_eq!(show(&table, takes), "fn(a: Int | String) -> Int | String");
    let record = table.record([(b, int_string)]);
    assert_eq!(show(&table, record), "{b: Int | String}");
    // The either of a union, and a provisional one.
    let either = table.deferrable(int_string);
    assert_eq!(show(&table, either), "Int | String | Future<Int | String>");
    let provisional = table.provisional(F, 0, int_string);
    assert_eq!(show(&table, provisional), "(Int | String)?");
    let with_fn = table.union([function, TyId::INT]);
    let either_fn = table.deferrable(with_fn);
    assert_eq!(
        show(&table, either_fn),
        "Int | (fn(): Int) | Future<Int | (fn(): Int)>"
    );
}

#[test]
fn value_kinds_of_unions() {
    let (mut table, _, [a, b, _]) = setup();
    let int_string = table.union([TyId::INT, TyId::STRING]);
    assert_eq!(table.value_kind(int_string), Some(ValueKind::Mixed));
    let ints = table.list(TyId::INT);
    let strings = table.list(TyId::STRING);
    let lists = table.union([ints, strings]);
    assert_eq!(table.value_kind(lists), Some(ValueKind::List));
    let a_int = table.record([(a, TyId::INT)]);
    let b_int = table.record([(b, TyId::INT)]);
    let records = table.union([a_int, b_int]);
    assert_eq!(table.value_kind(records), Some(ValueKind::Record));
}

#[test]
fn overlapping_types() {
    let (mut table, _, [a, b, _]) = setup();
    assert!(table.overlaps(TyId::INT, TyId::INT));
    assert!(!table.overlaps(TyId::INT, TyId::STRING));
    // Records overlap unless a field both have cannot.
    let a_int = table.record([(a, TyId::INT)]);
    let a_string = table.record([(a, TyId::STRING)]);
    let b_int = table.record([(b, TyId::INT)]);
    assert!(!table.overlaps(a_int, a_string));
    assert!(table.overlaps(a_int, b_int));
    let int_string = table.union([TyId::INT, TyId::STRING]);
    let a_union = table.record([(a, int_string), (b, TyId::INT)]);
    assert!(table.overlaps(a_union, a_int));
    // Lists by their elements; unions by any member.
    let ints = table.list(TyId::INT);
    let strings = table.list(TyId::STRING);
    assert!(!table.overlaps(ints, strings));
    assert!(table.overlaps(int_string, TyId::STRING));
    assert!(!table.overlaps(int_string, TyId::BOOL));
    assert!(!table.overlaps(a_int, ints));
}

#[test]
fn test_plans_look_only_where_in_doubt() {
    let (mut table, _, [a, b, _]) = setup();
    let ints = table.list(TyId::INT);
    let int_string = table.union([TyId::INT, TyId::STRING]);
    // The kind alone, where every value of that kind passes.
    assert_eq!(
        table.test_plan(Some(&[TyId::INT, ints]), ints),
        ValueTest::Kind(ValueKind::List)
    );
    assert_eq!(
        table.test_plan(Some(&[ints]), TyId::INT),
        ValueTest::Kind(ValueKind::Int)
    );
    assert_eq!(
        table.test_plan(Some(&[TyId::INT]), TyId::INT),
        ValueTest::Pass
    );
    // Into the elements, where a list's elements are in doubt.
    let mixed = table.list(int_string);
    assert_eq!(
        table.test_plan(Some(&[mixed]), ints),
        ValueTest::List(Box::new(ValueTest::Kind(ValueKind::Int)))
    );
    // Into the fields in doubt only.
    let ab = table.record([(a, int_string), (b, TyId::INT)]);
    let wants = table.record([(a, TyId::INT), (b, TyId::INT)]);
    assert_eq!(
        table.test_plan(Some(&[ab]), wants),
        ValueTest::Record(Box::new([(a, ValueTest::Kind(ValueKind::Int))]))
    );
    // A field some record type lacks is tested whole.
    let b_only = table.record([(b, TyId::INT)]);
    let a_list = table.record([(a, ints)]);
    assert_eq!(
        table.test_plan(Some(&[b_only]), a_list),
        ValueTest::Record(Box::new([(
            a,
            ValueTest::List(Box::new(ValueTest::Kind(ValueKind::Int)))
        )]))
    );
    // A union is any of its members' tests.
    let strings = table.list(TyId::STRING);
    let value = [TyId::INT, TyId::BOOL, strings];
    let target = table.union([TyId::INT, strings]);
    assert_eq!(
        table.test_plan(Some(&value), target),
        ValueTest::Any(Box::new([
            ValueTest::Kind(ValueKind::Int),
            ValueTest::Kind(ValueKind::List)
        ]))
    );
    assert!(table.test_plan(Some(&[mixed]), ints).walks());
    assert!(!table.test_plan(Some(&value), target).walks());
    // `{}` is every record's kind.
    assert_eq!(
        table.test_plan(None, TyId::EMPTY),
        ValueTest::Kind(ValueKind::Record)
    );
}

/// The function type of no parameters returning `Int`, of effect
/// `effect`, with the meta `meta`, if any.
fn with_meta(table: &mut TypeTable, effect: Effect, meta: Option<(Effect, TyId)>) -> TyId {
    let meta = meta.map(|(effect, ty)| Meta { effect, ty });
    let none: [(Option<Name>, TyId); 0] = [];
    table.fn_type_with_meta(none, TyId::INT, effect, meta)
}

#[test]
fn a_meta_is_part_of_its_function_type() {
    let (mut table, _, [a, _, _]) = setup();
    let rec = table.record([(a, TyId::INT)]);
    let plain = with_meta(&mut table, Effect::Pure, None);
    let pure = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, rec)));
    assert_ne!(plain, pure);
    assert_eq!(
        with_meta(&mut table, Effect::Pure, Some((Effect::Pure, rec))),
        pure
    );
    // The marker and the type distinguish.
    assert_ne!(
        with_meta(&mut table, Effect::Pure, Some((Effect::Impure, rec))),
        pure
    );
    assert_ne!(
        with_meta(&mut table, Effect::Pure, Some((Effect::Pure, TyId::EMPTY))),
        pure
    );
    assert_eq!(
        table.fn_sig(pure).and_then(|sig| sig.meta),
        Some(Meta {
            effect: Effect::Pure,
            ty: rec
        })
    );
    assert_eq!(table.fn_sig(plain).and_then(|sig| sig.meta), None);
}

#[test]
fn a_meta_is_purely_additive() {
    let (mut table, _, [a, b, _]) = setup();
    let a_only = table.record([(a, TyId::INT)]);
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let plain = with_meta(&mut table, Effect::Pure, None);
    let with_a = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_only)));
    let with_ab = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, ab)));
    // A function with a meta stands in for one without, never the reverse.
    assert!(table.is_subtype(with_a, plain));
    assert!(!table.is_subtype(plain, with_a));
    // The meta's type is covariant, as a result is.
    assert!(table.is_subtype(with_ab, with_a));
    assert!(!table.is_subtype(with_a, with_ab));
    // The markers are ordered as effects are: all nine pairs.
    let effects = [Effect::Pure, Effect::Memo, Effect::Impure];
    for offered in effects {
        for required in effects {
            let sub = with_meta(&mut table, Effect::Pure, Some((offered, a_only)));
            let sup = with_meta(&mut table, Effect::Pure, Some((required, a_only)));
            assert_eq!(
                table.is_subtype(sub, sup),
                offered <= required,
                "{offered:?} against {required:?}"
            );
        }
    }
    // The function's own effect is still compared on its own.
    let impure = with_meta(&mut table, Effect::Impure, Some((Effect::Pure, a_only)));
    assert!(table.is_subtype(with_a, impure));
    assert!(!table.is_subtype(impure, with_a));
}

#[test]
fn metas_join_as_results_do() {
    let (mut table, _, [a, b, _]) = setup();
    let a_only = table.record([(a, TyId::INT)]);
    let ab = table.record([(a, TyId::INT), (b, TyId::INT)]);
    let a_float = table.record([(a, TyId::FLOAT)]);
    let plain = with_meta(&mut table, Effect::Pure, None);
    let pure_ab = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, ab)));
    let impure_a = with_meta(&mut table, Effect::Pure, Some((Effect::Impure, a_only)));
    // Both sides: the join of the types and the greater marker.
    assert_eq!(table.join(pure_ab, impure_a), (impure_a, false));
    assert_eq!(table.join(impure_a, pure_ab), (impure_a, false));
    // One side only: forgotten.
    assert_eq!(table.join(pure_ab, plain), (plain, false));
    assert_eq!(table.join(plain, impure_a), (plain, false));
    // A widened meta widens the join.
    let float_a = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_float)));
    let pure_a = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_only)));
    let int_float = table.union([TyId::INT, TyId::FLOAT]);
    let a_number = table.record([(a, int_float)]);
    let joined = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, a_number)));
    assert_eq!(table.join(pure_a, float_a), (joined, true));
}

#[test]
fn display_metas_as_written() {
    let mut names = Names::new();
    let tag = names.intern("tag");
    let mut table = TypeTable::new();
    let rec = table.record([(tag, TyId::STRING)]);
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    for (effect, shown) in [
        (Effect::Pure, "fn(): Int META: {tag: String}"),
        (Effect::Memo, "fn(): Int META->> {tag: String}"),
        (Effect::Impure, "fn(): Int META-> {tag: String}"),
    ] {
        let ty = with_meta(&mut table, Effect::Pure, Some((effect, rec)));
        assert_eq!(show(&table, ty), shown);
    }
    // A function type returned by one with a meta is in parentheses,
    // which would take the meta otherwise; one with a meta returned by
    // one without is not.
    let inner = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, rec)));
    let meta = Some(Meta {
        effect: Effect::Impure,
        ty: rec,
    });
    let none: [(Option<Name>, TyId); 0] = [];
    let plain_inner = table.fn_type(none, TyId::INT, Effect::Pure);
    let outer = table.fn_type_with_meta(none, plain_inner, Effect::Pure, meta);
    assert_eq!(
        show(&table, outer),
        "fn(): (fn(): Int) META-> {tag: String}"
    );
    let outer = table.fn_type(none, inner, Effect::Impure);
    assert_eq!(show(&table, outer), "fn() -> fn(): Int META: {tag: String}");
    // In a union, in parentheses as any function type is.
    let union = table.union([inner, TyId::INT]);
    assert_eq!(show(&table, union), "Int | (fn(): Int META: {tag: String})");
}

#[test]
fn a_meta_s_provisional_types_are_settled() {
    let (mut table, _, [a, _, _]) = setup();
    let function = FnId::new(3);
    let provisional = table.provisional(function, 0, TyId::INT);
    let held = table.record([(a, provisional)]);
    let ty = with_meta(&mut table, Effect::Pure, Some((Effect::Pure, held)));
    assert!(table.contains_provisional(ty, Some(function)));
    let mut params = Vec::new();
    table.provisional_params(ty, function, &mut params);
    assert_eq!(params, [0]);
    let settled = table.settle(ty, function, &[true]);
    let present = table.record([(a, TyId::INT)]);
    assert_eq!(
        settled,
        with_meta(&mut table, Effect::Pure, Some((Effect::Pure, present)))
    );
}

/// The record type of `fields` with the rest-field `rest`.
fn with_rest(table: &mut TypeTable, fields: &[(Name, TyId)], rest: TyId) -> TyId {
    table.record_with_rest(fields.iter().copied(), Some(rest), Vec::new())
}

#[test]
fn a_rest_field_is_part_of_the_type_and_shown_after_the_named_fields() {
    let (mut table, names, [a, b, _]) = setup();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    let map = with_rest(&mut table, &[], TyId::STRING);
    assert_eq!(show(&table, map), "{*: String}");
    let file = with_rest(
        &mut table,
        &[(b, TyId::STRING), (a, TyId::STRING)],
        TyId::INT,
    );
    assert_eq!(show(&table, file), "{a: String, b: String, *: Int}");
    // Without it, or with another, it is another type.
    let plain = table.record([(a, TyId::STRING), (b, TyId::STRING)]);
    assert_ne!(file, plain);
    assert_ne!(map, TyId::EMPTY);
    assert_ne!(map, with_rest(&mut table, &[], TyId::INT));
    assert_eq!(table.record_rest(file), Some(TyId::INT));
    assert_eq!(table.record_rest(plain), None);
    let again = with_rest(&mut table, &[], TyId::STRING);
    assert!(table.same(map, again));
    assert!(!table.same(map, TyId::EMPTY));
}

#[test]
fn a_named_field_is_exempt_from_the_rest_field() {
    let (mut table, _, [a, b, _]) = setup();
    let file = with_rest(&mut table, &[(a, TyId::STRING)], TyId::INT);
    assert_eq!(table.field(file, a), Some(TyId::STRING));
    assert_eq!(table.field(file, b), None);
    assert_eq!(table.field_or_rest(file, a), Some(TyId::STRING));
    assert_eq!(table.field_or_rest(file, b), Some(TyId::INT));
    // A field set by a functional update is named, and the rest-field
    // stays.
    let updated = table.with_field(file, b, TyId::BOOL).unwrap();
    assert_eq!(table.field(updated, b), Some(TyId::BOOL));
    assert_eq!(table.record_rest(updated), Some(TyId::INT));
}

#[test]
fn only_a_type_that_says_what_its_other_fields_are_fits_a_rest_field() {
    let (mut table, _, [a, b, _]) = setup();
    let ints = with_rest(&mut table, &[], TyId::INT);
    let a_int = table.record([(a, TyId::INT)]);
    // A type without a rest-field says nothing of the fields it hides.
    assert!(!table.is_subtype(a_int, ints));
    assert!(!table.is_subtype(TyId::EMPTY, ints));
    // One with a rest-field fits if it and every field the other does not
    // name fit.
    let a_int_ints = with_rest(&mut table, &[(a, TyId::INT)], TyId::INT);
    assert!(table.is_subtype(a_int_ints, ints));
    let a_string_ints = with_rest(&mut table, &[(a, TyId::STRING)], TyId::INT);
    assert!(!table.is_subtype(a_string_ints, ints));
    let strings = with_rest(&mut table, &[], TyId::STRING);
    assert!(!table.is_subtype(strings, ints));
    // A named field required must be named, and fit as ever.
    let a_string = with_rest(&mut table, &[(a, TyId::STRING)], TyId::INT);
    assert!(table.is_subtype(a_string_ints, a_string));
    assert!(!table.is_subtype(ints, a_string));
    // The rest-field is covariant, and forgotten where none is required.
    let union = table.union([TyId::INT, TyId::STRING]);
    let either = with_rest(&mut table, &[], union);
    assert!(table.is_subtype(ints, either));
    assert!(!table.is_subtype(either, ints));
    assert!(table.is_subtype(a_int_ints, a_int));
    assert!(table.is_subtype(ints, TyId::EMPTY));
    // A named field the subtype has beyond the required ones is held to
    // the required rest-field.
    let b_string = with_rest(&mut table, &[(b, TyId::STRING)], TyId::STRING);
    let only_strings = with_rest(&mut table, &[], TyId::STRING);
    assert!(table.is_subtype(b_string, only_strings));
    assert!(!table.is_subtype(b_string, ints));
}

#[test]
fn the_join_of_two_rest_fields_covers_the_fields_it_no_longer_names() {
    let (mut table, names, [a, b, c]) = setup();
    let show = |table: &TypeTable, id| table.display(id, &names).to_string();
    let left = with_rest(&mut table, &[(a, TyId::INT), (b, TyId::STRING)], TyId::INT);
    let right = with_rest(&mut table, &[(a, TyId::INT), (c, TyId::BOOL)], TyId::INT);
    let (joined, widened) = table.join(left, right);
    assert_eq!(show(&table, joined), "{a: Int, *: Int | Bool | String}");
    assert!(widened);
    assert!(table.is_subtype(left, joined));
    assert!(table.is_subtype(right, joined));
    // One side without a rest-field: the join has none.
    let plain = table.record([(a, TyId::INT)]);
    let (joined, widened) = table.join(left, plain);
    assert_eq!(show(&table, joined), "{a: Int}");
    assert!(!widened);
    // Two maps of one type join to it.
    let ints = with_rest(&mut table, &[], TyId::INT);
    let more = with_rest(&mut table, &[], TyId::INT);
    assert_eq!(table.join(ints, more), (ints, false));
}

#[test]
fn a_field_one_side_names_overlaps_the_other_s_rest_field() {
    let (mut table, _, [a, _, _]) = setup();
    let ints = with_rest(&mut table, &[], TyId::INT);
    let a_string = table.record([(a, TyId::STRING)]);
    let a_int = table.record([(a, TyId::INT)]);
    // Both ways round.
    assert!(!table.overlaps(a_string, ints));
    assert!(!table.overlaps(ints, a_string));
    assert!(table.overlaps(a_int, ints));
    assert!(table.overlaps(ints, a_int));
    // Two maps hold at least the empty record in common.
    let strings = with_rest(&mut table, &[], TyId::STRING);
    assert!(table.overlaps(ints, strings));
}

#[test]
fn a_test_of_a_rest_field_tests_the_fields_the_type_does_not_name() {
    let (mut table, _, [a, b, _]) = setup();
    let ints = with_rest(&mut table, &[], TyId::INT);
    let rest_ints = ValueTest::RecordRest {
        fields: Box::default(),
        named: Box::default(),
        rest: Box::new(ValueTest::Kind(ValueKind::Int)),
    };
    // Of any value, or of a record type that hides its other fields.
    assert_eq!(table.test_plan(None, ints), rest_ints);
    let a_int = table.record([(a, TyId::INT)]);
    assert_eq!(table.test_plan(Some(&[a_int]), ints), rest_ints);
    // A type whose rest-field already fits passes.
    let a_int_ints = with_rest(&mut table, &[(a, TyId::INT)], TyId::INT);
    assert_eq!(table.test_plan(Some(&[a_int_ints]), ints), ValueTest::Pass);
    // Named fields are tested by name and passed over by the rest-field's
    // test, which looks only where the value's type leaves it in doubt.
    let file = with_rest(&mut table, &[(a, TyId::STRING)], TyId::INT);
    let union = table.union([TyId::INT, TyId::STRING]);
    let mixed = with_rest(&mut table, &[(a, TyId::STRING), (b, union)], TyId::INT);
    assert_eq!(
        table.test_plan(Some(&[mixed]), file),
        ValueTest::RecordRest {
            fields: Box::default(),
            named: Box::new([a]),
            rest: Box::new(ValueTest::Kind(ValueKind::Int)),
        }
    );
    assert!(table.test_plan(None, file).walks());
}

#[test]
fn a_rest_field_is_settled_and_made_present_as_a_field_is() {
    let (mut table, _, [a, _, _]) = setup();
    let function = FnId::new(1);
    let provisional = table.provisional(function, 0, TyId::INT);
    let held = with_rest(&mut table, &[(a, TyId::STRING)], provisional);
    assert!(table.contains_provisional(held, Some(function)));
    let settled = table.settle(held, function, &[true]);
    assert_eq!(
        settled,
        with_rest(&mut table, &[(a, TyId::STRING)], TyId::INT)
    );
    let lazy = table.future(TyId::INT);
    let futures = with_rest(&mut table, &[], lazy);
    assert!(table.contains_future(futures));
    let present = table.present(futures);
    assert_eq!(present, with_rest(&mut table, &[], TyId::INT));
}
