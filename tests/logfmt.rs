//! The logfmt fixtures read as logfmt: the keys chosen from each file, and each
//! entry's level, timestamp and exception in the index.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use vilog::format::{Anchor, Level, Mapping};
use vilog::index::{Chunk, Record, Settings};

mod common;

use common::{built_in, fixture, lines};

const FIXTURES: [&str; 2] = ["slog.log", "logrus.log"];

fn bytes(name: &str) -> Vec<u8> {
    fixture(&format!("logfmt/{name}"))
}

/// The records of the fixture `logfmt/<name>`, indexed with the mapping chosen from
/// it.
fn indexed(name: &str) -> Vec<Record> {
    let bytes = bytes(name);
    let settings = Settings {
        format: built_in("logfmt"),
        zone: TimeZone::UTC,
        anchor: Anchor::Named("2026-03-14".parse().unwrap()),
        mapping: Mapping::choose(lines(&bytes)),
    };
    Chunk::index(&bytes, 0, 0..bytes.len(), &settings).finish(&settings)
}

/// Both libraries write `time`, `level` and `msg`, and neither has a logger or a
/// thread by default.
#[test]
fn every_logfmt_fixture_maps_the_keys_its_library_writes() {
    let expected = Mapping {
        timestamp: Some("time"),
        level: Some("level"),
        logger: None,
        thread: None,
        message: Some("msg"),
    };
    for name in FIXTURES {
        assert_eq!(Mapping::choose(lines(&bytes(name))), expected, "{name}");
    }
}

const T: Option<Level> = Some(Level::Trace);
const D: Option<Level> = Some(Level::Debug);
const I: Option<Level> = Some(Level::Info);
const W: Option<Level> = Some(Level::Warn);
const E: Option<Level> = Some(Level::Error);
const F: Option<Level> = Some(Level::Fatal);

/// slog writes capitals, logrus lower case with `warning` and `panic`. The lines Go
/// prints when logrus panics continue its last entry.
#[test]
fn levels_of_every_logfmt_fixture_come_from_its_level_key() {
    let expected: [(&str, &[Option<Level>]); 2] = [
        ("slog.log", &[I, D, I, I, W, E, I, I, I]),
        ("logrus.log", &[I, D, I, W, E, I, T, F]),
    ];
    for (name, levels) in expected {
        let read: Vec<_> = indexed(name).iter().map(Record::level).collect();
        assert_eq!(read, levels, "{name}");
    }
}

/// slog's unquoted time with milliseconds and logrus' quoted time in seconds. Each
/// entry has a timestamp of its own, later than the one before, the cut-off last
/// line of slog's included.
#[test]
fn timestamps_of_every_logfmt_fixture_come_from_its_timestamp_key() {
    let expected = [
        ("slog.log", "2026-03-14T08:12:01.104Z"),
        ("logrus.log", "2026-03-14T08:12:01Z"),
    ];
    for (name, first) in expected {
        let read: Vec<Option<Timestamp>> = indexed(name).iter().map(Record::timestamp).collect();
        assert_eq!(read[0], Some(first.parse().unwrap()), "{name}");
        assert!(
            read.windows(2).all(|pair| pair[0] < pair[1]),
            "{name}: {read:?}"
        );
    }
}

/// slog's error is an ordinary field; logrus' panic is followed by Go's stack trace.
#[test]
fn exceptions_of_every_logfmt_fixture_come_from_the_lines_after_an_entry() {
    let expected: [(&str, &[usize]); 2] = [("slog.log", &[]), ("logrus.log", &[7])];
    for (name, entries) in expected {
        let with_exception: Vec<usize> = indexed(name)
            .iter()
            .enumerate()
            .filter(|(_, record)| record.has_exception())
            .map(|(entry, _)| entry)
            .collect();
        assert_eq!(with_exception, entries, "{name}");
    }
}
