//! Format detection on the fixture logs in `tests/fixtures`: one per text preset, one
//! per library in `json/` and `logfmt/`.

use std::fs;
use std::path::Path;

use vilog::format::{Format, Mapping, detect};

mod common;

use common::{built_in, fixture, fixtures, lines};

/// What detection makes of a file with these bytes, split into lines as the viewer
/// splits them.
fn detect_bytes(bytes: &[u8]) -> Option<&'static Format> {
    detect(lines(bytes))
}

/// Every fixture log with the format it is in: the text presets' own, a Logback log
/// whose messages and configuration dump are `key=value` pairs, and every file in
/// `json/` and `logfmt/`.
fn every_fixture() -> Vec<(&'static Format, String)> {
    let mut all: Vec<_> = fixtures()
        .map(|(format, name)| (format, name.to_string()))
        .collect();
    all.push((built_in("logback"), "logback-key-values.log".to_string()));
    for dir in ["json", "logfmt"] {
        all.extend(files_in(dir).map(|name| (built_in(dir), format!("{dir}/{name}"))));
    }
    all
}

/// The names of the `.log` files in `tests/fixtures/<dir>`, sorted.
fn files_in(dir: &str) -> impl Iterator<Item = String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir);
    let mut names: Vec<String> = fs::read_dir(&path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .map(|entry| entry.expect("a readable directory").file_name())
        .map(|name| name.into_string().expect("fixture names are UTF-8"))
        .filter(|name| name.ends_with(".log"))
        .collect();
    names.sort();
    names.into_iter()
}

#[test]
fn every_fixture_is_detected_as_its_own_format() {
    for (format, name) in every_fixture() {
        assert_eq!(detect_bytes(&fixture(&name)), Some(format), "{name}");
    }
}

/// The JSON fixtures in `tests/fixtures/json` and the mapping each is read with:
/// timestamp, level, logger, thread and message key.
const JSON_FIXTURES: [(&str, [Option<&str>; 5]); 11] = [
    (
        "zap",
        [Some("ts"), Some("level"), Some("logger"), None, Some("msg")],
    ),
    (
        "slog",
        [Some("time"), Some("level"), None, None, Some("msg")],
    ),
    (
        "pino",
        [Some("time"), Some("level"), Some("name"), None, Some("msg")],
    ),
    (
        "winston",
        [
            Some("timestamp"),
            Some("level"),
            None,
            None,
            Some("message"),
        ],
    ),
    (
        "structlog",
        [
            Some("timestamp"),
            Some("level"),
            Some("logger"),
            None,
            Some("event"),
        ],
    ),
    (
        "python-json-logger",
        [
            Some("asctime"),
            Some("levelname"),
            Some("name"),
            None,
            Some("message"),
        ],
    ),
    (
        "serilog-clef",
        [
            Some("@t"),
            Some("@l"),
            Some("SourceContext"),
            None,
            Some("@mt"),
        ],
    ),
    (
        "ms-json-console",
        [
            Some("Timestamp"),
            Some("LogLevel"),
            Some("Category"),
            None,
            Some("Message"),
        ],
    ),
    (
        "logstash-logback",
        [
            Some("@timestamp"),
            Some("level"),
            Some("logger_name"),
            Some("thread_name"),
            Some("message"),
        ],
    ),
    (
        "ecs",
        [
            Some("@timestamp"),
            Some("log.level"),
            Some("log.logger"),
            Some("process.thread.name"),
            Some("message"),
        ],
    ),
    (
        "docker-json-file",
        [Some("time"), None, None, None, Some("log")],
    ),
];

#[test]
fn every_json_fixture_maps_the_keys_its_library_writes() {
    for (name, [timestamp, level, logger, thread, message]) in JSON_FIXTURES {
        let bytes = fixture(&format!("json/{name}.log"));
        let expected = Mapping {
            timestamp,
            level,
            logger,
            thread,
            message,
        };
        assert_eq!(Mapping::choose(lines(&bytes)), expected, "{name}");
    }
}

#[test]
fn every_fixture_with_crlf_line_endings_is_detected_as_its_own_format() {
    for (format, name) in every_fixture() {
        let text = String::from_utf8(fixture(&name)).expect("fixtures are UTF-8");
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(detect_bytes(crlf.as_bytes()), Some(format), "{name}");
    }
}

#[test]
fn every_fixture_in_colour_is_detected_as_its_own_format() {
    for (format, name) in every_fixture() {
        let bytes = fixture(&name);
        let coloured: Vec<Vec<u8>> = lines(&bytes)
            .into_iter()
            .map(|line| [b"\x1b[1;36m".as_slice(), line, b"\x1b[0m"].concat())
            .collect();
        let detection = detect(coloured.iter().map(Vec::as_slice));
        assert_eq!(detection, Some(format), "{name}");
    }
}

#[test]
fn no_fixture_is_recognized_after_200_lines_of_jvm_startup_noise() {
    let noise =
        "OpenJDK 64-Bit Server VM warning: Sharing is only supported for boot loader classes\n"
            .repeat(200);
    for (_, name) in every_fixture() {
        let bytes = [noise.as_bytes(), &fixture(&name)].concat();
        let detection = detect_bytes(&bytes);
        assert_eq!(detection, None, "{name}");
    }
}

/// Traces of Python, Go, Node.js and .NET, as a log has them when it records what a
/// child process printed, are left out of the sample like Java's, every line of them.
#[test]
fn every_fixture_with_traces_of_every_ecosystem_mixed_in_is_detected_as_its_own_format() {
    let traces = fixture("traces.log");
    let traces = continuation_lines(&traces);
    for (format, name) in every_fixture() {
        let bytes = fixture(&name);
        let mixed: Vec<&[u8]> = lines(&bytes)
            .chunks(TRACE_EVERY)
            .flat_map(|chunk| [chunk, &traces].concat())
            .collect();
        assert_eq!(detect(mixed), Some(format), "{name}");
    }
}

/// How many fixture lines come before each copy of the traces.
const TRACE_EVERY: usize = 30;

/// The lines of `bytes` that don't start a Logback entry.
fn continuation_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let parser = built_in("logback")
        .entry_parser()
        .expect("logback has entries");
    let mut lines = lines(bytes);
    lines.retain(|line| parser.parse(line).is_none());
    lines
}
