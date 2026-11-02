use std::mem::size_of;
use std::ops::Range;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use proptest::prelude::*;

use super::*;
use crate::format::{BUILT_IN, Mapping, strip_ansi};
use crate::test_support::{EXAMPLES, built_in, no_thread_or_logger, no_timestamp};

fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn settings(preset: &'static Format) -> Settings {
    Settings {
        format: preset,
        zone: TimeZone::UTC,
        anchor: Anchor::Named("2026-10-08".parse().unwrap()),
        mapping: JSON_MAPPING,
    }
}

/// The example line of `preset`.
fn example(preset: &'static Format) -> &'static str {
    EXAMPLES
        .iter()
        .find(|(candidate, _)| *candidate == preset.id)
        .map(|(_, line)| *line)
        .unwrap()
}

/// An instant written in RFC 3339, e.g. `2026-10-07T10:00:00.123Z`.
fn instant(rfc3339: &str) -> Timestamp {
    rfc3339.parse().unwrap()
}

/// `bytes` indexed in a single chunk.
fn index(bytes: &[u8], settings: &Settings) -> Vec<Record> {
    Chunk::index(bytes, 0, 0..bytes.len(), settings).finish(settings)
}

fn index_in(preset: &'static Format, bytes: &[u8]) -> Vec<Record> {
    index(bytes, &settings(preset))
}

/// `bytes` indexed in chunks that end at `cuts`, which are in order, and joined.
fn chunked(bytes: &[u8], cuts: &[usize], settings: &Settings) -> Vec<Record> {
    let bounds: Vec<usize> = [0]
        .into_iter()
        .chain(cuts.iter().copied())
        .chain([bytes.len()])
        .collect();
    let mut chunks = bounds
        .windows(2)
        .map(|pair| Chunk::index(bytes, 0, pair[0]..pair[1], settings));
    let mut whole = chunks.next().unwrap();
    chunks.for_each(|chunk| whole.join(chunk));
    whole.finish(settings)
}

fn one_byte_chunks(bytes: &[u8], settings: &Settings) -> Vec<Record> {
    let cuts: Vec<usize> = (1..bytes.len()).collect();
    chunked(bytes, &cuts, settings)
}

fn spans(records: &[Record]) -> Vec<Range<u64>> {
    records.iter().map(Record::bytes).collect()
}

fn lines(records: &[Record]) -> Vec<Option<u64>> {
    records.iter().map(Record::line).collect()
}

/// The lines concatenated, as the bytes of a file.
fn log(lines: &[&str]) -> Vec<u8> {
    lines.concat().into_bytes()
}

/// Where each of `parts` lies when they follow each other from offset `base`.
fn laid_out(base: u64, parts: &[&[&str]]) -> Vec<Range<u64>> {
    let mut start = base;
    parts
        .iter()
        .map(|lines| {
            let end = start + lines.concat().len() as u64;
            let span = start..end;
            start = end;
            span
        })
        .collect()
}

const STARTED: &str = "2026-10-07 12:00:00.123 [main] INFO  com.example.App - Started\n";
const FAILED: &str = "2026-10-07 12:00:01.500 [main] ERROR com.example.App - Failed\n";
const EXCEPTION: &str = "java.lang.IllegalStateException: boom\n";
const FRAME: &str = "\tat com.example.App.main(App.java:10)\n";
const WRAPPED: &str = "  and the message goes on\n";
const DONE: &str = "2026-10-07 12:00:02.000 [main] WARN  com.example.App - Done\n";
/// The keys of the JSON lines below, as `Mapping::choose` picks them from such a file.
const JSON_MAPPING: Mapping = Mapping {
    timestamp: Some("time"),
    level: Some("level"),
    logger: Some("logger"),
    thread: None,
    message: Some("msg"),
};
const JSON_STARTED: &str = concat!(
    r#"{"time":"2026-10-07T12:00:00.123Z","level":"INFO","logger":"com.example.App","msg":"Started"}"#,
    "\n"
);
/// A pino-style line: the level on the pino scale, the time in epoch milliseconds and
/// the stack one object deep.
const JSON_FAILED: &str = concat!(
    r#"{"level":50,"time":1791374401500,"logger":"com.example.Pay","msg":"Failed","err":{"stack":"Error: boom\n    at main (app.js:1:1)"}}"#,
    "\n"
);
/// An entry start whose timestamp names a day that doesn't exist.
const ODD: &str = "2026-02-30 12:00:05.000 [main] INFO  com.example.App - Odd\n";

// --- the record ---

/// PRD §13 allows about 32 bytes per entry. The layout fits; this keeps it so.
#[test]
fn record_takes_at_most_32_bytes() {
    assert!(size_of::<Record>() <= 32, "{} bytes", size_of::<Record>());
}

// --- entries ---

#[test]
fn empty_file_has_no_entries() {
    for preset in BUILT_IN {
        assert_eq!(index_in(preset, b""), [], "{preset:?}");
    }
}

#[test]
fn raw_makes_every_line_an_entry() {
    let records = index_in(built_in("raw"), b"one\ntwo\r\n\nthree");
    assert_eq!(spans(&records), [0..4, 4..9, 9..10, 10..15]);
}

#[test]
fn raw_entries_have_no_timestamp_level_class_or_exception() {
    let records = index_in(built_in("raw"), &log(&[FAILED, EXCEPTION, FRAME]));
    assert_eq!(records.len(), 3);
    for record in records {
        assert_eq!(record.timestamp(), None);
        assert_eq!(record.level(), None);
        assert_eq!(record.class(), None);
        assert!(!record.has_exception());
    }
}

#[test]
fn entry_runs_over_its_continuation_lines_to_the_next_entry() {
    let records = index_in(
        built_in("logback"),
        &log(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]),
    );
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[STARTED], &[FAILED, EXCEPTION, FRAME], &[DONE]])
    );
}

#[test]
fn last_entry_runs_to_the_end_of_the_file() {
    let unterminated = DONE.trim_end();
    let records = index_in(built_in("logback"), &log(&[STARTED, unterminated, "\n  "]));
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[STARTED], &[unterminated, "\n  "]])
    );
}

#[test]
fn crlf_line_endings_belong_to_their_entry() {
    let started = STARTED.replace('\n', "\r\n");
    let frame = FRAME.replace('\n', "\r\n");
    let done = DONE.replace('\n', "\r\n");
    let records = index_in(built_in("logback"), &log(&[&started, &frame, &done]));
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[&started, &frame], &[&done]])
    );
}

