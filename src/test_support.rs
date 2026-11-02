//! Helpers shared by the unit tests.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::write::GzEncoder;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

use crate::app::{Action, App};
use crate::format::{self, BUILT_IN, EntryPattern, Format, Kind, Pattern};
use crate::keymap::Keymap;
use crate::source::{self, Source};

/// The built-in format `id` names.
pub fn built_in(id: &str) -> &'static Format {
    Format::by_id(id).unwrap_or_else(|| panic!("no built-in format {id}"))
}

/// Every built-in text format, in table order.
pub fn text_formats() -> Vec<&'static Format> {
    BUILT_IN
        .iter()
        .filter(|format| matches!(format.kind, Kind::Regex(_)))
        .collect()
}

/// A text format with a timestamp and a level but neither thread nor logger, like
/// Serilog's file sink without an offset:
/// `2026-10-07 12:00:00.123 [INF] Order 42 created`.
pub fn no_thread_or_logger() -> &'static Format {
    static PATTERN: Pattern = Pattern::new(|| {
        EntryPattern::from_regex(
            r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<level>[A-Z]+)\] ",
        )
    });
    static FORMAT: Format =
        format::text("no-thread-or-logger", "No thread or logger", &PATTERN, true);
    &FORMAT
}

/// A text format with a level and a logger but neither timestamp nor thread, like
/// Python's `logging.basicConfig()`: `WARNING:app.orders:Payment retry 1/3`.
pub fn no_timestamp() -> &'static Format {
    static PATTERN: Pattern =
        Pattern::new(|| EntryPattern::from_regex(r"^(?P<level>[A-Z]+):(?P<logger>[^:]*):"));
    static FORMAT: Format = format::text("no-timestamp", "No timestamp", &PATTERN, false);
    &FORMAT
}

/// A text format with a timestamp, a thread and a level but no logger, like
/// gunicorn's error log with the pid as its thread:
/// `2026-10-07 12:00:00.123 [1234] [INFO] Booting worker`.
pub fn no_logger() -> &'static Format {
    static PATTERN: Pattern = Pattern::new(|| {
        EntryPattern::from_regex(
            r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<thread>\d+)\] \[(?P<level>[A-Z]+)\] ",
        )
    });
    static FORMAT: Format = format::text("no-logger", "No logger", &PATTERN, true);
    &FORMAT
}

/// The example lines of PRD §6.1 by format id, one per built-in format but `raw`, in
/// table order.
pub const EXAMPLES: [(&str, &str); 6] = [
    (
        "logback",
        "2026-10-07 12:00:00.123 [http-nio-8080-exec-3] INFO  com.example.OrderService - Order 42 created",
    ),
    (
        "logback-basic",
        "12:00:00.123 [main] INFO  com.example.App -- Started",
    ),
    (
        "spring3",
        "2026-10-07T12:00:00.123+02:00  INFO 12345 --- [myapp] [           main] o.s.b.SpringApplication                  : Started App in 2.1s",
    ),
    (
        "spring2",
        "2026-10-07 12:00:00.123  INFO 12345 --- [           main] o.s.b.SpringApplication                  : Started App",
    ),
    (
        "wildfly",
        "2026-10-07 12:00:00,123 INFO  [org.jboss.as.server] (Controller Boot Thread) WFLYSRV0025: WildFly Full 33.0.0.Final started",
    ),
    (
        "wildfly-console",
        "12:00:00,123 INFO  [org.jboss.as] (MSC service thread 1-2) WFLYSRV0049: ...",
    ),
];

/// `content` mapped from a file. The file is gone by the time this returns; the
/// mapping stays valid.
pub fn source(content: &[u8]) -> Source {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("app.log");
    fs::write(&path, content).unwrap();
    source::open(&path).unwrap()
}

/// `bytes` as a gzip file, stored without compression: quick to make, and as big as
/// its content.
pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::none());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A viewer in `raw` over the gzip file `/var/log/app.log.gz` holding `gzip`, opened
/// as vilog opens a file by default, at the end. It hasn't taken in any entries yet.
pub fn gzip_app(gzip: &[u8]) -> App {
    App::new(
        Path::new("/var/log/app.log.gz"),
        source(gzip),
        Some(built_in("raw")),
        zone(),
        mtime(),
        true,
    )
}

/// The zone the test viewers read timestamps in.
pub fn zone() -> TimeZone {
    TimeZone::UTC
}

/// The modification time the test viewers' files have.
pub fn mtime() -> Timestamp {
    "2026-10-08T12:00:00Z".parse().unwrap()
}

/// A viewer over `content` in `raw`, opened at the top as `/var/log/app.log`, with
/// the whole file indexed.
pub fn app(content: &[u8]) -> App {
    named_app("/var/log/app.log", content)
}

/// A viewer over `content` in `raw`, opened at the top as `path`, with the whole
/// file indexed.
pub fn named_app(path: impl AsRef<Path>, content: &[u8]) -> App {
    let mut app = App::new(
        path.as_ref(),
        source(content),
        Some(built_in("raw")),
        zone(),
        mtime(),
        false,
    );
    wait_for_index(&mut app);
    app
}

/// A viewer over `content` in the format detected from it, opened at the top as
/// `/var/log/app.log`, with the whole file indexed.
pub fn detected_app(content: &[u8]) -> App {
    let mut app = opened(None, content, false);
    wait_for_index(&mut app);
    app
}

/// A viewer over `content` in `format`, opened at the top as `/var/log/app.log`,
/// with the whole file indexed.
pub fn app_in(format: &'static Format, content: &[u8]) -> App {
    let mut app = unindexed_app(format, content);
    wait_for_index(&mut app);
    app
}

