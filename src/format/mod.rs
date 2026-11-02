//! Understanding log lines: the formats and detecting which one a file is in,
//! severity levels, removing terminal escape codes before a line is parsed,
//! grouping lines into entries, reading their timestamps and dating them.

use std::borrow::Cow;

use memchr::{memchr, memchr2};

mod dates;
mod detect;
mod entry;
pub mod json;
pub mod logfmt;
mod mapping;
mod preset;
mod time;

pub(crate) use dates::DatesForward;
pub use dates::{Anchor, DatesBackward, midnights_between, resolve_timestamps};
pub use detect::detect;
pub use entry::{Entry, EntryGrouper, is_exception_line};
pub use mapping::{Mapping, Other, StructuredColumns, StructuredText, Syntax, decoded_line};
pub use preset::{BUILT_IN, Column, EntryHead, EntryParser, Format, Kind, Layout};
#[cfg(test)]
pub(crate) use preset::{EntryPattern, Pattern, text};
pub use time::LogTime;

#[cfg(test)]
mod tests;

/// The severity of a log entry, ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    /// Every level, from least to most severe.
    pub const ALL: [Level; 6] = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
        Level::Fatal,
    ];

    /// The level's name in capitals, as vilog shows it.
    pub fn name(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Fatal => "FATAL",
        }
    }

    /// Reads a level token as it appears in a log line, e.g. `INFO ` or `warning`.
    /// Surrounding spaces and tabs and letter case don't matter. Besides the canonical
    /// names it knows the aliases of the PRD (`WRN`, `SEVERE`, `CRITICAL`, ...). Anything
    /// else is `None`.
    pub fn parse(token: &[u8]) -> Option<Level> {
        const NAMES: &[(&[u8], Level)] = &[
            (b"TRACE", Level::Trace),
            (b"TRCE", Level::Trace),
            (b"VRB", Level::Trace),
            (b"VERBOSE", Level::Trace),
            (b"FINER", Level::Trace),
            (b"FINEST", Level::Trace),
            (b"DEBUG", Level::Debug),
            (b"DBG", Level::Debug),
            (b"DBUG", Level::Debug),
            (b"FINE", Level::Debug),
            (b"INFO", Level::Info),
            (b"INF", Level::Info),
            (b"INFORMATION", Level::Info),
            (b"NOTICE", Level::Info),
            (b"CONFIG", Level::Info),
            (b"WARN", Level::Warn),
            (b"WARNING", Level::Warn),
            (b"WRN", Level::Warn),
            (b"ERROR", Level::Error),
            (b"ERR", Level::Error),
            (b"FAIL", Level::Error),
            (b"SEVERE", Level::Error),
            (b"FATAL", Level::Fatal),
            (b"FTL", Level::Fatal),
            (b"CRITICAL", Level::Fatal),
            (b"CRIT", Level::Fatal),
            (b"PANIC", Level::Fatal),
            (b"EMERG", Level::Fatal),
            (b"ALERT", Level::Fatal),
        ];
        let name = trim_padding(token);
        NAMES
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|&(_, level)| level)
    }

    /// Reads a numeric level on the pino/bunyan scale: 10 is `Trace`, 20 `Debug`, and
    /// so on up to 60 for `Fatal`. Any other number is `None`.
    pub fn from_pino(number: u64) -> Option<Level> {
        match number {
            10 => Some(Level::Trace),
            20 => Some(Level::Debug),
            30 => Some(Level::Info),
            40 => Some(Level::Warn),
            50 => Some(Level::Error),
            60 => Some(Level::Fatal),
            _ => None,
        }
    }
}

/// `token` without leading and trailing spaces and tabs. Not `trim_ascii`: that would
/// also accept line breaks and form feeds as padding.
fn trim_padding(mut token: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = token {
        token = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = token {
        token = rest;
    }
    token
}

/// `line` without ANSI escape sequences: CSI (`ESC [ … final byte`), OSC
/// (`ESC ] … BEL` or `ESC ] … ESC \`) and other ESC sequences. As on a VT500, an ESC
/// inside an unfinished CSI or OSC abandons it and starts a new sequence. A sequence
/// cut off at the end of the line is dropped up to the end, a lone ESC is dropped.
/// Bytes from 0x80 up are left alone. Borrows `line` when there is nothing to remove.
pub fn strip_ansi(line: &[u8]) -> Cow<'_, [u8]> {
    if memchr(ESC, line).is_none() {
        return Cow::Borrowed(line);
    }
    let mut out = Vec::with_capacity(line.len());
    append_stripped(line, &mut out);
    Cow::Owned(out)
}

/// [`strip_ansi`] for a caller that strips line after line: a line with escape codes
/// is stripped into `buffer`, which keeps its allocation from one line to the next,
/// and the result borrows from there. A line without any is returned as is.
pub fn strip_ansi_into<'a>(line: &'a [u8], buffer: &'a mut Vec<u8>) -> &'a [u8] {
    if memchr(ESC, line).is_none() {
        return line;
    }
    buffer.clear();
    append_stripped(line, buffer);
    buffer
}

/// Appends `line` without its escape sequences to `out`.
fn append_stripped(line: &[u8], out: &mut Vec<u8>) {
    let mut rest = line;
    while let Some(esc) = memchr(ESC, rest) {
        let (text, sequence) = rest.split_at(esc);
        out.extend_from_slice(text);
        rest = after_escape(&sequence[1..]);
    }
    out.extend_from_slice(rest);
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// What follows the escape sequence whose ESC came just before `seq`.
fn after_escape(seq: &[u8]) -> &[u8] {
    match seq {
        [b'[', rest @ ..] => after_csi(rest),
        [b']', rest @ ..] => after_osc(rest),
        _ => after_plain_escape(seq),
    }
}

/// Parameter and intermediate bytes run up to the final byte, 0x40 to 0x7E.
fn after_csi(seq: &[u8]) -> &[u8] {
    let end = seq.iter().position(|b| matches!(*b, ESC | 0x40..=0x7e));
    rest_after(seq, end)
}

/// The text runs up to BEL or to the string terminator ESC `\`. Stopping at any ESC
/// is enough: `ESC \` is then removed like any other two-byte sequence.
fn after_osc(seq: &[u8]) -> &[u8] {
    rest_after(seq, memchr2(BEL, ESC, seq))
}

/// What follows a CSI or OSC in `seq` that ends at `end`. An ESC there doesn't belong
/// to it but starts the next sequence, so it stays.
fn rest_after(seq: &[u8], end: Option<usize>) -> &[u8] {
    match end {
        Some(esc) if seq[esc] == ESC => &seq[esc..],
        Some(last) => &seq[last + 1..],
        None => &[],
    }
}

/// Intermediate bytes 0x20 to 0x2F, then a final byte 0x30 to 0x7E. Any other byte
/// can't be part of the sequence, so it stays and only what came before it goes.
fn after_plain_escape(seq: &[u8]) -> &[u8] {
    let intermediates = seq.iter().take_while(|b| matches!(b, 0x20..=0x2f)).count();
    match seq.get(intermediates) {
        Some(0x30..=0x7e) => &seq[intermediates + 1..],
        Some(_) => &seq[intermediates..],
        None => &[],
    }
}