#[test]
fn entry_with_a_stack_trace_has_an_exception() {
    let records = index_in(built_in("logback"), &log(&[FAILED, EXCEPTION, FRAME, DONE]));
    assert!(records[0].has_exception());
    assert!(!records[1].has_exception());
}

#[test]
fn entry_with_plain_continuation_lines_has_no_exception() {
    let records = index_in(built_in("logback"), &log(&[STARTED, WRAPPED, WRAPPED]));
    assert_eq!(records.len(), 1);
    assert!(!records[0].has_exception());
}

#[test]
fn level_comes_from_the_level_column() {
    let records = index_in(built_in("logback"), &log(&[STARTED, FAILED, DONE]));
    let levels: Vec<_> = records.iter().map(Record::level).collect();
    assert_eq!(
        levels,
        [Some(Level::Info), Some(Level::Error), Some(Level::Warn)]
    );
}

#[test]
fn level_word_that_is_no_level_gives_none() {
    let audit = "2026-10-07 12:00:00.123 [main] AUDIT com.example.App - Hello\n";
    let records = index_in(built_in("logback"), audit.as_bytes());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].level(), None);
    assert!(records[0].class().is_some());
}

#[test]
fn class_is_a_span_of_the_first_line() {
    let records = index_in(built_in("logback"), &log(&[STARTED, FAILED]));
    let class = records[1].class().unwrap();
    assert_eq!(&FAILED[class], "com.example.App");
}

#[test]
fn class_span_is_counted_in_the_line_without_escape_codes() {
    let coloured = "\x1b[0m\x1b[32m12:00:00,123 INFO  \x1b[0m[\x1b[36morg.jboss.as\x1b[0m] (MSC service thread 1-2) WFLYSRV0049: starting\x1b[0m\n";
    let records = index_in(built_in("wildfly-console"), coloured.as_bytes());
    let stripped = strip_ansi(coloured.as_bytes());
    assert_eq!(&stripped[records[0].class().unwrap()], b"org.jboss.as");
}

/// A Logback line whose class `com.example.App` ends at byte `end` of the line.
fn line_with_class_ending_at(end: usize) -> String {
    let around_thread = "2026-10-07 12:00:00.123 [".len() + "] INFO  ".len();
    let thread = "t".repeat(end - "com.example.App".len() - around_thread);
    format!("2026-10-07 12:00:00.123 [{thread}] INFO  com.example.App - Hello\n")
}

#[test]
fn class_ending_at_byte_65535_is_kept() {
    let line = line_with_class_ending_at(65_535);
    let records = index_in(built_in("logback"), line.as_bytes());
    assert_eq!(records[0].class(), Some(65_520..65_535));
}

#[test]
fn class_reaching_past_byte_65535_is_left_out() {
    let line = line_with_class_ending_at(65_536);
    let records = index_in(built_in("logback"), line.as_bytes());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].class(), None);
    assert_eq!(records[0].level(), Some(Level::Info));
}

/// PRD §6.1: a format without a timestamp dates nothing.
#[test]
fn an_entry_with_a_level_but_no_timestamp_has_its_level_and_no_time() {
    let log = "WARNING:app.orders:Payment retry 1/3\nINFO:app.orders:Order 42 created\n";
    let records = index_in(no_timestamp(), log.as_bytes());
    let read: Vec<_> = records
        .iter()
        .map(|record| (record.level(), record.timestamp()))
        .collect();
    assert_eq!(read, [(Some(Level::Warn), None), (Some(Level::Info), None)]);
}

#[test]
fn an_entry_of_a_format_without_a_logger_has_no_logger_span() {
    let log = "2026-10-07 12:00:00.123 [INF] Order 42 created\n";
    let records = index_in(no_thread_or_logger(), log.as_bytes());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].class(), None);
}

// --- the headless entry ---

#[test]
fn lines_before_the_first_entry_are_a_headless_entry() {
    let banner = "  .   ____          _\n";
    let records = index_in(built_in("logback"), &log(&[banner, "\n", STARTED]));
    assert_eq!(spans(&records), laid_out(0, &[&[banner, "\n"], &[STARTED]]));
    let headless = records[0];
    assert_eq!(headless.timestamp(), None);
    assert_eq!(headless.level(), None);
    assert_eq!(headless.class(), None);
}

#[test]
fn headless_entry_checks_its_first_line_for_an_exception() {
    let records = index_in(built_in("logback"), &log(&[EXCEPTION, WRAPPED, STARTED]));
    assert!(records[0].has_exception());
}

#[test]
fn headless_entry_of_plain_lines_has_no_exception() {
    let records = index_in(built_in("logback"), &log(&[WRAPPED, STARTED]));
    assert!(!records[0].has_exception());
}

#[test]
fn headless_entry_starts_on_line_1() {
    let records = index_in(built_in("logback"), &log(&[WRAPPED, WRAPPED, STARTED]));
    assert_eq!(lines(&records), [Some(1), Some(3)]);
}

// --- timestamps ---

#[test]
fn timestamp_without_offset_is_local_time_in_the_zone() {
    let warsaw = Settings {
        zone: zone("Europe/Warsaw"),
        ..settings(built_in("logback"))
    };
    let records = index(STARTED.as_bytes(), &warsaw);
    assert_eq!(
        records[0].timestamp(),
        Some(instant("2026-10-07T10:00:00.123Z"))
    );
}

#[test]
fn timestamp_with_an_offset_ignores_the_zone() {
    let spring3 = example(built_in("spring3"));
    let new_york = Settings {
        zone: zone("America/New_York"),
        ..settings(built_in("spring3"))
    };
    let records = index(spring3.as_bytes(), &new_york);
    assert_eq!(
        records[0].timestamp(),
        Some(instant("2026-10-07T10:00:00.123Z"))
    );
}

#[test]
fn timestamp_keeps_microseconds_and_drops_what_is_finer() {
    let line = "2026-10-07 12:00:00.123456789 [main] INFO  com.example.App - Started\n";
    let records = index_in(built_in("logback"), line.as_bytes());
    assert_eq!(
        records[0].timestamp(),
        Some(instant("2026-10-07T12:00:00.123456Z"))
    );
}