/// A viewer over `content` in `format`, opened at the top as `/var/log/app.log`,
/// that hasn't taken in any entries yet.
pub fn unindexed_app(format: &'static Format, content: &[u8]) -> App {
    opened(Some(format), content, false)
}

/// A viewer over `content` in `format`, opened at the end as `/var/log/app.log` to
/// follow it, that hasn't taken in any entries yet.
pub fn following_app(format: &'static Format, content: &[u8]) -> App {
    opened(Some(format), content, true)
}

fn opened(format: Option<&'static Format>, content: &[u8], follow: bool) -> App {
    App::new(
        Path::new("/var/log/app.log"),
        source(content),
        format,
        zone(),
        mtime(),
        follow,
    )
}

/// How long indexing a test file of a few kilobytes may take before the test fails.
/// It takes milliseconds, even in a debug build.
pub const INDEX_DEADLINE: Duration = Duration::from_secs(1);

/// How long indexing a test file of a few MB may take before the test fails. It
/// takes well under a second on a few cores, even in a debug build, but more on a
/// two-core CI runner under coverage.
pub const LARGE_INDEX_DEADLINE: Duration = Duration::from_secs(5);

/// Polls `app`'s index until it is complete. Fails the test after
/// [`INDEX_DEADLINE`].
pub fn wait_for_index(app: &mut App) {
    wait_for_index_within(app, INDEX_DEADLINE);
}

/// Polls `app`'s index until it is complete. Fails the test after `within`.
pub fn wait_for_index_within(app: &mut App, within: Duration) {
    let deadline = Instant::now() + within;
    loop {
        app.poll_index();
        if app.progress().is_complete() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "indexing took longer than {within:?}: {:?}",
            app.progress()
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// Polls `app` until its index is complete and its filters' results are in. Fails
/// the test after [`INDEX_DEADLINE`].
pub fn wait_for_filter(app: &mut App) {
    let deadline = Instant::now() + INDEX_DEADLINE;
    loop {
        app.poll_index();
        if app.progress().is_complete() && !app.filtering() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "filtering took longer than {INDEX_DEADLINE:?}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// Polls `app` until its index is complete and the results of its filters and its
/// search are in. Fails the test after [`INDEX_DEADLINE`].
pub fn wait_for_search(app: &mut App) {
    let deadline = Instant::now() + INDEX_DEADLINE;
    loop {
        app.poll_index();
        if app.progress().is_complete() && !app.filtering() && !app.searching() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "searching took longer than {INDEX_DEADLINE:?}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// The bytes of the JSON lines fixture `tests/fixtures/json/<name>`.
pub fn json_fixture(name: &str) -> Vec<u8> {
    fixture("json", name)
}

/// The bytes of the logfmt fixture `tests/fixtures/logfmt/<name>`.
pub fn logfmt_fixture(name: &str) -> Vec<u8> {
    fixture("logfmt", name)
}

/// The bytes of the fixture `tests/fixtures/<dir>/<name>`.
fn fixture(dir: &str, name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir)
        .join(name);
    fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// `n` log lines that say which line they are, `… - step 1` to `… - step n`.
pub fn numbered_log(n: usize) -> String {
    (1..=n)
        .map(|i| format!("12:00:00.{i:03} [main] INFO  com.example.App - step {i}\n"))
        .collect()
}

/// A Logback log of two hours on 2026-10-07: an INFO entry `Tick` every minute from
/// 10:00 to 11:59, and among them a WARN at 10:12:30, two at 10:47 and 10:48, a
/// spike of 9 ERRORs at 11:03:00 to 11:03:08 and 5 more at 11:06:00 to 11:06:04, and
/// a WARN at 11:31:40.
pub fn spike_log() -> String {
    let mut entries: Vec<(u32, &str, &str)> = (0..120).map(|m| (m * 60, "INFO", "Tick")).collect();
    entries.push((12 * 60 + 30, "WARN", "Slow response"));
    entries.push((47 * 60 + 10, "WARN", "Slow response"));
    entries.push((48 * 60 + 20, "WARN", "Slow response"));
    entries.extend((0..9).map(|s| (63 * 60 + s, "ERROR", "Payment failed")));
    entries.extend((0..5).map(|s| (66 * 60 + s, "ERROR", "Payment failed")));
    entries.push((91 * 60 + 40, "WARN", "Slow response"));
    entries.sort_by_key(|&(second, ..)| second);
    entries
        .iter()
        .map(|(second, level, message)| {
            let (hour, minute, second) = (10 + second / 3600, second / 60 % 60, second % 60);
            format!(
                "2026-10-07 {hour:02}:{minute:02}:{second:02}.000 [main] {level:<5} com.example.App - {message}\n"
            )
        })
        .collect()
}

/// Hides `app`'s line numbers if they are shown, so that rows start with the `▶`
/// marker.
pub fn hide_line_numbers(app: &mut App) {
    if app.shows_line_numbers() {
        app.apply(Action::ToggleLineNumbers);
    }
}

/// The action the default keymap binds to `key`.
pub fn action(key: KeyEvent) -> Option<Action> {
    Keymap::default().action(key)
}

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn char_key(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

pub fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Screen line `y` as it looks, without trailing blanks: a wide character's second
/// column is not repeated.
pub fn row(terminal: &Terminal<TestBackend>, y: u16) -> String {
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    let mut x = 0;
    while x < buffer.area.width {
        let symbol = buffer[(x, y)].symbol();
        text.push_str(symbol);
        x += symbol.width().max(1) as u16;
    }
    text.trim_end().to_string()
}
