//! Parsing JSON text, as RFC 8259 defines it, into a tree for decoding.
//!
//! The parser is strict: it takes exactly one value, with whitespace around
//! it, and rejects anything RFC 8259 does not allow, a trailing comma, a
//! comment, a leading zero, a raw control character in a string, or an
//! unpaired surrogate escape. It also rejects an object with a key twice,
//! which RFC 8259 leaves unpredictable. An error names the line and column
//! of the character it stopped at, each counted from 1, columns in
//! characters.
//!
//! Nesting deeper than [`MAX_DEPTH`] is an error, so that a document cannot
//! exhaust the stack: the parser recurses once per level.

use std::collections::HashSet;

/// How deeply arrays and objects may nest.
pub const MAX_DEPTH: usize = 512;

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Number(Number),
    /// A string, its escapes resolved.
    Str(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, its members in the order written, no key twice.
    Object(Vec<(String, Json)>),
}

/// A JSON number, as written and as the values it can be.
#[derive(Clone, Debug, PartialEq)]
pub struct Number {
    /// The text written.
    pub text: Box<str>,
    /// The `Int` it is, if it is written as an integer, with no fraction or
    /// exponent, in the range of an `i64`.
    pub int: Option<i64>,
    /// The nearest `f64`, if that is finite.
    pub float: Option<f64>,
}

/// Parses `text` as one JSON value, or says why it is not one, with the
/// line and column it stopped at.
pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        at: 0,
        depth: 0,
    };
    let result = parser.document();
    result.map_err(|message| {
        let before = text.get(..parser.at).unwrap_or(text);
        let line = before.matches('\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map_or(0, |last| last.chars().count())
            + 1;
        format!("{message}, at line {line}, column {column}")
    })
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn document(&mut self) -> Result<Json, String> {
        self.whitespace();
        let value = self.value()?;
        self.whitespace();
        if self.at < self.bytes.len() {
            return Err("there is more after the value".into());
        }
        Ok(value)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8, what: &str) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {what}"))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => self.string().map(Json::Str),
            Some(b'-' | b'0'..=b'9') => self.number().map(Json::Number),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(_) => Err("expected a value".into()),
            None => Err("the text ends where a value should be".into()),
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes.get(self.at..self.at + word.len()) == Some(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err("expected a value".into())
        }
    }

    fn nested(&mut self, parse: fn(&mut Self) -> Result<Json, String>) -> Result<Json, String> {
        if self.depth == MAX_DEPTH {
            return Err(format!(
                "arrays and objects nest more than {MAX_DEPTH} deep"
            ));
        }
        self.depth += 1;
        let value = parse(self);
        self.depth -= 1;
        value
    }

    fn array(&mut self) -> Result<Json, String> {
        self.at += 1;
        self.whitespace();
        let mut elements = Vec::new();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(elements));
        }
        loop {
            self.whitespace();
            elements.push(self.value()?);
            self.whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(elements));
                }
                _ => return Err("expected `,` or `]`".into()),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.at += 1;
        self.whitespace();
        let mut members: Vec<(String, Json)> = Vec::new();
        let mut keys = HashSet::new();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.whitespace();
            let start = self.at;
            if self.peek() != Some(b'"') {
                return Err("expected a key, a string".into());
            }
            let key = self.string()?;
            if !keys.insert(key.clone()) {
                self.at = start;
                return Err(format!("the key {key:?} is in the object twice"));
            }
            self.whitespace();
            self.expect(b':', "`:`")?;
            self.whitespace();
            let value = self.value()?;
            members.push((key, value));
            self.whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err("expected `,` or `}`".into()),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let run = self.at;
            while let Some(byte) = self.peek()
                && byte != b'"'
                && byte != b'\\'
                && byte >= 0x20
            {
                self.at += 1;
            }
            // The run ends at an ASCII byte or the end, so it is whole
            // characters of the text, which is UTF-8.
            out.push_str(self.text.get(run..self.at).unwrap_or_default());
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    out.push(self.escape()?);
                }
                Some(_) => return Err("a control character must be escaped in a string".into()),
                None => return Err("the text ends inside a string".into()),
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let Some(byte) = self.peek() else {
            return Err("the text ends inside a string".into());
        };
        self.at += 1;
        Ok(match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode(),
            _ => {
                self.at -= 1;
                return Err("not an escape JSON has".into());
            }
        })
    }

    fn unicode(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                if self.bytes.get(self.at..self.at + 2) != Some(b"\\u") {
                    return Err("an escaped high surrogate must be followed by a low one".into());
                }
                self.at += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err("an escaped high surrogate must be followed by a low one".into());
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => {
                return Err("an escaped low surrogate must follow a high one".into());
            }
            _ => first,
        };
        char::from_u32(code).ok_or_else(|| "not a character".into())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .filter(|digits| digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("`\\u` needs four hexadecimal digits")?;
        self.at += 4;
        u32::from_str_radix(digits, 16).map_err(|_| "`\\u` needs four hexadecimal digits".into())
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while let Some(b'0'..=b'9') = self.peek() {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Number, String> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if let Some(b'0'..=b'9') = self.peek() {
                    return Err("a number cannot have a leading zero".into());
                }
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err("expected a digit".into()),
        }
        let mut integral = true;
        if self.peek() == Some(b'.') {
            integral = false;
            self.at += 1;
            if self.digits() == 0 {
                return Err("expected a digit after `.`".into());
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            integral = false;
            self.at += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err("expected a digit in the exponent".into());
            }
        }
        let text = self.text.get(start..self.at).unwrap_or_default();
        Ok(Number {
            text: Box::from(text),
            int: if integral { text.parse().ok() } else { None },
            float: text.parse::<f64>().ok().filter(|x| x.is_finite()),
        })
    }
}