#[test]
fn entry_with_an_impossible_timestamp_takes_the_one_before() {
    let records = index_in(built_in("logback"), &log(&[STARTED, ODD]));
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].timestamp(),
        Some(instant("2026-10-07T12:00:00.123Z"))
    );
}

#[test]
fn entries_before_the_first_timestamp_have_none() {
    let records = index_in(built_in("logback"), &log(&[WRAPPED, ODD, STARTED]));
    let timestamps: Vec<_> = records.iter().map(Record::timestamp).collect();
    assert_eq!(
        timestamps,
        [None, None, Some(instant("2026-10-07T12:00:00.123Z"))]
    );
}

const BEFORE_MIDNIGHT: &str = "23:59:59.000 [main] INFO  com.example.App -- before\n";
const AFTER_MIDNIGHT: &str = "00:00:01.000 [main] INFO  com.example.App -- after\n";

#[test]
fn time_only_entries_are_dated_back_from_a_named_anchor() {
    let records = index_in(
        built_in("logback-basic"),
        &log(&[BEFORE_MIDNIGHT, AFTER_MIDNIGHT]),
    );
    let timestamps: Vec<_> = records.iter().map(Record::timestamp).collect();
    assert_eq!(
        timestamps,
        [
            Some(instant("2026-10-07T23:59:59Z")),
            Some(instant("2026-10-08T00:00:01Z"))
        ]
    );
}

#[test]
fn time_only_entries_are_dated_back_from_the_mtime() {
    let settings = Settings {
        anchor: Anchor::Modified("2026-10-09T00:30:00".parse().unwrap()),
        ..settings(built_in("logback-basic"))
    };
    let records = index(BEFORE_MIDNIGHT.as_bytes(), &settings);
    assert_eq!(
        records[0].timestamp(),
        Some(instant("2026-10-08T23:59:59Z"))
    );
}

#[test]
fn time_only_dates_are_local_time_in_the_zone() {
    let warsaw = Settings {
        zone: zone("Europe/Warsaw"),
        ..settings(built_in("logback-basic"))
    };
    let records = index(&log(&[BEFORE_MIDNIGHT, AFTER_MIDNIGHT]), &warsaw);
    let timestamps: Vec<_> = records.iter().map(Record::timestamp).collect();
    assert_eq!(
        timestamps,
        [
            Some(instant("2026-10-07T21:59:59Z")),
            Some(instant("2026-10-07T22:00:01Z"))
        ]
    );
}

#[test]
fn time_only_entry_with_an_impossible_time_takes_the_one_before() {
    let odd = "25:00:00.000 [main] INFO  com.example.App -- odd\n";
    let records = index_in(built_in("logback-basic"), &log(&[BEFORE_MIDNIGHT, odd]));
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].timestamp(), records[0].timestamp());
    assert!(records[1].timestamp().is_some());
}

// --- chunks ---

#[test]
fn chunk_boundary_inside_a_line_changes_nothing() {
    let bytes = log(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]);
    let settings = settings(built_in("logback"));
    let cuts = [10, STARTED.len() + 10];
    assert_eq!(chunked(&bytes, &cuts, &settings), index(&bytes, &settings));
    assert_eq!(chunked(&bytes, &cuts, &settings).len(), 3);
}

#[test]
fn chunk_boundary_inside_a_multi_line_entry_changes_nothing() {
    let bytes = log(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]);
    let settings = settings(built_in("logback"));
    let exception = STARTED.len() + FAILED.len();
    let cuts = [exception, exception + EXCEPTION.len() + 3];
    let records = chunked(&bytes, &cuts, &settings);
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[STARTED], &[FAILED, EXCEPTION, FRAME], &[DONE]])
    );
    assert_eq!(records, index(&bytes, &settings));
}

#[test]
fn exception_after_a_chunk_boundary_flags_the_entry_before_it() {
    let bytes = log(&[FAILED, WRAPPED, EXCEPTION, DONE]);
    let cut = FAILED.len() + WRAPPED.len();
    let records = chunked(&bytes, &[cut], &settings(built_in("logback")));
    assert_eq!(records.len(), 2);
    assert!(records[0].has_exception());
}

#[test]
fn chunk_boundary_between_cr_and_lf_changes_nothing() {
    let started = STARTED.replace('\n', "\r\n");
    let frame = FRAME.replace('\n', "\r\n");
    let bytes = log(&[&started, &frame, &started]);
    let settings = settings(built_in("logback"));
    let cuts = [started.len() - 1, started.len() + frame.len() - 1];
    let records = chunked(&bytes, &cuts, &settings);
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[&started, &frame], &[&started]])
    );
    assert_eq!(records, index(&bytes, &settings));
}

#[test]
fn chunk_without_a_line_start_changes_nothing() {
    let bytes = log(&[STARTED, FAILED]);
    let settings = settings(built_in("logback"));
    assert_eq!(
        chunked(&bytes, &[5, 10], &settings),
        index(&bytes, &settings)
    );
}

#[test]
fn empty_chunks_change_nothing() {
    let bytes = log(&[STARTED, FRAME, FAILED]);
    let settings = settings(built_in("logback"));
    let len = bytes.len();
    let cuts = [0, 0, STARTED.len(), STARTED.len(), len, len];
    assert_eq!(chunked(&bytes, &cuts, &settings), index(&bytes, &settings));
}

#[test]
fn chunk_of_only_continuation_lines_extends_the_entry_before_it() {
    let bytes = log(&[STARTED, WRAPPED, FRAME, WRAPPED, DONE]);
    let first = STARTED.len();
    let cuts = [first, first + WRAPPED.len()];
    let records = chunked(&bytes, &cuts, &settings(built_in("logback")));
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[STARTED, WRAPPED, FRAME, WRAPPED], &[DONE]])
    );
    assert!(records[0].has_exception());
}

#[test]
fn headless_entry_cut_into_chunks_stays_one_entry() {
    let bytes = log(&[WRAPPED, EXCEPTION, FRAME, STARTED]);
    let cuts = [WRAPPED.len(), WRAPPED.len() + EXCEPTION.len()];
    let records = chunked(&bytes, &cuts, &settings(built_in("logback")));
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[WRAPPED, EXCEPTION, FRAME], &[STARTED]])
    );
    assert!(records[0].has_exception());
    assert_eq!(records, index_in(built_in("logback"), &bytes));
}

