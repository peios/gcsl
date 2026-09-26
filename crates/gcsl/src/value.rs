//! Runtime values.

use std::fmt;
use std::rc::Rc;

use crate::names::Name;
use crate::strings::{FieldName, Quoted};

/// The kind of value an expression produces: all that the IR and the
/// evaluator need to know of its type. Every record, the empty record
/// included, is [`ValueKind::Record`]. See
/// [`crate::types::TypeTable::value_kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueKind {
    /// An `Int`, [`Value::Int`].
    Int,
    /// A `Float`, [`Value::Float`].
    Float,
    /// A `Bool`, [`Value::Bool`].
    Bool,
    /// A `String`, [`Value::Str`].
    String,
    /// A record: [`Value::Empty`] or [`Value::Record`].
    Record,
}

/// The field names of a record, in definition order. Built once per record
/// block when a program is lowered and shared by every record that block
/// builds, so building a record never copies names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The name of each field, in definition order. Never empty and never
    /// repeating a name.
    pub names: Box<[Name]>,
    /// The text of each name, in the same order, so that a record can be
    /// shown without the names table.
    pub texts: Box<[Box<str>]>,
}

/// A record with at least one field. The empty record is [`Value::Empty`].
#[derive(Clone, Debug)]
pub struct Record {
    /// The record's field names, shared with every record built by the
    /// same record block.
    pub layout: Rc<Layout>,
    /// The value of each field, in the layout's order.
    pub values: Box<[Value]>,
}

impl Record {
    /// The value of field `name`, or `None` if the record has no such
    /// field.
    ///
    /// A linear scan of the layout, comparing interned names: records are
    /// small. A bytecode VM would cache each access site's field index
    /// against the layout it last saw instead.
    #[must_use]
    pub fn get(&self, name: Name) -> Option<&Value> {
        let index = self.layout.names.iter().position(|&field| field == name)?;
        self.values.get(index)
    }
}

/// Dropping a record frees its nested records iteratively, not recursively.
///
/// The default drop would recurse once per level of nesting, and a record
/// nested through variables (`let a = { x = a }` repeated) can be nested as
/// deeply as the program is long, far beyond any stack. Instead, the nested
/// records this record uniquely owns are moved onto a work list and freed
/// level by level. A record with no nested records allocates nothing here:
/// the list only grows when there is something to put on it. A nested
/// record that is still shared elsewhere is merely released.
impl Drop for Record {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_nested(&mut self.values, &mut pending);
        while let Some(child) = pending.pop() {
            if let Ok(mut child) = Rc::try_unwrap(child) {
                // Emptied of nested records, `child` drops without adding
                // to the list or recursing.
                take_nested(&mut child.values, &mut pending);
            }
        }
    }
}

/// Moves the records among `values` onto `pending`, leaving `{}` in their
/// places.
fn take_nested(values: &mut [Value], pending: &mut Vec<Rc<Record>>) {
    for value in values {
        if let Value::Record(_) = value
            && let Value::Record(child) = std::mem::replace(value, Value::Empty)
        {
            pending.push(child);
        }
    }
}

/// Deep equality by field name: two records are equal when they have the
/// same number of fields and every field of one is in the other with an
/// equal value. Field order does not matter, and `Float` fields compare as
/// IEEE 754 does, so a record holding NaN is not equal to itself.
///
/// Nested records are compared iteratively through a work list, for the
/// same reason [`Record`]'s drop is iterative: nesting is bounded only by
/// the program's length. The list only grows when both records have a
/// nested record in the same field, so comparing flat records allocates
/// nothing.
impl PartialEq for Record {
    fn eq(&self, other: &Self) -> bool {
        let mut pending = Vec::new();
        if !fields_equal(self, other, &mut pending) {
            return false;
        }
        while let Some((mine, theirs)) = pending.pop() {
            if !fields_equal(mine, theirs, &mut pending) {
                return false;
            }
        }
        true
    }
}

/// Compares the fields of `mine` and `theirs`, deferring pairs of nested
/// records to `pending` instead of recursing into them.
fn fields_equal<'a>(
    mine: &'a Record,
    theirs: &'a Record,
    pending: &mut Vec<(&'a Record, &'a Record)>,
) -> bool {
    if mine.values.len() != theirs.values.len() {
        return false;
    }
    let same_layout =
        Rc::ptr_eq(&mine.layout, &theirs.layout) || mine.layout.names == theirs.layout.names;
    for (index, (&name, value)) in mine.layout.names.iter().zip(&mine.values).enumerate() {
        let other = if same_layout {
            theirs.values.get(index)
        } else {
            theirs.get(name)
        };
        let Some(other) = other else {
            return false;
        };
        match (value, other) {
            (Value::Record(a), Value::Record(b)) => pending.push((a, b)),
            _ if value == other => {}
            _ => return false,
        }
    }
    true
}

