//! The entry index: one compact record per log entry, built from the mapped file in
//! chunks on background threads.
//!
//! Indexing is split so that the threading stays thin: [`Chunk::index`] indexes the
//! lines that start in one byte range of the file, [`Chunk::join`] puts neighbouring
//! chunks together, and [`Chunk::finish`] turns the whole file's chunk into records.
//! Any chunking gives the same records as one chunk over the whole file. [`Indexer`]
//! runs this on threads and hands the records over as they are found. [`tail`] reads
//! the last entries from the end of the file, for showing them before the index is
//! complete. [`append`] keeps the records up to date as the file grows.

use std::mem;
use std::ops::Range;

use jiff::Timestamp;
use jiff::civil::Time;
use jiff::tz::TimeZone;
use memchr::memchr;

use crate::format::{
    Anchor, DatesBackward, EntryGrouper, EntryHead, EntryParser, Format, Level, LogTime, Mapping,
    StructuredColumns, Syntax, strip_ansi_into,
};
use crate::source;

mod append;
mod indexer;
mod tail;

pub use append::append;
pub use indexer::{Indexer, Progress};
pub use tail::tail;

#[cfg(test)]
mod tests;

/// How to read a file's entries.
#[derive(Debug, Clone)]
pub struct Settings {
    pub format: &'static Format,
    /// The time zone of timestamps without an offset.
    pub zone: TimeZone,
    /// Where dating a time-only preset's entries starts (see
    /// [`Anchor::for_file`]). Other presets don't use it.
    pub anchor: Anchor,
    /// Which keys a JSON lines or logfmt file's columns are read from, as
    /// [`Mapping::choose`] picks them from the file. Other formats don't use it.
    pub mapping: Mapping,
}

/// One entry of the index, at most 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    offset: U48,
    end: U48,
    /// The line the entry starts on, counted from 1 within its chunk until the chunk
    /// is joined to the ones before it; [`UNKNOWN_LINE`] in a tail.
    line: U48,
    /// Microseconds since the Unix epoch, or [`NO_TIMESTAMP`].
    timestamp: i64,
    /// The class span; `0..0` for none, as a class column is never empty.
    class_start: u16,
    class_end: u16,
    level: Option<Level>,
    has_exception: bool,
}

/// [`Record::timestamp`] when there is none: further back than jiff reaches, so no
/// timestamp is this many microseconds.
const NO_TIMESTAMP: i64 = i64::MIN;

/// [`Record::line`] when it isn't known, as lines count from 1.
const UNKNOWN_LINE: U48 = U48([0; 6]);

/// A byte offset or a line number in 6 bytes, which keeps a [`Record`] at 32. No file
/// is mapped past 2^48 bytes (256 TiB), the most a 64-bit address space maps, so no
/// offset or line number reaches that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct U48([u8; 6]);

impl U48 {
    fn new(value: u64) -> Self {
        debug_assert!(value >> 48 == 0, "{value} doesn't fit 48 bits");
        let [bytes @ .., _, _] = value.to_le_bytes();
        U48(bytes)
    }

    fn get(self) -> u64 {
        let mut bytes = [0; 8];
        bytes[..6].copy_from_slice(&self.0);
        u64::from_le_bytes(bytes)
    }
}

impl Record {
    /// The record of an entry that starts at `offset` and has no columns: the
    /// headless entry, or any entry under `Raw`.
    fn new(offset: u64) -> Record {
        Record {
            offset: U48::new(offset),
            end: U48::new(offset),
            line: UNKNOWN_LINE,
            timestamp: NO_TIMESTAMP,
            class_start: 0,
            class_end: 0,
            level: None,
            has_exception: false,
        }
    }

    /// Keeps `class` unless it reaches past what 16 bits hold.
    fn set_class(&mut self, class: Option<&Range<usize>>) {
        if let Some(class) = class
            && let (Ok(start), Ok(end)) = (u16::try_from(class.start), u16::try_from(class.end))
        {
            (self.class_start, self.class_end) = (start, end);
        }
    }

    /// Where the entry is in the file: from the first byte of its first line to the
    /// first byte of the next entry, or to the end of the file. The line endings are
    /// part of it, so the entries of a file cover it without gaps.
    pub fn bytes(&self) -> Range<u64> {
        self.offset.get()..self.end.get()
    }