#[test]
fn impossible_timestamp_after_a_chunk_boundary_takes_the_one_before() {
    let bytes = log(&[STARTED, WRAPPED, ODD]);
    let records = chunked(
        &bytes,
        &[STARTED.len() + WRAPPED.len()],
        &settings(built_in("logback")),
    );
    assert_eq!(
        records[1].timestamp(),
        Some(instant("2026-10-07T12:00:00.123Z"))
    );
}

#[test]
fn time_only_entries_in_separate_chunks_are_dated_as_one() {
    let bytes = log(&[BEFORE_MIDNIGHT, AFTER_MIDNIGHT, BEFORE_MIDNIGHT]);
    let settings = settings(built_in("logback-basic"));
    let cuts = [
        BEFORE_MIDNIGHT.len(),
        BEFORE_MIDNIGHT.len() + AFTER_MIDNIGHT.len(),
    ];
    assert_eq!(chunked(&bytes, &cuts, &settings), index(&bytes, &settings));
}

/// Every preset's example line, a coloured one, a headless start, stack traces,
/// CRLF and LF endings, an impossible timestamp and no final newline.
fn mixed_log() -> Vec<u8> {
    let mut text = String::from("Picked up JAVA_TOOL_OPTIONS: -Xmx1g\r\n");
    text += EXCEPTION;
    for (_, example) in EXAMPLES {
        text += example;
        text += "\r\n";
        text += FRAME;
        text += "\x1b[31m";
        text += example;
        text += "\x1b[0m\n";
        text += WRAPPED;
    }
    text += ODD;
    text += "\r\n";
    text += STARTED.trim_end();
    text.into_bytes()
}

#[test]
fn one_byte_chunks_give_the_same_entries_in_every_preset() {
    let bytes = mixed_log();
    for preset in BUILT_IN {
        let settings = settings(preset);
        assert_eq!(
            one_byte_chunks(&bytes, &settings),
            index(&bytes, &settings),
            "{preset:?}"
        );
    }
}

// --- JSON lines ---

fn json(lines: &[&str]) -> Vec<Record> {
    index_in(built_in("json"), &log(lines))
}

fn levels(records: &[Record]) -> Vec<Option<Level>> {
    records.iter().map(Record::level).collect()
}

#[test]
fn json_line_with_a_null_or_no_level_has_no_level() {
    let records = json(&[
        "{\"level\":null,\"msg\":\"a\"}\n",
        "{\"msg\":\"b\"}\n",
        JSON_STARTED,
    ]);
    assert_eq!(levels(&records), [None, None, Some(Level::Info)]);
}

#[test]
fn json_timestamp_in_quotes_is_not_read_as_epoch_time() {
    let records = json(&[
        "{\"time\":\"1791374400123\",\"msg\":\"a\"}\n",
        "{\"time\":1791374400123,\"msg\":\"b\"}\n",
    ]);
    assert_eq!(records[0].timestamp(), None);
    assert_eq!(
        records[1].timestamp(),
        Some(instant("2026-10-07T12:00:00.123Z"))
    );
}

#[test]
fn json_exception_field_that_is_empty_or_null_is_no_exception() {
    let records = json(&[
        "{\"msg\":\"a\",\"stack_trace\":\"\"}\n",
        "{\"msg\":\"b\",\"stack_trace\":null}\n",
        "{\"msg\":\"c\",\"stack_trace\":\"java.lang.IllegalStateException: boom\"}\n",
    ]);
    assert_eq!(
        records
            .iter()
            .map(Record::has_exception)
            .collect::<Vec<_>>(),
        [false, false, true]
    );
}

/// The exception key isn't chosen for the file: each line has the first candidate
/// that holds a value.
#[test]
fn json_exception_is_the_first_candidate_on_its_line_with_a_value() {
    let records = json(&[
        "{\"msg\":\"a\",\"stack_trace\":\"boom\"}\n",
        "{\"msg\":\"b\",\"stack_trace\":\"\",\"exception\":\"boom\"}\n",
    ]);
    assert!(records.iter().all(Record::has_exception));
}

#[test]
fn json_logger_after_a_byte_order_mark_or_leading_blanks_is_found_in_the_line() {
    let lines = [
        format!("\u{feff}{JSON_STARTED}"),
        format!(" \t{JSON_STARTED}"),
    ];
    let records = json(&[&lines[0], &lines[1]]);
    for (line, record) in lines.iter().zip(&records) {
        assert_eq!(
            &line[record.class().unwrap()],
            "com.example.App",
            "{line:?}"
        );
    }
}

/// In JSON lines the exception field decides: a continuation line that looks like a
/// stack frame doesn't give the entry an exception.
#[test]
fn json_entry_continued_by_stack_trace_lines_has_no_exception() {
    let records = json(&[JSON_STARTED, EXCEPTION, FRAME, JSON_FAILED]);
    assert_eq!(
        records
            .iter()
            .map(Record::has_exception)
            .collect::<Vec<_>>(),
        [false, true]
    );
}

#[test]
fn json_logger_value_reaching_past_64_kib_is_not_kept() {
    let long = format!(
        "{{\"level\":\"WARN\",\"logger\":\"{}\",\"msg\":\"m\"}}\n",
        "a".repeat(70_000)
    );
    let records = json(&[&long, JSON_STARTED]);
    assert_eq!(records[0].class(), None);
    assert_eq!(records[0].level(), Some(Level::Warn));
    assert!(records[1].class().is_some());
}

#[test]
fn line_that_is_not_a_json_object_continues_the_entry_before() {
    let plain = "plain text\n";
    let records = json(&[JSON_STARTED, plain, JSON_FAILED]);
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[JSON_STARTED, plain], &[JSON_FAILED]])
    );
    assert_eq!(levels(&records), [Some(Level::Info), Some(Level::Error)]);
}

// --- logfmt ---

fn logfmt(lines: &[&str]) -> Vec<Record> {
    index_in(built_in("logfmt"), &log(lines))
}

/// PRD §6.7: only an unquoted number is epoch time. In quotes it is text, and text
/// that isn't a timestamp gives none.
#[test]
fn logfmt_timestamp_is_epoch_time_only_without_quotes() {
    let records = logfmt(&[
        "time=\"1791374400123\" msg=a\n",
        "time=1791374400123 msg=b\n",
    ]);
    assert_eq!(records[0].timestamp(), None);
    assert_eq!(
        records[1].timestamp(),
        Some(instant("2026-10-07T12:00:00.123Z"))
    );
}

