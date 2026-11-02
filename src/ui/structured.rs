//! JSON lines and logfmt as shown: values decoded, the message split into lines, the
//! fields that are no column listed as `key: value`.

use std::borrow::Cow;

use memchr::memchr;

use super::details::source_line;
use super::table::text;
use crate::format::json::{self, Field};
use crate::format::{StructuredText, Syntax};

/// The value of `field` made safe to show, nothing for no field.
pub(super) fn column(line: &[u8], field: Option<&Field<'_>>) -> String {
    field.map_or_else(String::new, |field| text(&field.decoded(line)))
}

/// What the table's row shows of the message of `line`: its decoded first line. A
/// line without a message field is its own message, as written.
pub(super) fn row_message<'a>(line: &'a [u8], columns: &StructuredText<'_>) -> Cow<'a, [u8]> {
    let Some(message) = &columns.message else {
        return Cow::Borrowed(line);
    };
    let end = |bytes: &[u8]| memchr(b'\n', bytes).unwrap_or(bytes.len());
    match message.decoded(line) {
        Cow::Borrowed(bytes) => Cow::Borrowed(&bytes[..end(bytes)]),
        Cow::Owned(mut bytes) => {
            bytes.truncate(end(&bytes));
            Cow::Owned(bytes)
        }
    }
}

/// The lines of the details pane for the JSON or logfmt line `line`, whose columns are
/// `columns`: where it was logged, the message, the exception, then the other fields.
pub(super) fn details(syntax: Syntax, line: &[u8], columns: &StructuredText<'_>) -> Vec<String> {
    let optional =
        |field: &Option<Field<'_>>| field.as_ref().map(|field| text(&field.decoded(line)));
    let mut shown = Vec::new();
    shown.extend(source_line(
        optional(&columns.logger),
        optional(&columns.thread),
    ));
    match &columns.message {
        Some(message) => shown.extend(text_lines(&message.decoded(line))),
        None => shown.push(text(line)),
    }
    if let Some(exception) = &columns.exception {
        shown.extend(text_lines(&exception.decoded(line)));
    }
    for other in columns.others(syntax, line) {
        let key = json::decode(&other.key);
        let value = other.value.decoded(line);
        shown.push(format!("{}: {}", text(&key), text(&value)));
    }
    shown
}

/// `bytes` split at line breaks and made safe to show. A last break ends the last
/// line instead of starting an empty one.
fn text_lines(bytes: &[u8]) -> Vec<String> {
    let mut lines: Vec<&[u8]> = bytes.split(|&byte| byte == b'\n').collect();
    if lines.len() > 1 && lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    lines.into_iter().map(text).collect()
}
