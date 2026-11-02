//! Reading one JSON or logfmt line's columns with a [`Mapping`], in a single pass over
//! its fields.

use std::borrow::Cow;
use std::ops::Range;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::{Mapping, Syntax};
use crate::format::json::{self, Field};
use crate::format::{Level, LogTime, time};

/// The exception candidates of PRD §6.2. They are not part of the [`Mapping`]: a line's
/// exception is the first of them it has a value for.
const EXCEPTION: &[&str] = &[
    "stack_trace",
    "error.stack_trace",
    "stacktrace",
    "exception",
    "exc_info",
    "@x",
    "Exception",
    "stack",
    "err.stack",
];

/// The text of a JSON or logfmt line that the viewer shows and the text filter reads. Values are
/// fields whose ranges are into the line; a string is still to be
/// [decoded](json::decode).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StructuredText<'a> {
    pub logger: Option<Field<'a>>,
    pub thread: Option<Field<'a>>,
    pub message: Option<Field<'a>>,
    /// The line's exception: its first candidate, in the order of PRD §6.2, that has
    /// a value.
    pub exception: Option<Field<'a>>,
}

/// A field of a JSON or logfmt line that the details show as `key: value`.
#[derive(Debug, PartialEq, Eq)]
pub struct Other<'a> {
    /// The key as written; inside an object that holds a column, the outer key, a
    /// dot and the inner key.
    pub key: Cow<'a, [u8]>,
    pub value: Field<'a>,
}

impl<'a> StructuredText<'a> {
    /// The fields of `line`, the line [`Mapping::text`] read this from in `syntax`,
    /// that are none of the columns above, in file order. In JSON an object that holds
    /// one of them, such as pino's `err` around `err.stack`, is replaced by its other
    /// fields.
    pub fn others(&self, syntax: Syntax, line: &'a [u8]) -> Vec<Other<'a>> {
        let taken: Vec<&Range<usize>> =
            [&self.logger, &self.thread, &self.message, &self.exception]
                .into_iter()
                .flatten()
                .map(|field| &field.value)
                .collect();
        let mut others = Vec::new();
        let Some(at) = syntax.start(line) else {
            return others;
        };
        for field in syntax.fields(&line[at..]).map(|field| shifted(field, at)) {
            if taken.contains(&&field.value) {
                continue;
            }
            let holder = syntax == Syntax::Json
                && !field.string
                && taken.iter().any(|range| contains(&field.value, range));
            if !holder {
                others.push(Other {
                    key: Cow::Borrowed(field.key),
                    value: field,
                });
                continue;
            }
            for inner in json::fields(&line[field.value.clone()]) {
                let inner = shifted(inner, field.value.start);
                if !taken.contains(&&inner.value) {
                    let key = [field.key, b".", inner.key].concat();
                    others.push(Other {
                        key: Cow::Owned(key),
                        value: inner,
                    });
                }
            }
        }
        others
    }
}

fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// `field` of an object that starts at `at` in the line, as a field of the line.
fn shifted(mut field: Field<'_>, at: usize) -> Field<'_> {
    field.value = field.value.start + at..field.value.end + at;
    field
}

/// What the index keeps of a JSON or logfmt line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StructuredColumns {
    pub timestamp: Option<Timestamp>,
    pub level: Option<Level>,
    /// The logger's text between its quotes, as a byte range into the line.
    pub logger: Option<Range<usize>>,
    pub has_exception: bool,
}

impl Mapping {
    /// The text columns of `line`, an entry's first line in `syntax` (a JSON object
    /// line, or a logfmt line that starts with a key and its `=`); `None` for anything
    /// else. Columns are found as [`read`](Self::read) finds them.
    pub fn text<'a>(&self, syntax: Syntax, line: &'a [u8]) -> Option<StructuredText<'a>> {
        let at = syntax.start(line)?;
        let body = &line[at..];
        let mut text = StructuredText::default();
        // The better a candidate ranks, the earlier it is in the list.
        let mut rank = EXCEPTION.len();
        for field in syntax.fields(body) {
            let pick = |key: Option<&str>| resolve(syntax, body, &field, key?);
            text.logger = text.logger.or_else(|| pick(self.logger));
            text.thread = text.thread.or_else(|| pick(self.thread));
            text.message = text.message.or_else(|| pick(self.message));
            let better = EXCEPTION[..rank].iter().enumerate().find_map(|(i, key)| {
                let found = resolve(syntax, body, &field, key)?;
                has_value(syntax, body, &found).then_some((i, found))
            });
            if let Some((better, found)) = better {
                (rank, text.exception) = (better, Some(found));
            }
        }
        for field in [
            &mut text.logger,
            &mut text.thread,
            &mut text.message,
            &mut text.exception,
        ] {
            *field = field.take().map(|found| shifted(found, at));
        }
        Some(text)
    }

    /// The columns of `line`, an entry's first line in `syntax` (see
    /// [`text`](Self::text); anything else reads as no columns). Timestamp strings are
    /// read as [`time::read`] reads them, in `zone` if they have no offset, and
    /// unquoted numbers as epoch time; a quoted number is a string. In logfmt an
    /// unquoted value that isn't a number may be a timestamp too, and the logger may
    /// be unquoted. JSON levels are strings or numbers on the pino scale, logfmt
    /// levels are names. A line of a CLEF file (one that maps `@t`) without a level is
    /// Information.
    pub fn read(&self, syntax: Syntax, line: &[u8], zone: &TimeZone) -> StructuredColumns {
        let Some(at) = syntax.start(line) else {
            return StructuredColumns::default();
        };
        let body = &line[at..];
        let (mut timestamp, mut level, mut logger) = (None, None, None);
        let mut has_exception = false;
        for field in syntax.fields(body) {
            let pick = |key: Option<&str>| resolve(syntax, body, &field, key?);
            timestamp = timestamp.or_else(|| pick(self.timestamp));
            level = level.or_else(|| pick(self.level));
            logger = logger.or_else(|| pick(self.logger));
            has_exception = has_exception || is_exception(syntax, body, &field);
        }
        let clef = syntax == Syntax::Json && self.timestamp == Some("@t");
        StructuredColumns {
            timestamp: timestamp.and_then(|field| timestamp_of(syntax, body, &field, zone)),
            level: match level {
                Some(field) => level_of(syntax, body, &field),
                None => clef.then_some(Level::Info),
            },
            logger: logger
                .filter(|field| field.string || syntax == Syntax::Logfmt)
                .map(|field| field.value.start + at..field.value.end + at),
            has_exception,
        }
    }
}