    /// The entry's timestamp as [`resolve_timestamps`] resolves it: an entry whose
    /// first line has no readable timestamp takes the one of the entry before it, and
    /// a time-only preset's times of day get dates counted back from the anchor.
    /// Kept to the microsecond, truncated as [`Timestamp::as_microsecond`] does.
    /// `None` for the entries before the first timestamp and for every entry under
    /// `Raw`.
    ///
    /// [`resolve_timestamps`]: crate::format::resolve_timestamps
    pub fn timestamp(&self) -> Option<Timestamp> {
        // `NO_TIMESTAMP` lies outside jiff's range, so it comes out as `None`.
        Timestamp::from_microsecond(self.timestamp).ok()
    }

    /// The level its level column names, `None` for a word [`Level::parse`] doesn't
    /// know, for the headless entry and under `Raw`.
    pub fn level(&self) -> Option<Level> {
        self.level
    }

    /// Whether the entry has an exception, as [`Entry::has_exception`] says.
    ///
    /// [`Entry::has_exception`]: crate::format::Entry::has_exception
    pub fn has_exception(&self) -> bool {
        self.has_exception
    }

    /// The class column of the entry's first line, as a byte range into that line
    /// once ANSI escape codes are stripped ([`EntryHead::logger`]). `None` for the
    /// headless entry, under `Raw`, for a format without a logger, and when the span reaches past byte 65 535 of the
    /// line (`end > 65_535`), which is too far into the line to be kept here.
    ///
    /// [`EntryHead::logger`]: crate::format::EntryHead::logger
    pub fn class(&self) -> Option<Range<usize>> {
        (self.class_start < self.class_end)
            .then(|| usize::from(self.class_start)..usize::from(self.class_end))
    }

    /// The line of the file the entry starts on, counted from 1 as lines are split by
    /// [`crate::source::lines`]; its continuation lines are the lines after it.
    /// `None` for the records [`tail`] reads from the end of the file, as the lines
    /// before them aren't counted.
    pub fn line(&self) -> Option<u64> {
        (self.line != UNKNOWN_LINE).then(|| self.line.get())
    }
}

/// The entries of a run of whole lines, ready to be joined with the chunks around it.
#[derive(Debug)]
pub struct Chunk {
    records: Vec<Record>,
    /// Whether the first record is no entry of its own but the lines before the
    /// chunk's first entry start, which continue the entry before the chunk.
    leads_in: bool,
    /// Under a time-only preset, the time of day of each entry, which can only be
    /// dated once the whole file is known. Empty under other presets.
    times: Vec<Option<Time>>,
    /// How many lines start in the chunk.
    lines: u64,
}

impl Chunk {
    /// Indexes the lines of `bytes` whose first byte lies in `range`, reading each
    /// of them to its end even past `range`. `bytes` starts at the start of a line,
    /// at offset `base` in the file; lines are split as [`crate::source::lines`]
    /// splits them. `range` lies within `bytes`.
    ///
    /// Lines before the first entry start in `range` continue the last entry of the
    /// chunk before; without one they are the file's headless entry.
    pub fn index(bytes: &[u8], base: u64, range: Range<usize>, settings: &Settings) -> Chunk {
        let mut reader = ChunkReader::new(settings);
        let mut at = first_line_start(bytes, &range);
        let mut lines = source::lines(&bytes[at..]);
        while at < range.end
            && let Some(line) = lines.next()
        {
            reader.push(base + at as u64, line);
            at = bytes.len() - lines.rest().len();
        }
        reader.finish(base + at as u64)
    }

    /// Appends `next`, the chunk of the range right after this chunk's.
    pub fn join(&mut self, mut next: Chunk) {
        next.shift_lines(self.lines);
        if self.records.is_empty() {
            *self = next;
            return;
        }
        let continued = usize::from(next.leads_in);
        self.times.extend(next.times.iter().skip(continued));
        attach(&mut self.records, &next.records, next.leads_in);
        self.lines = next.lines;
    }

    /// The records of every entry in the chunk, in file order, once it covers the
    /// whole file: the last entry ends with the chunk and time-only presets are dated.
    pub fn finish(self, settings: &Settings) -> Vec<Record> {
        let mut records = self.records;
        set_timestamps(&mut records, &dated(&self.times, settings));
        records
    }

