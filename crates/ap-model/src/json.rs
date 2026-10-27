//! A kotlinx `JsonElement` equivalent: number literals keep their exact text, objects keep
//! insertion order (a duplicate key replaces the value in place, like `LinkedHashMap`), and
//! `Display` prints compactly exactly like kotlinx `JsonElement.toString()`.

use std::fmt::{self, Write as _};

use indexmap::IndexMap;

/// A parsed JSON value with kotlinx tree semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonElement {
    Null,
    Bool(bool),
    /// The literal text, e.g. `1.0`, `1000`, `1e3`.
    Number(String),
    String(String),
    Array(Vec<JsonElement>),
    Object(IndexMap<String, JsonElement>),
}

/// A JSON syntax error with its byte offset.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid JSON at byte {offset}: {message}")]
pub struct JsonSyntaxError {
    pub offset: usize,
    pub message: &'static str,
}

/// Writes `s` as a JSON string literal with kotlinx `printQuoted` escaping: `"` `\\` and the
/// short escapes `\b \t \n \f \r`; other characters below U+0020 as `\u00xx` (lowercase hex);
/// everything else, including non-ASCII, verbatim.
pub fn write_quoted(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0C}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

impl JsonElement {
    /// Parses strict JSON (kotlinx `Json.parseToJsonElement` with default settings).
    pub fn parse(text: &str) -> Result<Self, JsonSyntaxError> {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        p.ws();
        let value = p.value(0)?;
        p.ws();
        if p.i != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(value)
    }

    #[must_use]
    pub fn as_object(&self) -> Option<&IndexMap<String, JsonElement>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[JsonElement]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// kotlinx `JsonPrimitive.contentOrNull`: string/number/boolean content; `None` for null and
    /// non-primitives.
    #[must_use]
    pub fn content_or_null(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            Self::String(s) | Self::Number(s) => Some(s.as_str().into()),
            Self::Bool(b) => Some(if *b { "true" } else { "false" }.into()),
            Self::Null | Self::Array(_) | Self::Object(_) => None,
        }
    }

    /// Android sync `JsonObject.number`: `longOrNull ?: doubleOrNull?.toLong()` of a primitive's
    /// content (strings included).
    #[must_use]
    pub fn as_long_lenient(&self) -> Option<i64> {
        let content = self.content_or_null()?;
        if let Ok(v) = content.parse::<i64>() {
            return Some(v);
        }
        // Kotlin `Double.toLong()` saturates and maps NaN to 0, like Rust `as`.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Kotlin Double.toLong semantics"
        )]
        content.parse::<f64>().ok().map(|d| d as i64)
    }

    /// Compact kotlinx text.
    pub fn write_to(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Number(n) => out.push_str(n),
            Self::String(s) => write_quoted(out, s),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_to(out);
                }
                out.push(']');
            }
            Self::Object(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_quoted(out, k);
                    out.push(':');
                    v.write_to(out);
                }
                out.push('}');
            }
        }
    }
}

impl fmt::Display for JsonElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        self.write_to(&mut out);
        f.write_str(&out)
    }
}

impl From<&serde_json::Value> for JsonElement {
    /// Numbers take serde_json's text (exact for integers; floats print shortest round-trip).
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => Self::Number(n.to_string()),
            serde_json::Value::String(s) => Self::String(s.clone()),
            serde_json::Value::Array(items) => Self::Array(items.iter().map(Self::from).collect()),
            serde_json::Value::Object(map) => Self::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), Self::from(v)))
                    .collect(),
            ),
        }
    }
}