/// The result of evaluating an expression.
///
/// Strings and records have value semantics but are shared: cloning a
/// `Value::Str` or a `Value::Record` increments a reference count and never
/// copies the text or the fields.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An `Int`.
    Int(i64),
    /// A `Float`.
    Float(f64),
    /// A `Bool`.
    Bool(bool),
    /// A `String`.
    Str(Rc<str>),
    /// `{}`, the empty record. Unboxed, so that the many statements whose
    /// value it is produce it without allocating.
    Empty,
    /// A record with at least one field; the empty record is always
    /// [`Value::Empty`].
    Record(Rc<Record>),
}

impl Value {
    /// The kind of the value.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Value::Int(_) => ValueKind::Int,
            Value::Float(_) => ValueKind::Float,
            Value::Bool(_) => ValueKind::Bool,
            Value::Str(_) => ValueKind::String,
            Value::Empty | Value::Record(_) => ValueKind::Record,
        }
    }

    /// The value as it would be written in a program, for the REPL: a
    /// string double-quoted with its special characters escaped (see
    /// [`Quoted`]), and every other value as its `Display` form.
    #[must_use]
    pub const fn repr(&self) -> Repr<'_> {
        Repr(self)
    }
}

/// Shows the value only, never its type.
///
/// A `Float` always shows a decimal point or an exponent, so it cannot be
/// mistaken for an `Int`: `3.0`, `1.5`, `1e300`, `1e-7`. It uses the
/// shortest text that reads back as the same value. The non-finite values
/// show as `inf`, `-inf` and `nan`. A `Bool` shows as `true` or `false`, and
/// a `String` as its text with no quotes or escaping.
///
/// A record shows as GCSL syntax, the same as its [`Value::repr`]: its
/// fields in definition order, each value in `repr` form, so strings are
/// quoted, as in `{x = 1, y = "a", inner = {n = 2}}`; a field name that is
/// not a plain identifier is quoted too (see [`FieldName`]). The empty
/// record is `{}`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(value) => write!(f, "{value}"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Str(text) => f.write_str(text),
            Value::Empty => f.write_str("{}"),
            Value::Record(record) => {
                f.write_str("{")?;
                for (index, (text, value)) in
                    record.layout.texts.iter().zip(&record.values).enumerate()
                {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{} = {}", FieldName(text), value.repr())?;
                }
                f.write_str("}")
            }
            // `{:?}` would show NaN as `NaN`.
            Value::Float(value) if value.is_nan() => f.write_str("nan"),
            // `{:?}` on f64 is the shortest round-trip representation and
            // always includes `.` or `e`; it shows infinities as `inf`/`-inf`.
            Value::Float(value) => write!(f, "{value:?}"),
        }
    }
}

/// A [`Value`] shown as it would be written in a program. Returned by
/// [`Value::repr`].
#[derive(Clone, Copy, Debug)]
pub struct Repr<'a>(&'a Value);

impl fmt::Display for Repr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Str(text) => Quoted(text).fmt(f),
            value => value.fmt(f),
        }
    }
}

/// Converts an `Int` to the nearest `Float` (ties to even), the meaning of
/// both an integer literal in `Float` context and an `IntToFloat` node.
#[expect(
    clippy::cast_precision_loss,
    reason = "rounding to the nearest f64 is the defined meaning of Int-to-Float conversion"
)]
pub(crate) const fn int_to_float(value: i64) -> f64 {
    value as f64
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "these tests check exact IEEE 754 results, not approximations"
)]
mod tests {
    use super::*;
    use crate::names::Names;

    fn show(value: f64) -> String {
        Value::Float(value).to_string()
    }

    fn string(text: &str) -> Value {
        Value::Str(Rc::from(text))
    }

    /// A layout for `names`, interned into `table`.
    fn layout(table: &mut Names, names: &[&str]) -> Rc<Layout> {
        Rc::new(Layout {
            names: names.iter().map(|text| table.intern(text)).collect(),
            texts: names.iter().map(|&text| Box::from(text)).collect(),
        })
    }