/// `field` as the value of `key`, if it is: the field written with that very key or,
/// for a dotted key in JSON, the path one object deep. Ranges are into `body`.
fn resolve<'a>(syntax: Syntax, body: &'a [u8], field: &Field<'a>, key: &str) -> Option<Field<'a>> {
    let key = key.as_bytes();
    if field.key == key {
        return Some(field.clone());
    }
    if syntax != Syntax::Json {
        return None;
    }
    let inner = key.strip_prefix(field.key)?.strip_prefix(b".")?;
    if field.string {
        return None;
    }
    let start = field.value.start;
    let mut nested = json::find(&body[field.value.clone()], inner)?;
    nested.value = nested.value.start + start..nested.value.end + start;
    Some(nested)
}

fn is_exception(syntax: Syntax, body: &[u8], field: &Field<'_>) -> bool {
    EXCEPTION.iter().any(|key| {
        resolve(syntax, body, field, key).is_some_and(|found| has_value(syntax, body, &found))
    })
}

/// Whether `found` holds an exception. Nothing holds none; in JSON `null` and an
/// empty container hold none either.
fn has_value(syntax: Syntax, body: &[u8], found: &Field<'_>) -> bool {
    let value = &body[found.value.clone()];
    if found.string || syntax == Syntax::Logfmt {
        !value.is_empty()
    } else {
        !matches!(value, b"null" | b"{}" | b"[]")
    }
}

fn timestamp_of(
    syntax: Syntax,
    body: &[u8],
    field: &Field<'_>,
    zone: &TimeZone,
) -> Option<Timestamp> {
    let value = &body[field.value.clone()];
    let text = || match time::read(value, zone)? {
        LogTime::Instant(instant) => Some(instant),
        LogTime::TimeOfDay(_) => None,
    };
    match (field.string, syntax) {
        (true, _) => text(),
        (false, Syntax::Json) => time::epoch(value),
        // slog writes its time without quotes.
        (false, Syntax::Logfmt) => time::epoch(value).or_else(text),
    }
}

fn level_of(syntax: Syntax, body: &[u8], field: &Field<'_>) -> Option<Level> {
    let value = &body[field.value.clone()];
    if field.string || syntax == Syntax::Logfmt {
        Level::parse(value)
    } else {
        Level::from_pino(str::from_utf8(value).ok()?.parse().ok()?)
    }
}