    fn empty() -> Chunk {
        Chunk {
            records: Vec::new(),
            leads_in: false,
            times: Vec::new(),
            lines: 0,
        }
    }

    /// Numbers the lines as if `lines` more lines came before the chunk.
    fn shift_lines(&mut self, lines: u64) {
        for record in &mut self.records {
            record.line = U48::new(record.line.get() + lines);
        }
        self.lines += lines;
    }

    /// Takes out the records that chunks joined later can't change: all but the last,
    /// which their leading lines may extend. The times stay, as dating needs all of
    /// them, so [`finish`](Self::finish) can't be used afterwards.
    fn take_closed(&mut self) -> Vec<Record> {
        let mut closed = mem::take(&mut self.records);
        self.records.extend(closed.pop());
        closed
    }

    /// Ends the last entry at `end`.
    fn close(&mut self, end: u64, has_exception: bool) {
        if let Some(last) = self.records.last_mut() {
            last.end = U48::new(end);
            last.has_exception |= has_exception;
        }
    }
}

/// Appends `next`, the records that follow `records` in the file, and carries the
/// last timestamp of `records` forward into them. With `leads_in`, the first of
/// `next` is no entry but the end of the last of `records`, if there is one.
fn attach(records: &mut Vec<Record>, mut next: &[Record], leads_in: bool) {
    if leads_in
        && let Some(last) = records.last_mut()
        && let Some((lead, rest)) = next.split_first()
    {
        last.end = lead.end;
        last.has_exception |= lead.has_exception;
        next = rest;
    }
    let previous = records.last().map_or(NO_TIMESTAMP, |last| last.timestamp);
    let joined = records.len();
    records.extend_from_slice(next);
    carry_forward(
        records[joined..]
            .iter_mut()
            .map(|record| &mut record.timestamp),
        previous,
    );
}

/// Where the first line that starts in `range` starts, `range.end` if none does.
fn first_line_start(bytes: &[u8], range: &Range<usize>) -> usize {
    let Some(before) = range.start.checked_sub(1) else {
        return 0;
    };
    memchr(b'\n', &bytes[before..range.end]).map_or(range.end, |newline| before + newline + 1)
}

/// Groups the lines of one chunk into entries and keeps a record of each.
struct ChunkReader<'a> {
    zone: &'a TimeZone,
    /// How a JSON lines or logfmt file is read, whose entries carry their columns in
    /// fields the grouper doesn't find.
    structured: Option<(Syntax, &'a Mapping)>,
    parser: Option<EntryParser>,
    time_only: bool,
    grouper: EntryGrouper,
    /// Reused for every line with escape codes.
    stripped: Vec<u8>,
    chunk: Chunk,
}

impl<'a> ChunkReader<'a> {
    fn new(settings: &'a Settings) -> Self {
        let parser = settings.format.entry_parser();
        ChunkReader {
            zone: &settings.zone,
            structured: settings
                .format
                .kind
                .syntax()
                .map(|syntax| (syntax, &settings.mapping)),
            parser,
            time_only: parser.is_some_and(|parser| !parser.has_date()),
            grouper: EntryGrouper::new(settings.format),
            stripped: Vec::new(),
            chunk: Chunk::empty(),
        }
    }

    /// Takes the line that starts at `offset`, without its line ending.
    fn push(&mut self, offset: u64, line: &[u8]) {
        // `Raw` doesn't look into lines, so they needn't be stripped.
        let line = match self.parser {
            Some(_) => strip_ansi_into(line, &mut self.stripped),
            None => line,
        };
        self.chunk.lines += 1;
        let closed = self.grouper.push_stripped(line);
        // A line that closes an entry opens the next, and the first line opens one.
        let opens = closed.is_some() || self.chunk.records.is_empty();
        if let Some(closed) = closed {
            self.chunk.close(
                offset,
                continuation_exception(
                    self.structured.map(|(syntax, _)| syntax),
                    closed.has_exception,
                ),
            );
        }
        if let Some(open) = self.grouper.open_entry()
            && opens
        {
            let (mut record, time) = match (self.structured, self.parser.zip(open.head.as_ref())) {
                (Some((syntax, mapping)), Some(_)) => (
                    read_structured(offset, line, syntax, mapping, self.zone),
                    None,
                ),
                (None, Some((parser, head))) => read_head(offset, line, head, parser, self.zone),
                _ => (Record::new(offset), None),
            };
            record.line = U48::new(self.chunk.lines);
            // The grouper only makes the first lines an entry without a head, unless
            // under `Raw`, where no entry has one.
            self.chunk.leads_in |= open.head.is_none() && self.parser.is_some();
            self.chunk.records.push(record);
            if self.time_only {
                self.chunk.times.push(time);
            }
        }
    }