/// A number is no level: the pino scale is for JSON lines.
#[test]
fn logfmt_level_is_read_by_name_with_or_without_quotes() {
    let records = logfmt(&[
        "level=warning msg=a\n",
        "level=\"ERROR\" msg=b\n",
        "level=30 msg=c\n",
    ]);
    assert_eq!(
        levels(&records),
        [Some(Level::Warn), Some(Level::Error), None]
    );
}

#[test]
fn logfmt_logger_span_is_the_value_inside_any_quotes() {
    let lines = [
        "logger=com.example.App msg=a\n",
        "logger=\"com.example.App\" msg=b\n",
    ];
    let records = logfmt(&lines);
    for (line, record) in lines.iter().zip(&records) {
        assert_eq!(
            record.class().map(|class| &line[class]),
            Some("com.example.App"),
            "{line:?}"
        );
    }
}

/// As in JSON lines, a line's exception is an exception candidate with a value.
#[test]
fn logfmt_exception_field_with_a_value_is_an_exception() {
    let records = logfmt(&[
        "msg=a stacktrace=\n",
        "msg=b stacktrace=\"goroutine 1 [running]:\\nmain.main()\"\n",
    ]);
    let exceptions: Vec<bool> = records.iter().map(Record::has_exception).collect();
    assert_eq!(exceptions, [false, true]);
}

/// Only a line that starts with a key and its `=` starts an entry: indented pairs,
/// text with a pair further on and an `=` without a key continue the entry before.
#[test]
fn line_that_does_not_start_with_a_key_continues_the_logfmt_entry_before() {
    let (first, indented, text, keyless, last) = (
        "level=INFO msg=a\n",
        "  user=42 status=ok\n",
        "retrying with attempt=2\n",
        "=2 more\n",
        "level=ERROR msg=b\n",
    );
    let records = logfmt(&[first, indented, text, keyless, last]);
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[first, indented, text, keyless], &[last]])
    );
    assert_eq!(levels(&records), [Some(Level::Info), Some(Level::Error)]);
}

// --- files larger than 4 GiB ---

const FIVE_GIB: u64 = 5 << 30;

#[test]
fn offsets_past_4_gib_are_kept() {
    let bytes = log(&[STARTED, FAILED, FRAME]);
    let settings = settings(built_in("logback"));
    let records = Chunk::index(&bytes, FIVE_GIB, 0..bytes.len(), &settings).finish(&settings);
    assert_eq!(
        spans(&records),
        laid_out(FIVE_GIB, &[&[STARTED], &[FAILED, FRAME]])
    );
}

#[test]
fn chunks_past_4_gib_join() {
    let bytes = log(&[STARTED, FAILED, FRAME]);
    let settings = settings(built_in("logback"));
    let mut whole = Chunk::index(&bytes, FIVE_GIB, 0..STARTED.len() + 4, &settings);
    whole.join(Chunk::index(
        &bytes,
        FIVE_GIB,
        STARTED.len() + 4..bytes.len(),
        &settings,
    ));
    assert_eq!(
        spans(&whole.finish(&settings)),
        laid_out(FIVE_GIB, &[&[STARTED], &[FAILED, FRAME]])
    );
}

// --- any input ---

/// Bytes with line endings and entry starts common enough to matter.
fn any_bytes() -> impl Strategy<Value = Vec<u8>> {
    let piece = prop_oneof![
        4 => prop::collection::vec(any::<u8>(), 1..4),
        1 => Just(b"\n".to_vec()),
        1 => Just(b"\r\n".to_vec()),
        1 => Just(STARTED.as_bytes().to_vec()),
        1 => Just(example(built_in("logback-basic")).as_bytes().to_vec()),
        1 => Just(example(built_in("wildfly-console")).as_bytes().to_vec()),
        1 => Just(JSON_STARTED.as_bytes().to_vec()),
        1 => Just(JSON_FAILED.as_bytes().to_vec()),
    ];
    prop::collection::vec(piece, 0..40).prop_map(|pieces| pieces.concat())
}

fn any_preset() -> impl Strategy<Value = &'static Format> {
    prop::sample::select(BUILT_IN.iter().collect::<Vec<_>>())
}

/// Bytes and positions to cut them at, in order.
fn bytes_and_cuts() -> impl Strategy<Value = (Vec<u8>, Vec<usize>)> {
    any_bytes().prop_flat_map(|bytes| {
        let len = bytes.len();
        let cuts = prop::collection::vec(0..=len, 0..8).prop_map(|mut cuts| {
            cuts.sort_unstable();
            cuts
        });
        (Just(bytes), cuts)
    })
}

proptest! {
    #[test]
    fn indexing_never_panics_on_arbitrary_bytes(
        (bytes, cuts) in bytes_and_cuts(),
        preset in any_preset(),
    ) {
        chunked(&bytes, &cuts, &settings(preset));
    }

    #[test]
    fn entries_cover_the_file_without_gaps(bytes in any_bytes(), preset in any_preset()) {
        let records = index_in(preset, &bytes);
        let mut end = 0;
        for record in &records {
            prop_assert_eq!(record.bytes().start, end);
            prop_assert!(record.bytes().end > end);
            end = record.bytes().end;
        }
        prop_assert_eq!(end, bytes.len() as u64);
    }

    #[test]
    fn base_offset_moves_every_entry_by_as_much(
        bytes in any_bytes(),
        preset in any_preset(),
        // No file reaches 2^48 bytes (256 TiB), the most a 64-bit address space maps.
        base in 0..1u64 << 47,
    ) {
        let settings = settings(preset);
        let moved = Chunk::index(&bytes, base, 0..bytes.len(), &settings).finish(&settings);
        let at_zero = index(&bytes, &settings);
        prop_assert_eq!(moved.len(), at_zero.len());
        for (moved, at_zero) in moved.iter().zip(&at_zero) {
            let span = at_zero.bytes();
            prop_assert_eq!(moved.bytes(), span.start + base..span.end + base);
        }
    }
}

// --- appends ---

