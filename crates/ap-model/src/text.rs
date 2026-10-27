//! Kotlin/JVM string semantics needed for byte-compatible ports.

use std::cmp::Ordering;

/// Kotlin `Char.isWhitespace()` on the JVM: `Character.isWhitespace || Character.isSpaceChar`.
///
/// Differs from Rust's `char::is_whitespace` in two places: U+001C..U+001F are whitespace
/// (Java information separators) and U+0085 (NEL) is not.
#[must_use]
pub fn is_kotlin_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t'..='\r'
            | '\u{1C}'..='\u{1F}'
            | ' '
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// Kotlin `String.trim()`.
#[must_use]
pub fn kotlin_trim(s: &str) -> &str {
    s.trim_matches(is_kotlin_whitespace)
}

/// Kotlin `String.isBlank()`.
#[must_use]
pub fn kotlin_is_blank(s: &str) -> bool {
    s.chars().all(is_kotlin_whitespace)
}

/// Kotlin/Java `String.compareTo`: lexicographic over UTF-16 code units.
///
/// Rust's `str` ordering is by code point, which differs for supplementary characters
/// versus U+E000..U+FFFF.
#[must_use]
pub fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Lowercase hex of `bytes` (Kotlin `"%02x".format` joined).
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0xf)]));
    }
    out
}

/// `sha256` of the UTF-8 bytes of `text`, lowercase hex.
#[must_use]
pub fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex_lower(&Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_matches_kotlin_whitespace_set() {
        assert_eq!(kotlin_trim("\u{1F} a \u{A0}\u{3000}"), "a");
        assert_eq!(kotlin_trim("\u{85}a\u{85}"), "\u{85}a\u{85}");
        assert!(kotlin_is_blank(" \t\u{2007}"));
        assert!(!kotlin_is_blank(" x "));
    }

    #[test]
    fn utf16_order_differs_from_code_point_order() {
        // U+1F600 (surrogates D83D DE00) sorts before U+FF21 in UTF-16.
        assert_eq!(cmp_utf16("\u{1F600}", "\u{FF21}"), Ordering::Less);
        assert_eq!("\u{1F600}".cmp("\u{FF21}"), Ordering::Greater);
    }

    #[test]
    fn sha256_of_empty_string() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
