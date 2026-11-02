//! Finding the keys of a JSON line and where their values are, without parsing or
//! allocating anything (PRD §6.2, §14).

use std::borrow::Cow;
use std::ops::Range;

use memchr::memchr;

#[cfg(test)]
mod tests;

/// One key of a JSON object line and where its value sits in the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field<'a> {
    /// The key as written between its quotes, escapes not decoded.
    pub key: &'a [u8],
    /// Where the value sits in the line. For a string that is the text between its
    /// quotes, escapes not decoded; any other value is covered whole: `42`, `null`,
    /// `[1,2]`, `{"a":1}`.
    pub value: Range<usize>,
    /// Whether the value is a string.
    pub string: bool,
}

/// The fields of the object on `line`, one level deep and in the order they are
/// written; a repeated key comes once per occurrence. Whitespace may come before the
/// object and between any of its tokens. A line that isn't an object yields nothing.
///
/// Nothing is rejected outright: at the first byte that can't continue the object,
/// or where the line ends inside a value, the fields read so far are all there is.
/// Bytes inside strings are not checked, so invalid UTF-8 there does no harm.
pub fn fields(line: &[u8]) -> Fields<'_> {
    let start = skip_whitespace(line, 0);
    // `at` rests on the byte before the first key, as it does after every field.
    let at = if line.get(start) == Some(&b'{') {
        start
    } else {
        line.len()
    };
    Fields { line, at }
}

/// The first field of the object on `line` whose key is written exactly as `key`.
pub fn find<'a>(line: &'a [u8], key: &[u8]) -> Option<Field<'a>> {
    fields(line).find(|field| field.key == key)
}

/// The text between the quotes of a JSON string with its escapes decoded: `\"`,
/// `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t` and `\uXXXX` in either case of hex digits,
/// written in UTF-8, a surrogate pair of `\u` escapes as the one character it encodes.
/// A lone surrogate becomes U+FFFD. A backslash that starts no escape, such as `\x`,
/// a `\u` without four hex digits or a backslash at the end, is kept as written, and
/// so is every other byte, invalid UTF-8 included. Borrows `text` when it has no
/// backslash.
pub fn decode(text: &[u8]) -> Cow<'_, [u8]> {
    if memchr(b'\\', text).is_none() {
        return Cow::Borrowed(text);
    }
    let mut decoded = Vec::with_capacity(text.len());
    append_decoded(text, &mut decoded);
    Cow::Owned(decoded)
}

/// [`decode`] for a caller that decodes text after text: decoded text is written into
/// `buffer`, which keeps its allocation from one text to the next, and the result
/// borrows from there. Text without a backslash is returned as is.
pub fn decode_into<'a>(text: &'a [u8], buffer: &'a mut Vec<u8>) -> &'a [u8] {
    if memchr(b'\\', text).is_none() {
        return text;
    }
    buffer.clear();
    append_decoded(text, buffer);
    buffer
}

pub(super) fn append_decoded(text: &[u8], out: &mut Vec<u8>) {
    let mut rest = text;
    while let Some(backslash) = memchr(b'\\', rest) {
        out.extend_from_slice(&rest[..backslash]);
        rest = append_escape(&rest[backslash + 1..], out);
    }
    out.extend_from_slice(rest);
}

/// Appends what the escape that follows a backslash, `escaped`, stands for and returns
/// what follows the escape. When it isn't one, the backslash is appended instead and
/// `escaped` is returned whole.
fn append_escape<'a>(escaped: &'a [u8], out: &mut Vec<u8>) -> &'a [u8] {
    let simple = match escaped.first() {
        Some(b'"') => b'"',
        Some(b'\\') => b'\\',
        Some(b'/') => b'/',
        Some(b'b') => 0x08,
        Some(b'f') => 0x0c,
        Some(b'n') => b'\n',
        Some(b'r') => b'\r',
        Some(b't') => b'\t',
        Some(b'u') => return append_unicode_escape(escaped, out),
        _ => {
            out.push(b'\\');
            return escaped;
        }
    };
    out.push(simple);
    &escaped[1..]
}