    /// The chunk, once the last line, which ends at `end`, is pushed.
    fn finish(mut self, end: u64) -> Chunk {
        if let Some(open) = self.grouper.open_entry() {
            self.chunk.close(
                end,
                continuation_exception(
                    self.structured.map(|(syntax, _)| syntax),
                    open.has_exception,
                ),
            );
        }
        let timestamps = self
            .chunk
            .records
            .iter_mut()
            .map(|record| &mut record.timestamp);
        carry_forward(timestamps, NO_TIMESTAMP);
        self.chunk
    }
}

/// Whether an entry whose continuation lines look like an exception (`looks_like_one`)
/// has one. In JSON lines the entry's own exception field decides instead; a logfmt
/// line has such a field too, but Go prints a panic's trace after the entry.
fn continuation_exception(syntax: Option<Syntax>, looks_like_one: bool) -> bool {
    looks_like_one && syntax.is_none_or(|syntax| syntax == Syntax::Logfmt)
}

/// The record of an entry that starts at `offset` with `line`, ANSI-stripped, whose
/// columns `parser` found at `head`. A time of day is returned instead of put into
/// the record, as it can only be dated once the whole file is known.
fn read_head(
    offset: u64,
    line: &[u8],
    head: &EntryHead,
    parser: EntryParser,
    zone: &TimeZone,
) -> (Record, Option<Time>) {
    let mut record = Record::new(offset);
    record.level = head
        .level
        .clone()
        .and_then(|level| Level::parse(&line[level]));
    record.set_class(head.logger.as_ref());
    let timestamp = head.timestamp.clone();
    let time = match timestamp.and_then(|timestamp| parser.timestamp(&line[timestamp], zone)) {
        Some(LogTime::Instant(instant)) => {
            record.timestamp = instant.as_microsecond();
            None
        }
        Some(LogTime::TimeOfDay(time)) => Some(time),
        None => None,
    };
    (record, time)
}

/// The record of a JSON or logfmt line that starts at `offset`; `line` is
/// ANSI-stripped.
fn read_structured(
    offset: u64,
    line: &[u8],
    syntax: Syntax,
    mapping: &Mapping,
    zone: &TimeZone,
) -> Record {
    let StructuredColumns {
        timestamp,
        level,
        logger,
        has_exception,
    } = mapping.read(syntax, line, zone);
    let mut record = Record::new(offset);
    record.timestamp = timestamp.map_or(NO_TIMESTAMP, Timestamp::as_microsecond);
    record.level = level;
    record.has_exception = has_exception;
    record.set_class(logger.as_ref());
    record
}

/// Gives each of `timestamps` that is [`NO_TIMESTAMP`] the one before it, `previous`
/// for the first.
fn carry_forward<'a>(timestamps: impl Iterator<Item = &'a mut i64>, mut previous: i64) {
    for timestamp in timestamps {
        if *timestamp == NO_TIMESTAMP {
            *timestamp = previous;
        } else {
            previous = *timestamp;
        }
    }
}

/// The timestamps of the entries whose times of day are `times`, in file order: dated
/// back from the anchor, as local time in the zone, and carried forward to the
/// entries without one.
fn dated(times: &[Option<Time>], settings: &Settings) -> Vec<i64> {
    let mut dates = DatesBackward::new(settings.anchor);
    let mut timestamps = vec![NO_TIMESTAMP; times.len()];
    for (time, timestamp) in times.iter().zip(&mut timestamps).rev() {
        if let Some(time) = *time
            && let Some(instant) = dates.timestamp(time, &settings.zone)
        {
            *timestamp = instant.as_microsecond();
        }
    }
    carry_forward(timestamps.iter_mut(), NO_TIMESTAMP);
    timestamps
}

/// Puts `timestamps` into the first records, one each.
fn set_timestamps(records: &mut [Record], timestamps: &[i64]) {
    for (record, &timestamp) in records.iter_mut().zip(timestamps) {
        record.timestamp = timestamp;
    }
}
