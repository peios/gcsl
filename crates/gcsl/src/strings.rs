//! The string table: the text of every string literal piece.
//!
//! The lexer processes each string literal completely, stripping its
//! indentation and replacing its escapes, and stores the resulting text of
//! each piece (a whole literal, or the text before, between or after its
//! interpolations) in a [`Strings`] table. A string token therefore carries
//! a [`StrId`], not text, and no later stage looks at a literal's source
//! again.
//!
//! Unlike [`crate::names::Names`], the table does not deduplicate: every
//! piece gets its own entry, since equal pieces are rare and comparing them
//! would cost more than it saves. The table belongs to one input: the lexer
//! fills it, the [`crate::ast::Ast`] owns it, and lowering copies each piece
//! the program uses into the IR's own reference-counted strings, after
//! which the table is no longer needed.

use std::fmt;

/// A piece of string literal text: an index into the [`Strings`] table that
/// produced it.
///
/// Ids are only meaningful for the table that produced them; using one with
/// a different table is a logic error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StrId(u32);

impl StrId {
    /// The id as an index into its table.
    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The text of every string literal piece of one input, indexed by
/// [`StrId`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strings {
    texts: Vec<Box<str>>,
}

impl Strings {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a copy of `text`, allocated at exactly its length, and returns
    /// its id.
    ///
    /// A table holds at most `u32::MAX` pieces. Every piece comes from a
    /// literal of at least one byte of source, and a literal has at most one
    /// piece more than it has bytes, so a lexer can never reach the limit;
    /// past it, ids would saturate.
    pub fn push(&mut self, text: &str) -> StrId {
        let id = StrId(u32::try_from(self.texts.len()).unwrap_or(u32::MAX));
        self.texts.push(Box::from(text));
        id
    }

    /// The text of piece `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this table.
    #[must_use]
    pub fn get(&self, id: StrId) -> &str {
        &self.texts[id.index()]
    }

    /// The number of pieces in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// Whether the table has no pieces.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }
}

/// Shows text as a double-quoted GCSL string literal that reads back as the
/// same text: `\n`, `\t`, `\r`, `\` and `"` are written as the escapes
/// `\n`, `\t`, `\r`, `\\` and `\"`, `{` and `}` are doubled, and every other
/// character is written verbatim. Used wherever a string is shown as a
/// value rather than as text: [`crate::Value::repr`] and the dumps.
#[derive(Clone, Copy, Debug)]
pub struct Quoted<'a>(pub &'a str);

impl fmt::Display for Quoted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        let text = self.0;
        let mut run_start = 0;
        for (at, ch) in text.char_indices() {
            let escape = match ch {
                '\n' => "\\n",
                '\t' => "\\t",
                '\r' => "\\r",
                '\\' => "\\\\",
                '"' => "\\\"",
                '{' => "{{",
                '}' => "}}",
                _ => continue,
            };
            // Write the unescaped run before this character in one go.
            f.write_str(text.get(run_start..at).unwrap_or_default())?;
            f.write_str(escape)?;
            run_start = at + ch.len_utf8();
        }
        f.write_str(text.get(run_start..).unwrap_or_default())?;
        f.write_str("\"")
    }
}

/// Shows the name of a record field as it can be written in source: as it
/// is if it is a plain identifier (see [`crate::lexer::is_identifier`]), and
/// otherwise as a string literal, [`Quoted`]: `x`, `hello-world`, `"a b"`,
/// `"let"`. Used wherever a field name is shown: types, record values,
/// diagnostics and dumps.
#[derive(Clone, Copy, Debug)]
pub struct FieldName<'a>(pub &'a str);

impl fmt::Display for FieldName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if crate::lexer::is_identifier(self.0) {
            f.write_str(self.0)
        } else {
            Quoted(self.0).fmt(f)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_names_are_quoted_unless_plain_identifiers() {
        assert_eq!(FieldName("x").to_string(), "x");
        assert_eq!(FieldName("hello-world").to_string(), "hello-world");
        assert_eq!(FieldName("_1").to_string(), "_1");
        assert_eq!(FieldName("a b").to_string(), "\"a b\"");
        assert_eq!(FieldName("").to_string(), "\"\"");
        assert_eq!(FieldName("1x").to_string(), "\"1x\"");
        assert_eq!(FieldName("a-").to_string(), "\"a-\"");
        assert_eq!(FieldName("if").to_string(), "\"if\"");
        assert_eq!(FieldName("é").to_string(), "\"é\"");
        assert_eq!(FieldName("{x}").to_string(), "\"{{x}}\"");
    }

    #[test]
    fn pushed_text_comes_back_by_id() {
        let mut strings = Strings::new();
        assert!(strings.is_empty());
        let a = strings.push("hello");
        let b = strings.push("");
        // No deduplication: equal texts get separate entries.
        let c = strings.push("hello");
        assert_eq!(strings.get(a), "hello");
        assert_eq!(strings.get(b), "");
        assert_eq!(strings.get(c), "hello");
        assert_ne!(a, c);
        assert_eq!(strings.len(), 3);
    }

    #[test]
    fn quoted_reads_back_as_the_same_text() {
        assert_eq!(Quoted("").to_string(), "\"\"");
        assert_eq!(Quoted("plain").to_string(), "\"plain\"");
        assert_eq!(
            Quoted("a\nb\tc\rd\\e\"f").to_string(),
            "\"a\\nb\\tc\\rd\\\\e\\\"f\""
        );
        // Braces are doubled so the literal does not read as an
        // interpolation; everything else is verbatim, including other
        // control characters and non-ASCII text.
        assert_eq!(Quoted("{x} \u{0} é😀").to_string(), "\"{{x}} \u{0} é😀\"");
    }
}