/// The index of `bytes` built as the file grows: its first `cuts[0]` bytes indexed
/// at once, then the file up to each later cut and finally the whole of it appended.
/// `cuts` are in order.
fn grown(bytes: &[u8], cuts: &[usize], settings: &Settings) -> Vec<Record> {
    let (&first, later) = cuts.split_first().unwrap_or((&bytes.len(), &[]));
    let mut records = index(&bytes[..first], settings);
    for &end in later.iter().chain([&bytes.len()]) {
        append(&mut records, &bytes[..end], settings);
    }
    records
}

#[test]
fn appending_byte_by_byte_gives_the_index_of_the_file_so_far_in_every_preset() {
    let bytes = mixed_log();
    for preset in BUILT_IN {
        let settings = settings(preset);
        let mut records = Vec::new();
        for end in 1..=bytes.len() {
            append(&mut records, &bytes[..end], &settings);
            assert_eq!(
                records,
                index(&bytes[..end], &settings),
                "{preset:?}, {end} bytes"
            );
        }
    }
}

#[test]
fn appending_only_continuation_lines_extends_the_last_entry() {
    let settings = settings(built_in("logback"));
    let mut records = index(&log(&[STARTED, FAILED]), &settings);
    append(
        &mut records,
        &log(&[STARTED, FAILED, EXCEPTION, FRAME]),
        &settings,
    );
    assert_eq!(
        spans(&records),
        laid_out(0, &[&[STARTED], &[FAILED, EXCEPTION, FRAME]])
    );
    assert!(records[1].has_exception());
}

#[test]
fn appended_entries_go_on_counting_lines() {
    let settings = settings(built_in("logback"));
    let mut records = index(&log(&[STARTED, FAILED]), &settings);
    append(
        &mut records,
        &log(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]),
        &settings,
    );
    assert_eq!(lines(&records), [Some(1), Some(2), Some(5)]);
}

/// Before midnight, after it, a late line from before it and after it again: dated
/// forward from the first entry, which keeps the date the anchor gave it.
#[test]
fn time_only_entries_appended_across_midnight_are_dated_forward() {
    let bytes = log(&[
        BEFORE_MIDNIGHT,
        AFTER_MIDNIGHT,
        BEFORE_MIDNIGHT,
        AFTER_MIDNIGHT,
    ]);
    let settings = settings(built_in("logback-basic"));
    let start = BEFORE_MIDNIGHT.len() + 3;
    let records = grown(&bytes, &(start..bytes.len()).collect::<Vec<_>>(), &settings);
    let timestamps: Vec<_> = records.iter().map(Record::timestamp).collect();
    assert_eq!(
        timestamps,
        [
            Some(instant("2026-10-08T23:59:59Z")),
            Some(instant("2026-10-09T00:00:01Z")),
            Some(instant("2026-10-08T23:59:59Z")),
            Some(instant("2026-10-09T00:00:01Z")),
        ]
    );
}

#[test]
fn time_only_entries_appended_are_dated_forward_in_the_zone() {
    let early = "01:00:00.000 [main] INFO  com.example.App -- early\n";
    let late = "21:30:00.000 [main] INFO  com.example.App -- late\n";
    let warsaw = Settings {
        zone: zone("Europe/Warsaw"),
        ..settings(built_in("logback-basic"))
    };
    let records = grown(&log(&[early, late]), &[early.len()], &warsaw);
    assert_eq!(
        records[1].timestamp(),
        Some(instant("2026-10-08T19:30:00Z"))
    );
}

#[test]
fn time_only_entries_appended_after_no_time_are_dated_back_from_the_anchor() {
    let bytes = log(&[WRAPPED, BEFORE_MIDNIGHT, AFTER_MIDNIGHT]);
    let settings = settings(built_in("logback-basic"));
    assert_eq!(
        grown(&bytes, &[WRAPPED.len()], &settings),
        index(&bytes, &settings)
    );
}

/// A log of `Logback` and `LogbackBasic` entries, each the other's continuation
/// lines, with stack traces, wrapped and blank lines, an impossible timestamp, LF and
/// CRLF endings and maybe no final line ending. The times of day lie between 10:00
/// and 12:00, so dating them crosses no midnight.
fn growing_log() -> impl Strategy<Value = Vec<u8>> {
    let time = (10..12u8, 0..60u8, 0..60u8, 0..1000u16)
        .prop_map(|(h, m, s, ms)| format!("{h:02}:{m:02}:{s:02}.{ms:03}"));
    let line = prop_oneof![
        2 => time
            .clone()
            .prop_map(|time| format!("2026-10-07 {time} [main] ERROR com.example.App - m")),
        2 => time.prop_map(|time| format!("{time} [main] INFO  com.example.App -- m")),
        1 => Just(EXCEPTION.trim_end().to_owned()),
        1 => Just(FRAME.trim_end().to_owned()),
        1 => Just(WRAPPED.trim_end().to_owned()),
        1 => Just(String::new()),
        1 => Just(ODD.trim_end().to_owned()),
        1 => Just(JSON_STARTED.trim_end().to_owned()),
        1 => Just(JSON_FAILED.trim_end().to_owned()),
    ];
    let ending = prop_oneof![Just("\n"), Just("\r\n")];
    (prop::collection::vec((line, ending), 0..30), any::<bool>()).prop_map(|(lines, terminated)| {
        let mut text: String = lines.iter().map(|(line, end)| line.clone() + end).collect();
        if !terminated {
            text.truncate(text.trim_end_matches(['\r', '\n']).len());
        }
        text.into_bytes()
    })
}

/// `growing_log` and where to cut it, into pieces of at most 16 bytes, down to one.
fn growing_log_and_cuts() -> impl Strategy<Value = (Vec<u8>, Vec<usize>)> {
    growing_log().prop_flat_map(|bytes| {
        let len = bytes.len();
        let cuts = prop::collection::vec(1..=16usize, 0..200).prop_map(move |sizes| {
            sizes
                .iter()
                .scan(0, |end, size| {
                    *end += size;
                    Some(*end)
                })
                .take_while(|&end| end < len)
                .collect()
        });
        (Just(bytes), cuts)
    })
}

