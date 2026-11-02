//! Finding the keys of a logfmt line and where their values are, without parsing or
//! allocating anything (PRD §6.3, §14). The fields are the same as those of a JSON
//! line, so the [`Mapping`](super::Mapping), decoding and display are shared.

use std::ops::Range;

use memchr::{memchr, memchr2, memchr3};

use super::json::{self, Field};

#[cfg(test)]
mod tests;

/// The `key=value` pairs of `line`, in the order they are written; a repeated key
/// comes once per occurrence. Pairs are separated by spaces and tabs, as many as
/// there are, and any may come before the first pair and after the last.
///
/// - A key runs up to the first `=`, space or tab. A key in double quotes, as slog
///   writes a key with a space or an `=` in it, is the text between the quotes,
///   escapes not decoded.
/// - A value in double quotes is a [string](Field::string): its range is the text
///   between the quotes, escapes not decoded. A backslash escapes the byte after it,
///   so `\"` doesn't close the value; spaces and `=` inside the quotes are part of it.
///   Where the line ends before the closing quote, the value runs to the end of the
///   line.
/// - Any other value runs up to the next space or tab and is taken as written, `=`
///   and backslashes included. `key=` has an empty value.
/// - A key without `=` after it has an empty value that isn't a string.
///
/// Nothing is rejected: bytes that fit none of this, or aren't UTF-8, are read by
/// the same rules.
pub fn fields(line: &[u8]) -> Fields<'_> {
    Fields { line, at: 0 }
}

/// Whether `line` starts an entry: its first byte is the first of a bare key, which
/// is followed by its `=`. Indented pairs and text with a pair further on don't.
pub(super) fn starts_entry(line: &[u8]) -> bool {
    if line.first() == Some(&b'"') {
        return false;
    }
    memchr3(b'=', b' ', b'\t', line).is_some_and(|at| at > 0 && line[at] == b'=')
}

/// `line` with the escapes of its quoted values decoded, written into `buffer`. A line
/// without a backslash in a quoted value is returned as is.
pub(super) fn decode_values_into<'a>(line: &'a [u8], buffer: &'a mut Vec<u8>) -> &'a [u8] {
    if memchr(b'\\', line).is_none() {
        return line;
    }
    let mut quoted = fields(line).filter(|field| field.string);
    let Some(first) = quoted.find(|field| line[field.value.clone()].contains(&b'\\')) else {
        return line;
    };
    buffer.clear();
    let mut copied = 0;
    for field in std::iter::once(first).chain(quoted) {
        buffer.extend_from_slice(&line[copied..field.value.start]);
        json::append_decoded(&line[field.value.clone()], buffer);
        copied = field.value.end;
    }
    buffer.extend_from_slice(&line[copied..]);
    buffer
}

/// The fields of one line, see [`fields`].
#[derive(Debug, Clone)]
pub struct Fields<'a> {
    line: &'a [u8],
    /// Where the next field's search starts; the line's length once done.
    at: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = Field<'a>;

    fn next(&mut self) -> Option<Field<'a>> {
        let line = self.line;
        let start = self.at + skip_blanks(&line[self.at..]);
        if start >= line.len() {
            self.at = line.len();
            return None;
        }
        let (key, after_key) = if line[start] == b'"' {
            let end = json::string_end(line, start + 1).unwrap_or(line.len());
            (&line[start + 1..end], (end + 1).min(line.len()))
        } else {
            let end =
                memchr3(b'=', b' ', b'\t', &line[start..]).map_or(line.len(), |at| start + at);
            (&line[start..end], end)
        };
        let (value, string) = if line.get(after_key) == Some(&b'=') {
            self.value_from(after_key + 1)
        } else {
            self.at = after_key;
            (after_key..after_key, false)
        };
        Some(Field { key, value, string })
    }
}

impl Fields<'_> {
    /// The value that starts at `start`, with `self.at` moved past it.
    fn value_from(&mut self, start: usize) -> (Range<usize>, bool) {
        let line = self.line;
        if line.get(start) == Some(&b'"') {
            let end = json::string_end(line, start + 1).unwrap_or(line.len());
            self.at = (end + 1).min(line.len());
            (start + 1..end, true)
        } else {
            let end = memchr2(b' ', b'\t', &line[start..]).map_or(line.len(), |at| start + at);
            self.at = end;
            (start..end, false)
        }
    }
}

fn skip_blanks(text: &[u8]) -> usize {
    text.iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count()
}
