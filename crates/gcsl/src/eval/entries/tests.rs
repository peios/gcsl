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