proptest! {
    #[test]
    fn appending_in_any_pieces_gives_the_index_of_the_whole_file(
        (bytes, cuts) in growing_log_and_cuts(),
        preset in prop::sample::select(vec![
            built_in("logback"),
            built_in("logback-basic"),
            built_in("json"),
        ]),
    ) {
        let settings = settings(preset);
        prop_assert_eq!(grown(&bytes, &cuts, &settings), index(&bytes, &settings));
    }

    #[test]
    fn appending_never_panics_on_arbitrary_bytes(
        (bytes, cuts) in bytes_and_cuts(),
        preset in any_preset(),
    ) {
        grown(&bytes, &cuts, &settings(preset));
    }
}

// --- progress ---

fn progress(indexed: u64, total: u64) -> Progress {
    Progress { indexed, total }
}

#[test]
fn progress_is_complete_once_every_byte_is_indexed() {
    assert!(progress(10, 10).is_complete());
}

#[test]
fn progress_short_of_the_total_is_not_complete() {
    assert!(!progress(9, 10).is_complete());
}

#[test]
fn empty_file_is_complete_from_the_start() {
    assert!(progress(0, 0).is_complete());
    assert_eq!(progress(0, 0).percent(), 100);
}

#[test]
fn percent_of_nothing_is_zero() {
    assert_eq!(progress(0, 10).percent(), 0);
}

#[test]
fn percent_of_everything_is_100() {
    assert_eq!(progress(10, 10).percent(), 100);
}

#[test]
fn percent_rounds_down() {
    assert_eq!(progress(1, 3).percent(), 33);
    assert_eq!(progress(2, 3).percent(), 66);
}

#[test]
fn percent_shows_100_only_when_complete() {
    assert_eq!(progress(999, 1000).percent(), 99);
}

#[test]
fn percent_of_huge_files_does_not_overflow() {
    assert_eq!(progress(FIVE_GIB, 2 * FIVE_GIB).percent(), 50);
    assert_eq!(progress(u64::MAX - 1, u64::MAX).percent(), 99);
    assert_eq!(progress(u64::MAX / 2, u64::MAX).percent(), 49);
}

// --- the background indexer ---

/// How long a test waits for the indexer on a file of a few kilobytes before it
/// fails. Such a file takes milliseconds, even in a debug build.
const DEADLINE: Duration = Duration::from_secs(1);

/// How long a test waits for the indexer on a [`LARGE`] file before it fails. Such
/// a file takes well under a second on a few cores, even in a debug build.
const LARGE_DEADLINE: Duration = Duration::from_secs(5);

/// How soon after it is dropped the indexer has to stop.
const PROMPTLY: Duration = Duration::from_secs(2);

/// Polls until indexing is complete and returns every progress seen. Fails the test
/// after `within`.
fn poll_to_the_end(
    indexer: &mut Indexer,
    records: &mut Vec<Record>,
    within: Duration,
) -> Vec<Progress> {
    let deadline = Instant::now() + within;
    let mut seen = Vec::new();
    loop {
        let progress = indexer.poll(records);
        seen.push(progress);
        if progress.is_complete() {
            return seen;
        }
        assert!(Instant::now() < deadline, "not complete: {progress:?}");
        thread::sleep(Duration::from_millis(1));
    }
}

/// `bytes` indexed by an [`Indexer`] that has to be done within `within`.
fn indexed_in_background(bytes: Vec<u8>, settings: &Settings, within: Duration) -> Vec<Record> {
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings.clone());
    let mut records = Vec::new();
    poll_to_the_end(&mut indexer, &mut records, within);
    records
}

/// About `size` bytes of numbered entries, every seventh with a stack trace. The
/// times run through a whole day and start over, so a time-only log crosses
/// midnight again and again.
fn large_log(preset: &'static Format, size: usize) -> Vec<u8> {
    let mut block = String::new();
    for i in 0..1000 {
        let second = i * 86;
        let time = format!(
            "{:02}:{:02}:{:02}.{:03}",
            second / 3600,
            second / 60 % 60,
            second % 60,
            i % 1000
        );
        let (level, trace) = if i % 7 == 0 {
            ("ERROR", [EXCEPTION, FRAME, FRAME].concat())
        } else {
            ("INFO ", String::new())
        };
        let thread = format!("worker-{}", i % 8);
        let class = format!("com.example.Service{}", i % 13);
        block += &match preset.id {
            "logback-basic" => {
                format!("{time} [{thread}] {level} {class} -- step {i}\n{trace}")
            }
            _ => format!("2026-10-07 {time} [{thread}] {level} {class} - step {i}\n{trace}"),
        };
    }
    block.repeat(size / block.len() + 1).into_bytes()
}

/// 16 chunks of the indexer's 128 KiB: enough for chunks to finish out of order and
/// for the records to arrive in several deliveries.
const LARGE: usize = 2 << 20;

#[test]
fn indexer_finds_the_entries_of_one_pass() {
    for preset in BUILT_IN {
        let settings = settings(preset);
        assert_eq!(
            indexed_in_background(mixed_log(), &settings, DEADLINE),
            index(&mixed_log(), &settings),
            "{preset:?}"
        );
    }
}

#[test]
fn indexer_finds_the_entries_of_one_pass_in_a_large_file() {
    let bytes = large_log(built_in("logback"), LARGE);
    let settings = settings(built_in("logback"));
    let expected = index(&bytes, &settings);
    assert_eq!(
        indexed_in_background(bytes, &settings, LARGE_DEADLINE),
        expected
    );
}

#[test]
fn indexer_dates_a_large_time_only_file_as_one_pass_does() {
    let bytes = large_log(built_in("logback-basic"), LARGE);
    let settings = settings(built_in("logback-basic"));
    let expected = index(&bytes, &settings);
    assert_eq!(
        indexed_in_background(bytes, &settings, LARGE_DEADLINE),
        expected
    );
}

#[test]
fn indexer_completes_an_empty_file_without_entries() {
    assert_eq!(
        indexed_in_background(Vec::new(), &settings(built_in("logback")), DEADLINE),
        []
    );
}

#[test]
fn progress_never_goes_back_and_ends_at_the_file_size() {
    let bytes = large_log(built_in("logback"), LARGE);
    let total = bytes.len() as u64;
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings(built_in("logback")));
    let seen = poll_to_the_end(&mut indexer, &mut Vec::new(), LARGE_DEADLINE);
    assert!(seen.iter().all(|progress| progress.total == total));
    assert!(
        seen.windows(2)
            .all(|pair| pair[0].indexed <= pair[1].indexed),
        "{seen:?}"
    );
    assert_eq!(seen.last(), Some(&progress(total, total)));
}

