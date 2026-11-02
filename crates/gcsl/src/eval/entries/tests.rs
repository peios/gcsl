//! Records of any names, end to end.

/// The value `text`, a lone file, gives, shown as GCSL source.
fn value(text: &str) -> String {
    let source = crate::source::SourceFile::new("test.gcsl", text).unwrap();
    match crate::run(&source) {
        Ok(outcome) => outcome.value.to_string(),
        Err(errors) => panic!("{text}: {errors:?}"),
    }
}

/// The first error `text`, a lone file, fails with.
fn failure(text: &str) -> String {
    let source = crate::source::SourceFile::new("test.gcsl", text).unwrap();
    let errors = crate::run(&source)
        .err()
        .unwrap_or_else(|| panic!("{text} ran"));
    errors.into_iter().next().unwrap().message
}

#[test]
fn a_record_has_a_field_for_each_entry() {
    assert_eq!(
        value("= record([{key = \"A\"; value = 1}, {key = \"B\"; value = 2}])"),
        "{A = 1; B = 2}"
    );
    // Named as the program names them, in any order.
    assert_eq!(
        value(
            "let r = record([{key = \"B\"; value = 2}, {key = \"A\"; value = 1}])\n\
             = r == ({A = 1; B = 2}: {*: Int})"
        ),
        "true"
    );
    // A type expected gives the entries theirs; none is the empty record.
    assert_eq!(value("let r: {*: String} = record([])\n= r"), "{}");
    let entries = "let es: List<Entry<Int>> = [{key = \"n\"; value = 1}]\n= record(es)";
    assert_eq!(value(entries), "{n = 1}");
    // What takes a record of any names takes it: a program's environment.
    assert_eq!(
        value(
            "= exec([\"sh\", \"-c\", \"echo $V\"], env = record([{key = \"V\"; value = \"x\"}])).stdout"
        ),
        "x\n"
    );
}

#[test]
fn a_record_s_entries_are_checked() {
    assert_eq!(
        failure("= record([1, 2])"),
        "`record` takes a list of `{key: String, value: T}` entries, found `List<Int>`"
    );
    assert_eq!(
        failure("= record([{key = 1; value = 2}])"),
        "`record` takes a list of `{key: String, value: T}` entries, found `List<{key: Int, value: Int}>`"
    );
    // No field of it is read by name.
    assert_eq!(
        failure("let r = record([{key = \"x\"; value = 1}])\n= r.x"),
        "no field `x` in `{*: Int}`"
    );
    // A key twice, at run time.
    assert_eq!(
        failure("= record([{key = \"A\"; value = 1}, {key = \"A\"; value = 2}])"),
        "the key `A` is in the entries twice"
    );
}

#[test]
fn a_record_s_entries_are_its_fields_sorted_by_name() {
    assert_eq!(
        value("let r: {*: String} = {b = \"2\"; \"a.b\" = \"1\"; B = \"0\"}\n= entries(r)"),
        "[{key = \"B\"; value = \"0\"}, {key = \"a.b\"; value = \"1\"}, {key = \"b\"; value = \"2\"}]"
    );
    // The same record, made in another order, gives the same list.
    assert_eq!(
        value(
            "let r = record([{key = \"b\"; value = 2}, {key = \"a\"; value = 1}])\n\
             let s: {*: Int} = {a = 1; b = 2}\n\
             = entries(r) == entries(s)"
        ),
        "true"
    );
    // `record` makes the record again; the empty record has no entries.
    assert_eq!(
        value("let r: {*: Int} = {b = 2; a = 1}\n= record(entries(r)) == r"),
        "true"
    );
    assert_eq!(value("let r: {*: Int} = {}\n= entries(r)"), "[]");
    // Of a call's result, which the machine evaluates as an operand.
    assert_eq!(
        value("fn m() -> {*: Int} { {a = 1} }\n= entries(m())"),
        "[{key = \"a\"; value = 1}]"
    );
    // Each entry is an `Entry`, its fields read by name.
    assert_eq!(
        value(
            "let r: {*: String} = {x = \"1\"}\n\
             let es: List<Entry<String>> = entries(r)\n\
             = es[0].key + es[0].value"
        ),
        "x1"
    );
    // A value that was a future is present.
    assert_eq!(
        value("let r: {*: String | Future<String>} = {a = lazy \"v\"}\n= entries(r)"),
        "[{key = \"a\"; value = \"v\"}]"
    );
}

#[test]
fn an_entry_s_value_covers_the_named_fields_too() {
    assert_eq!(
        value(
            "let r: {to: String, *: Int} = {to = \"x\"; n = 1}\n\
             let es: List<Entry<Int | String>> = entries(r)\n\
             = es"
        ),
        "[{key = \"n\"; value = 1}, {key = \"to\"; value = \"x\"}]"
    );
}

#[test]
fn the_entries_of_an_open_record_are_read_by_a_test() {
    let open = "type Service = {name: String, *: *}\n\
                let s: Service = {name = \"web\"; limits = {cpu = 2}}\n";
    assert_eq!(
        value(&format!(
            "{open}= for e in entries(s) {{ e is {{value: String}} }}"
        )),
        "[false, true]"
    );
    assert_eq!(
        value(&format!(
            "{open}let e = entries(s)[0]\n= (e as {{value: {{cpu: Int}}}}).value.cpu"
        )),
        "2"
    );
    // A field the record's type hid is an entry too, which a test of data
    // does not take for data.
    assert_eq!(
        value(
            "fn mk() -> {a: Int} { {a = 1; f = fn(x: Int) -> Int { x }} }\n\
             let r: {*: *} = mk()\n\
             = for e in entries(r) { [e.key, show(e is {value: Int})] }"
        ),
        "[[\"a\", \"true\"], [\"f\", \"false\"]]"
    );
    assert_eq!(
        failure("let r: {*: *} = {a = 1}\n= entries(r)[0].value"),
        "no field `value` in `{key: String, *: *}`"
    );
}

#[test]
fn only_a_record_with_a_rest_field_has_entries() {
    assert_eq!(
        failure("let r = {a = 1}\n= entries(r)"),
        "`entries` takes a record whose type has a rest-field, found `{a: Int}`"
    );
    assert_eq!(
        failure("= entries([1])"),
        "`entries` takes a record whose type has a rest-field, found `List<Int>`"
    );
    assert_eq!(
        failure(
            "let r: {f: fn(x: Int) -> Int, *: *} = {f = fn(x: Int) -> Int { x }}\n= entries(r)"
        ),
        "`entries` lists data, but the field `f` is `fn(x: Int | Future<Int>) -> Int | Future<Int>`"
    );
}