    fn record(layout: &Rc<Layout>, values: Vec<Value>) -> Value {
        Value::Record(Rc::new(Record {
            layout: Rc::clone(layout),
            values: values.into_boxed_slice(),
        }))
    }

    /// A record nested `depth` deep through the field `inner`, ending in
    /// `leaf`.
    fn nested(names: &mut Names, depth: usize, leaf: Value) -> Value {
        let layout = layout(names, &["inner"]);
        let mut value = leaf;
        for _ in 0..depth {
            value = record(&layout, vec![value]);
        }
        value
    }

    #[test]
    fn comparing_deeply_nested_records_does_not_recurse() {
        // Far deeper than a test thread's stack could take recursively.
        let mut names = Names::new();
        let a = nested(&mut names, 1_000_000, Value::Int(0));
        let b = nested(&mut names, 1_000_000, Value::Int(0));
        let c = nested(&mut names, 1_000_000, Value::Int(1));
        assert_eq!(a, b);
        assert_ne!(a, c);
        // Different depths differ where one bottoms out.
        assert_ne!(a, nested(&mut names, 999_999, Value::Int(0)));
    }

    #[test]
    fn dropping_a_deeply_nested_record_does_not_recurse() {
        // Far deeper than a test thread's stack could take recursively.
        let mut names = Names::new();
        let value = nested(&mut names, 1_000_000, Value::Int(0));
        // A shared nested record is released, not freed, and stays intact.
        let Value::Record(outer) = &value else {
            panic!("expected a record");
        };
        let shared = outer.values[0].clone();
        drop(value);
        let Value::Record(shared) = shared else {
            panic!("expected a record");
        };
        assert!(matches!(shared.values[0], Value::Record(_)));
        drop(shared);
    }

    #[test]
    fn values_are_small() {
        // A string is a fat `Rc<str>` pointer, 16 bytes, plus the tag; a
        // record is a thin `Rc<Record>`. A thin string pointer would bring
        // this back to 16 bytes; that is a later optimisation for the VM's
        // value representation, not this tree walker's.
        assert!(size_of::<Value>() <= 24);
        assert_eq!(size_of::<Rc<Record>>(), size_of::<usize>());
    }

    #[test]
    fn ints_display_as_decimal() {
        assert_eq!(Value::Int(0).to_string(), "0");
        assert_eq!(Value::Int(-42).to_string(), "-42");
        assert_eq!(Value::Int(i64::MIN).to_string(), "-9223372036854775808");
    }

    #[test]
    fn floats_always_show_their_type() {
        assert_eq!(show(3.0), "3.0");
        assert_eq!(show(1.5), "1.5");
        assert_eq!(show(-0.0), "-0.0");
        assert_eq!(show(0.1 + 0.2), "0.30000000000000004");
    }

    #[test]
    fn non_finite_floats() {
        assert_eq!(show(f64::INFINITY), "inf");
        assert_eq!(show(f64::NEG_INFINITY), "-inf");
        assert_eq!(show(f64::NAN), "nan");
        assert_eq!(show(-f64::NAN), "nan");
    }

    #[test]
    fn large_and_small_floats() {
        assert_eq!(show(1e300), "1e300");
        assert_eq!(show(1e16), "1e16");
        assert_eq!(show(1e15), "1000000000000000.0");
        assert_eq!(show(1.5e-7), "1.5e-7");
        assert_eq!(show(0.0001), "0.0001");
        assert_eq!(show(f64::MAX), "1.7976931348623157e308");
        assert_eq!(show(f64::MIN_POSITIVE), "2.2250738585072014e-308");
        assert_eq!(show(5e-324), "5e-324");
    }

    #[test]
    fn kinds() {
        let mut names = Names::new();
        let xs = layout(&mut names, &["x"]);
        assert_eq!(Value::Int(1).kind(), ValueKind::Int);
        assert_eq!(Value::Float(1.0).kind(), ValueKind::Float);
        assert_eq!(Value::Bool(true).kind(), ValueKind::Bool);
        assert_eq!(string("a").kind(), ValueKind::String);
        assert_eq!(Value::Empty.kind(), ValueKind::Record);
        assert_eq!(record(&xs, vec![Value::Int(1)]).kind(), ValueKind::Record);
    }

    #[test]
    fn the_empty_record_displays_as_empty_braces() {
        assert_eq!(Value::Empty.to_string(), "{}");
        assert_eq!(Value::Empty.repr().to_string(), "{}");
    }