/// Dating the entries of a time-only file takes a while after its last chunk is
/// read; progress stays short of the total until it is done.
#[test]
fn progress_of_a_time_only_file_never_goes_back_or_past_the_file_size() {
    let bytes = large_log(built_in("logback-basic"), LARGE);
    let total = bytes.len() as u64;
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings(built_in("logback-basic")));
    let seen = poll_to_the_end(&mut indexer, &mut Vec::new(), LARGE_DEADLINE);
    assert!(
        seen.iter().all(|progress| progress.indexed <= total),
        "{seen:?}"
    );
    assert!(
        seen.windows(2)
            .all(|pair| pair[0].indexed <= pair[1].indexed),
        "{seen:?}"
    );
    assert_eq!(seen.last(), Some(&progress(total, total)));
}

#[test]
fn entries_handed_over_lie_within_the_bytes_indexed_so_far() {
    let bytes = large_log(built_in("logback"), LARGE);
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings(built_in("logback")));
    let mut records = Vec::new();
    let deadline = Instant::now() + LARGE_DEADLINE;
    loop {
        let progress = indexer.poll(&mut records);
        if progress.is_complete() {
            return;
        }
        if let Some(last) = records.last() {
            assert!(
                last.bytes().end <= progress.indexed,
                "{:?} handed over at {progress:?}",
                last.bytes()
            );
        }
        assert!(Instant::now() < deadline, "not complete: {progress:?}");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn progress_is_complete_only_at_the_end() {
    let bytes = large_log(built_in("logback"), LARGE);
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings(built_in("logback")));
    let seen = poll_to_the_end(&mut indexer, &mut Vec::new(), LARGE_DEADLINE);
    let (last, before) = seen.split_last().unwrap();
    assert!(last.is_complete());
    assert!(before.iter().all(|progress| !progress.is_complete()));
}

#[test]
fn entries_arrive_before_indexing_completes() {
    let bytes = large_log(built_in("logback"), LARGE);
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings(built_in("logback")));
    let mut records = Vec::new();
    let deadline = Instant::now() + LARGE_DEADLINE;
    loop {
        let progress = indexer.poll(&mut records);
        assert!(
            !progress.is_complete(),
            "no entries arrived before indexing completed"
        );
        if !records.is_empty() {
            return;
        }
        assert!(Instant::now() < deadline, "no entries: {progress:?}");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn entries_handed_over_before_completion_stay_as_they_are() {
    let bytes = large_log(built_in("logback"), LARGE);
    let settings = settings(built_in("logback"));
    let expected = index(&bytes, &settings);
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings);
    let mut records = Vec::new();
    let deadline = Instant::now() + LARGE_DEADLINE;
    while !indexer.poll(&mut records).is_complete() {
        assert_eq!(records, expected[..records.len()]);
        assert!(Instant::now() < deadline, "not complete");
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(records, expected);
}

#[test]
fn time_only_entries_get_their_timestamps_when_indexing_completes() {
    let bytes = large_log(built_in("logback-basic"), LARGE);
    let settings = settings(built_in("logback-basic"));
    let expected = index(&bytes, &settings);
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings);
    let mut records = Vec::new();
    let deadline = Instant::now() + LARGE_DEADLINE;
    while !indexer.poll(&mut records).is_complete() {
        for (record, expected) in records.iter().zip(&expected) {
            assert_eq!(record.timestamp(), None);
            assert_eq!(record.bytes(), expected.bytes());
            assert_eq!(record.level(), expected.level());
            assert_eq!(record.class(), expected.class());
            assert_eq!(record.has_exception(), expected.has_exception());
        }
        assert!(Instant::now() < deadline, "not complete");
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(records, expected);
}

#[test]
fn polling_after_completion_changes_nothing() {
    let bytes = mixed_log();
    let total = bytes.len() as u64;
    let mut indexer = Indexer::spawn(Arc::new(bytes), settings(built_in("logback")));
    let mut records = Vec::new();
    poll_to_the_end(&mut indexer, &mut records, DEADLINE);
    let complete = records.clone();
    assert_eq!(indexer.poll(&mut records), progress(total, total));
    assert_eq!(records, complete);
}

/// Blank lines under a preset they don't match: the slowest bytes to index, and
/// enough of them to keep the indexer busy far longer than [`PROMPTLY`], while the
/// whole file stays a single entry.
fn slow_file() -> Arc<Vec<u8>> {
    Arc::new(vec![b'\n'; 256 << 20])
}

/// Polls until some of the file is indexed, which takes a small part of a second.
/// Fails the test after [`LARGE_DEADLINE`].
fn wait_until_started(indexer: &mut Indexer) {
    let deadline = Instant::now() + LARGE_DEADLINE;
    let mut records = Vec::new();
    while indexer.poll(&mut records).indexed == 0 {
        assert!(Instant::now() < deadline, "indexing didn't start");
        thread::sleep(Duration::from_millis(1));
    }
}

/// Drops `indexer` on another thread, so that a drop that blocks can't hang the
/// test. The receiver hears when the drop has returned.
fn drop_elsewhere(indexer: Indexer) -> Receiver<()> {
    let (dropped, receiver) = mpsc::channel();
    thread::spawn(move || {
        drop(indexer);
        let _ = dropped.send(());
    });
    receiver
}

#[test]
fn dropping_the_indexer_mid_way_returns_promptly() {
    let mut indexer = Indexer::spawn(slow_file(), settings(built_in("logback")));
    wait_until_started(&mut indexer);
    let dropped = drop_elsewhere(indexer);
    assert!(
        dropped.recv_timeout(PROMPTLY).is_ok(),
        "dropping took longer than {PROMPTLY:?}"
    );
}

#[test]
fn dropping_the_indexer_mid_way_ends_its_threads_promptly() {
    let bytes = slow_file();
    let mut indexer = Indexer::spawn(Arc::clone(&bytes), settings(built_in("logback")));
    wait_until_started(&mut indexer);
    let dropped_at = Instant::now();
    let _dropped = drop_elsewhere(indexer);
    // The threads share the file until they end.
    while Arc::strong_count(&bytes) > 1 {
        assert!(
            dropped_at.elapsed() < PROMPTLY,
            "the indexer's threads still run {PROMPTLY:?} after it was dropped"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
