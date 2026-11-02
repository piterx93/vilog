//! Which key of a JSON or logfmt line each column is read from (PRD §6.2, §6.3),
//! chosen once per file from the detection sample.

use super::detect::sample;
use super::json::{self, Field};
use super::logfmt;

pub use read::{Other, StructuredColumns, StructuredText};

mod read;
#[cfg(test)]
mod tests;

const TIMESTAMP: &[&str] = &[
    "@timestamp",
    "timestamp",
    "time",
    "ts",
    "@t",
    "Timestamp",
    "asctime",
];
const LEVEL: &[&str] = &[
    "level",
    "log.level",
    "levelname",
    "severity",
    "@l",
    "LogLevel",
];
const LOGGER: &[&str] = &[
    "logger_name",
    "logger",
    "log.logger",
    "name",
    "caller",
    "Category",
    "SourceContext",
];
const THREAD: &[&str] = &["thread_name", "thread", "process.thread.name"];
const MESSAGE: &[&str] = &["message", "msg", "event", "@m", "@mt", "Message", "log"];

/// Where the JSON object on `line` starts: its `{`, after an optional UTF-8 byte order
/// mark and any spaces and tabs. `None` for a line that isn't an object.
pub(super) fn object(line: &[u8]) -> Option<usize> {
    let bom = if line.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let blanks = line[bom..]
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    (line.get(bom + blanks) == Some(&b'{')).then_some(bom + blanks)
}

/// How the lines of a file write their fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    Json,
    Logfmt,
}

impl Syntax {
    /// Where the fields of `line` start if it starts an entry: for JSON the `{` of its
    /// object (see [`object`]), for logfmt the line's start (see
    /// [`logfmt::fields`]).
    pub(super) fn start(self, line: &[u8]) -> Option<usize> {
        match self {
            Syntax::Json => object(line),
            Syntax::Logfmt => logfmt::starts_entry(line).then_some(0),
        }
    }

    /// The fields of `line`, which starts where [`start`](Self::start) says.
    pub(super) fn fields(self, line: &[u8]) -> Pairs<'_> {
        match self {
            Syntax::Json => Pairs::Json(json::fields(line)),
            Syntax::Logfmt => Pairs::Logfmt(logfmt::fields(line)),
        }
    }
}

pub(super) enum Pairs<'a> {
    Json(json::Fields<'a>),
    Logfmt(logfmt::Fields<'a>),
}

impl<'a> Iterator for Pairs<'a> {
    type Item = Field<'a>;

    fn next(&mut self) -> Option<Field<'a>> {
        match self {
            Pairs::Json(fields) => fields.next(),
            Pairs::Logfmt(fields) => fields.next(),
        }
    }
}

/// `line` as search reads it: an entry's first line with the escapes of its strings
/// decoded, into `buffer` if it has any. A logfmt line has them in quoted values only.
/// Any other line is returned as is.
pub fn decoded_line<'a>(syntax: Syntax, line: &'a [u8], buffer: &'a mut Vec<u8>) -> &'a [u8] {
    match syntax {
        Syntax::Json if object(line).is_some() => json::decode_into(line, buffer),
        Syntax::Logfmt if logfmt::starts_entry(line) => logfmt::decode_values_into(line, buffer),
        _ => line,
    }
}

/// The field `key` names on the object `line`: the key written literally, else, for
/// a dotted key, the part after the first dot inside the object under the part
/// before it. The value range is into `line`.
pub(super) fn lookup<'a>(line: &'a [u8], key: &str) -> Option<Field<'a>> {
    if let Some(field) = json::find(line, key.as_bytes()) {
        return Some(field);
    }
    let (outer, inner) = key.split_once('.')?;
    let outer = json::find(line, outer.as_bytes()).filter(|field| !field.string)?;
    let start = outer.value.start;
    let mut field = json::find(&line[outer.value], inner.as_bytes())?;
    field.value = field.value.start + start..field.value.end + start;
    Some(field)
}

/// [`lookup`] for a line in `syntax`; a logfmt key is only ever the literal one.
pub(super) fn lookup_in<'a>(syntax: Syntax, line: &'a [u8], key: &str) -> Option<Field<'a>> {
    match syntax {
        Syntax::Json => lookup(line, key),
        Syntax::Logfmt => logfmt::fields(line).find(|field| field.key == key.as_bytes()),
    }
}

/// Whether `line` is a logfmt line detection counts (PRD §6.3): it starts an entry and
/// has a timestamp or level key and a message key. Keys are literal and case-sensitive.
pub(super) fn is_logfmt_log_line(line: &[u8]) -> bool {
    if !logfmt::starts_entry(line) {
        return false;
    }
    let (mut when, mut message) = (false, false);
    for field in logfmt::fields(line) {
        let key = field.key;
        let is = |candidates: &[&str]| candidates.iter().any(|name| name.as_bytes() == key);
        when = when || is(TIMESTAMP) || is(LEVEL);
        message = message || is(MESSAGE);
        if when && message {
            return true;
        }
    }
    false
}

/// The key each column of a JSON lines or logfmt file is read from. A column without a
/// key is `None`; without a message key the whole line is the message. A key is one of the
/// column's candidates as PRD §6.2 lists it, e.g. `log.level`, which a line may write
/// as a literal dotted key or one object deep.
///
/// The exception has no key here: exceptions are rare, so no exception key would be on
/// half of the lines. A line's exception is the first exception candidate it has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mapping {
    pub timestamp: Option<&'static str>,
    pub level: Option<&'static str>,
    pub logger: Option<&'static str>,
    pub thread: Option<&'static str>,
    pub message: Option<&'static str>,
}

impl Mapping {
    /// The mapping of the file whose lines in order, without line endings, are
    /// `lines`, chosen from the sample [`detect`](super::detect) reads from them: for
    /// each column the first of its candidates that is present in at least half of
    /// the sample's JSON and logfmt lines. A JSON line starts with `{`; a UTF-8 byte
    /// order mark before the first line is skipped. A logfmt line starts with a key
    /// and its `=` (see [`logfmt::fields`](super::logfmt::fields)), and a dotted
    /// candidate matches only the literal dotted key there. Other lines, such as a
    /// banner, don't count. No line past the sample is read.
    pub fn choose<'a>(lines: impl IntoIterator<Item = &'a [u8]>) -> Mapping {
        let sample = sample(lines);
        let lines: Vec<(Syntax, &[u8])> = sample
            .iter()
            .filter_map(|line| {
                [Syntax::Json, Syntax::Logfmt]
                    .into_iter()
                    .find_map(|syntax| Some((syntax, &line[syntax.start(line)?..])))
            })
            .collect();
        let timestamp = first_on_half(TIMESTAMP, &lines);
        // CLEF leaves `@l` out of Information lines, so it can't be held to the half.
        let level = if timestamp == Some("@t") {
            Some("@l")
        } else {
            first_on_half(LEVEL, &lines)
        };
        Mapping {
            timestamp,
            level,
            logger: first_on_half(LOGGER, &lines),
            thread: first_on_half(THREAD, &lines),
            message: first_on_half(MESSAGE, &lines),
        }
    }
}

fn first_on_half(candidates: &[&'static str], lines: &[(Syntax, &[u8])]) -> Option<&'static str> {
    candidates.iter().copied().find(|key| {
        let present = lines
            .iter()
            .filter(|(syntax, line)| lookup_in(*syntax, line, key).is_some());
        // Doubled so that exactly half counts; no lines at all count for nothing.
        !lines.is_empty() && present.count() * 2 >= lines.len()
    })
}