    #[test]
    fn bools_and_strings_display_plainly() {
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Bool(false).to_string(), "false");
        // A string's text, unquoted and unescaped.
        assert_eq!(string("say \"hi\"\n\t{x}").to_string(), "say \"hi\"\n\t{x}");
        assert_eq!(string("").to_string(), "");
    }

    #[test]
    fn repr_quotes_strings_and_shows_everything_else_as_display() {
        assert_eq!(string("hello").repr().to_string(), "\"hello\"");
        assert_eq!(string("").repr().to_string(), "\"\"");
        assert_eq!(
            string("a\nb\tc\rd\\e\"f").repr().to_string(),
            "\"a\\nb\\tc\\rd\\\\e\\\"f\""
        );
        // Braces are doubled so the literal reads back; other characters
        // are verbatim.
        assert_eq!(string("{é}").repr().to_string(), "\"{{é}}\"");
        assert_eq!(Value::Int(-3).repr().to_string(), "-3");
        assert_eq!(Value::Float(2.0).repr().to_string(), "2.0");
        assert_eq!(Value::Float(f64::NAN).repr().to_string(), "nan");
        assert_eq!(Value::Bool(false).repr().to_string(), "false");
    }

    #[test]
    fn records_display_as_gcsl_syntax() {
        let mut names = Names::new();
        let inner = record(&layout(&mut names, &["n"]), vec![Value::Int(2)]);
        let outer = record(
            &layout(&mut names, &["x", "y", "inner", "a b", "e"]),
            vec![
                Value::Int(1),
                string("a\n"),
                inner,
                Value::Float(1.0),
                Value::Empty,
            ],
        );
        // Definition order, values in `repr` form, quoted names.
        let shown = "{x = 1, y = \"a\\n\", inner = {n = 2}, \"a b\" = 1.0, e = {}}";
        assert_eq!(outer.to_string(), shown);
        assert_eq!(outer.repr().to_string(), shown);
    }

    #[test]
    fn records_compare_deeply_by_field_name() {
        let mut names = Names::new();
        let xy = layout(&mut names, &["x", "y"]);
        let yx = layout(&mut names, &["y", "x"]);
        let xz = layout(&mut names, &["x", "z"]);
        let x = layout(&mut names, &["x"]);
        let a = record(&xy, vec![Value::Int(1), string("s")]);
        // The same layout, or another with the same names in another order.
        assert_eq!(a, record(&xy, vec![Value::Int(1), string("s")]));
        assert_eq!(a, record(&yx, vec![string("s"), Value::Int(1)]));
        assert_ne!(a, record(&yx, vec![string("t"), Value::Int(1)]));
        assert_ne!(a, record(&xy, vec![Value::Int(2), string("s")]));
        // Different names, or more or fewer of them.
        assert_ne!(a, record(&xz, vec![Value::Int(1), string("s")]));
        assert_ne!(a, record(&x, vec![Value::Int(1)]));
        assert_ne!(record(&x, vec![Value::Int(1)]), a);
        assert_ne!(record(&x, vec![Value::Int(1)]), Value::Empty);
        assert_eq!(Value::Empty, Value::Empty);
        // Nested, and IEEE for floats.
        let nested = |value: f64| record(&x, vec![record(&x, vec![Value::Float(value)])]);
        assert_eq!(nested(1.0), nested(1.0));
        assert_eq!(nested(0.0), nested(-0.0));
        assert_ne!(nested(1.0), nested(2.0));
        assert_ne!(nested(f64::NAN), nested(f64::NAN));
        // Reading a field.
        let Value::Record(fields) = &a else {
            panic!("expected a record");
        };
        assert_eq!(fields.get(names.intern("y")), Some(&string("s")));
        assert_eq!(fields.get(names.intern("z")), None);
    }

    #[test]
    fn cloning_a_string_shares_its_text() {
        let original = string("shared");
        let copy = original.clone();
        let (Value::Str(a), Value::Str(b)) = (&original, &copy) else {
            panic!("expected strings");
        };
        assert!(Rc::ptr_eq(a, b));
        assert_eq!(original, copy);
        // Equality compares content, not identity.
        assert_eq!(string("x"), string("x"));
        assert_ne!(string("x"), string("y"));
    }

    #[test]
    fn int_to_float_rounds_to_nearest() {
        assert_eq!(int_to_float(3), 3.0);
        assert_eq!(int_to_float(9_007_199_254_740_993), 9_007_199_254_740_992.0);
        assert_eq!(int_to_float(i64::MIN), -9_223_372_036_854_775_808.0);
    }
}
