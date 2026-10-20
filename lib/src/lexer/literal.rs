//! Canonical decoding of bodies already recognized as Rock quoted literals.
//! Unknown string escapes stay verbatim; this helper does not recognize quotes.

use std::ops::Range;

pub(crate) fn decoded_units(body: &str) -> impl Iterator<Item = (char, Range<usize>)> + '_ {
    let mut chars = body.char_indices().peekable();
    std::iter::from_fn(move || {
        let (start, character) = chars.next()?;
        if character == '\\' {
            let escaped = chars.peek().and_then(|(_, next)| match next {
                'n' => Some('\n'),
                't' => Some('\t'),
                'r' => Some('\r'),
                '0' => Some('\0'),
                '\\' => Some('\\'),
                '"' => Some('"'),
                '\'' => Some('\''),
                _ => None,
            });
            if let Some(escaped) = escaped {
                chars.next();
                return Some((escaped, start..start + 2));
            }
        }
        Some((character, start..start + character.len_utf8()))
    })
}

pub(crate) fn decode_string(body: &str) -> String {
    decoded_units(body).map(|(character, _)| character).collect()
}

pub(crate) fn decode_char(body: &str) -> Option<char> {
    let mut units = decoded_units(body);
    let (value, _) = units.next()?;
    units.next().is_none().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_b_18_canonical_decoding_preserves_escape_and_unicode_semantics() {
        let body = "é\\n中\\t\\r\\0\\\\\\\"\\'\\q\\雪\\";
        assert_eq!(decode_string(body), "é\n中\t\r\0\\\"'\\q\\雪\\");
        let units: Vec<_> = decoded_units(body).collect();
        assert_eq!(&units[..3], &[('é', 0..2), ('\n', 2..4), ('中', 4..7)]);
        for (value, range) in units {
            assert!(body.get(range.clone()).is_some());
            assert_eq!(decode_string(&body[range]), value.to_string());
        }
        for (body, value) in [("🦀", '🦀'), ("\\n", '\n'), ("\\'", '\''), ("\\", '\\')] {
            assert_eq!(decode_char(body), Some(value));
        }
        for body in ["", "ab", "\\q", "\\雪"] { assert_eq!(decode_char(body), None); }
    }
}