const MAX_DEPTH: usize = 200;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, message: &'static str) -> JsonSyntaxError {
        JsonSyntaxError {
            offset: self.i,
            message,
        }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<JsonElement, JsonSyntaxError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.s.get(self.i) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(JsonElement::String),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) if self.eat(b"null") => Ok(JsonElement::Null),
            Some(_) if self.eat(b"true") => Ok(JsonElement::Bool(true)),
            Some(_) if self.eat(b"false") => Ok(JsonElement::Bool(false)),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<JsonElement, JsonSyntaxError> {
        self.i += 1;
        let mut map = IndexMap::new();
        self.ws();
        if self.eat(b"}") {
            return Ok(JsonElement::Object(map));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return Err(self.err("expected object key"));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(b":") {
                return Err(self.err("expected ':'"));
            }
            self.ws();
            let value = self.value(depth + 1)?;
            map.insert(key, value);
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"}") {
                return Ok(JsonElement::Object(map));
            }
            return Err(self.err("expected ',' or '}'"));
        }
    }

    fn array(&mut self, depth: usize) -> Result<JsonElement, JsonSyntaxError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b"]") {
            return Ok(JsonElement::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"]") {
                return Ok(JsonElement::Array(items));
            }
            return Err(self.err("expected ',' or ']'"));
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<JsonElement, JsonSyntaxError> {
        let start = self.i;
        self.eat(b"-");
        if self.digits() == 0 {
            return Err(self.err("expected digit"));
        }
        if self.eat(b".") && self.digits() == 0 {
            return Err(self.err("expected fraction digit"));
        }
        if let Some(b'e' | b'E') = self.s.get(self.i) {
            self.i += 1;
            if let Some(b'+' | b'-') = self.s.get(self.i) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("expected exponent digit"));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("utf-8"))?;
        Ok(JsonElement::Number(text.to_owned()))
    }

    fn hex4(&mut self) -> Result<u32, JsonSyntaxError> {
        let digits = self
            .s
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonSyntaxError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while let Some(&b) = self.s.get(self.i) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(
                std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("utf-8"))?,
            );
            match self.s.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let esc = *self
                        .s
                        .get(self.i)
                        .ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) && self.eat(b"\\u") {
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(code)
                                    .ok_or_else(|| self.err("unpaired surrogate"))?,
                            );
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_table_matches_kotlinx() {
        let mut out = String::new();
        let input: String = (0u8..0x20)
            .map(char::from)
            .chain("\"\\/\u{7f}é😀\u{2028}".chars())
            .collect();
        write_quoted(&mut out, &input);
        assert_eq!(
            out,
            "\"\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007\\b\\t\\n\\u000b\\f\\r\\u000e\\u000f\
             \\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f\
             \\\"\\\\/\u{7f}é😀\u{2028}\""
        );
        // serde_json output is identical, so typed DTOs can use it directly.
        assert_eq!(serde_json::to_string(&input).unwrap(), out);
    }

    #[test]
    fn numbers_keep_their_literal_text() {
        let v = JsonElement::parse(
            r#" {"a": 1.0, "b": 1e3, "c": -0, "d": [12345678901234567890123]} "#,
        )
        .unwrap();
        assert_eq!(
            v.to_string(),
            r#"{"a":1.0,"b":1e3,"c":-0,"d":[12345678901234567890123]}"#
        );
    }

    #[test]
    fn duplicate_keys_replace_in_place() {
        let v = JsonElement::parse(r#"{"a":1,"b":2,"a":3}"#).unwrap();
        assert_eq!(v.to_string(), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn strings_unescape() {
        let v = JsonElement::parse(r#""q\"\\\/\u00e9\ud83d\ude00\n""#).unwrap();
        assert_eq!(v, JsonElement::String("q\"\\/é😀\n".into()));
        assert!(JsonElement::parse("[1,]").is_err());
        assert!(JsonElement::parse("01x").is_err());
    }

    #[test]
    fn lenient_long_follows_kotlin() {
        let n = |s: &str| JsonElement::parse(s).unwrap().as_long_lenient();
        assert_eq!(n("3"), Some(3));
        assert_eq!(n("\"4\""), Some(4));
        assert_eq!(n("2.9"), Some(2));
        assert_eq!(n("null"), None);
        assert_eq!(n("true"), None);
        assert_eq!(n("{}"), None);
    }
}