/// [`append_escape`] for `escaped` that starts with the `u` of `\u`.
fn append_unicode_escape<'a>(escaped: &'a [u8], out: &mut Vec<u8>) -> &'a [u8] {
    let Some(unit) = hex_unit(&escaped[1..]) else {
        out.push(b'\\');
        return escaped;
    };
    let after = &escaped[5..];
    let low = after
        .strip_prefix(b"\\u")
        .and_then(hex_unit)
        .filter(|low| (0xDC00..0xE000).contains(low));
    let (c, used) = match (unit, low) {
        (0xD800..0xDC00, Some(low)) => {
            let pair = 0x10000 + ((u32::from(unit) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
            (char::from_u32(pair), 6)
        }
        _ => (char::from_u32(u32::from(unit)), 0),
    };
    let mut utf8 = [0; 4];
    let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
    out.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
    &after[used..]
}

/// The number the four hex digits that `text` starts with write.
fn hex_unit(text: &[u8]) -> Option<u16> {
    let digits = text.get(..4)?;
    digits.iter().try_fold(0u16, |unit, &digit| {
        let digit = char::from(digit).to_digit(16)?;
        Some(unit << 4 | digit as u16)
    })
}

impl Field<'_> {
    /// The value as text: a string decoded, anything else as written. `line` is the
    /// line the field was read from.
    pub fn decoded<'a>(&self, line: &'a [u8]) -> Cow<'a, [u8]> {
        let value = &line[self.value.clone()];
        if self.string {
            decode(value)
        } else {
            Cow::Borrowed(value)
        }
    }

    /// The value as text: a string decoded, written into `buffer` if it has escapes,
    /// anything else as written. `line` is the line the field was read from.
    pub fn text<'a>(&self, line: &'a [u8], buffer: &'a mut Vec<u8>) -> &'a [u8] {
        let value = &line[self.value.clone()];
        if self.string {
            decode_into(value, buffer)
        } else {
            value
        }
    }
}

/// The fields of one line, see [`fields`].
#[derive(Debug, Clone)]
pub struct Fields<'a> {
    line: &'a [u8],
    /// The `{` or `,` that precedes the next field; the line's length once done.
    at: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = Field<'a>;

    fn next(&mut self) -> Option<Field<'a>> {
        let field = self.read();
        if field.is_none() {
            self.at = self.line.len();
        }
        field
    }
}

impl<'a> Fields<'a> {
    fn read(&mut self) -> Option<Field<'a>> {
        let line = self.line;
        if self.at >= line.len() {
            return None;
        }
        let key_start = skip_whitespace(line, self.at + 1);
        if line.get(key_start) != Some(&b'"') {
            return None;
        }
        let key_end = string_end(line, key_start + 1)?;
        let colon = skip_whitespace(line, key_end + 1);
        if line.get(colon) != Some(&b':') {
            return None;
        }
        let value_start = skip_whitespace(line, colon + 1);
        let (value, string, value_end) = value_at(line, value_start)?;
        // The next byte must be the comma that continues the object, or the object
        // ends here.
        let after = skip_whitespace(line, value_end);
        self.at = if line.get(after) == Some(&b',') {
            after
        } else {
            line.len()
        };
        Some(Field {
            key: &line[key_start + 1..key_end],
            value,
            string,
        })
    }
}

fn skip_whitespace(line: &[u8], from: usize) -> usize {
    let rest = line.get(from..).unwrap_or_default();
    let blanks = rest
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .count();
    from + blanks
}

/// Where the string whose first byte after the opening quote is `from` closes.
pub(super) fn string_end(line: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        let found = at + memchr::memchr2(b'"', b'\\', line.get(at..)?)?;
        if line[found] == b'"' {
            return Some(found);
        }
        at = found + 2;
    }
}

/// The value starting at `start`: its range, whether it is a string, and where the
/// value (quotes included) ends. `None` when the line ends inside it.
fn value_at(line: &[u8], start: usize) -> Option<(Range<usize>, bool, usize)> {
    match *line.get(start)? {
        b'"' => {
            let end = string_end(line, start + 1)?;
            Some((start + 1..end, true, end + 1))
        }
        b'{' | b'[' => {
            let end = container_end(line, start)?;
            Some((start..end, false, end))
        }
        _ => {
            let rest = &line[start..];
            let len = rest.iter().position(|byte| {
                matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n')
            })?;
            (len > 0).then_some((start..start + len, false, start + len))
        }
    }
}

/// The end of the object or array opening at `start`. Depth is counted rather than
/// recursed into, so no nesting can overflow the stack.
fn container_end(line: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = start;
    while let Some(&byte) = line.get(at) {
        match byte {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            b'"' => at = string_end(line, at + 1)?,
            _ => {}
        }
        at += 1;
    }
    None
}
