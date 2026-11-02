//! The log formats vilog knows, as data, and how to recognise the first line of an
//! entry in each.

use std::fmt;
use std::ops::Range;
use std::sync::LazyLock;

use clap::ValueEnum;
use clap::builder::PossibleValue;
use jiff::tz::TimeZone;
use regex::bytes::{CaptureLocations, Regex};

use crate::format::mapping::lookup_in;
use crate::format::{LogTime, Mapping, Syntax, time};

#[cfg(test)]
mod tests;

/// A log format: how to find where an entry starts and what its first line holds.
/// Built-in formats live in [`BUILT_IN`] for the whole run, so a format is passed
/// around as `&'static Format`.
pub struct Format {
    /// The name `--format` takes: `logback`, `logback-basic`, …, `raw`.
    pub id: &'static str,
    /// The name the header shows: `Logback`, `Logback basic`, …, `Raw`.
    pub name: &'static str,
    pub kind: Kind,
}

/// How a format reads a line.
#[derive(Debug)]
pub enum Kind {
    /// A regex over the start of a line finds an entry's first line and its columns.
    Regex(Layout),
    /// One JSON object per line; which keys are the columns is chosen per file, see
    /// [`Mapping`].
    Json,
    /// `key=value` pairs, one entry per line that starts with a key and its `=`; the
    /// columns come from the keys a [`Mapping`] chooses, as for [`Json`](Kind::Json).
    Logfmt,
    /// No structure: every line is an entry of its own.
    Raw,
}

/// A text format's entry-start regex and what its timestamps hold.
#[derive(Debug)]
pub struct Layout {
    pattern: &'static Pattern,
    /// Whether the timestamp has a date. Without one it is only a time of day.
    has_date: bool,
}

/// The built-in formats in a fixed order: within a family the one with a date comes
/// before the time-only one, and `raw` comes last. Detection breaks ties in this
/// order and `P` steps through it.
///
/// - `logback`, Logback / Log4j2 classic:
///   `2026-10-07 12:00:00.123 [main] INFO  com.example.App - Started`
/// - `logback-basic`, Logback's BasicConfigurator, time only:
///   `12:00:00.123 [main] INFO  com.example.App -- Started`
/// - `spring3`, Spring Boot 3, the `[app]` group is optional:
///   `2026-10-07T12:00:00.123+02:00  INFO 12345 --- [app] [           main] o.s.b.SpringApplication                  : Started`
/// - `spring2`, Spring Boot 2:
///   `2026-10-07 12:00:00.123  INFO 12345 --- [           main] o.s.b.SpringApplication                  : Started`
/// - `wildfly`, WildFly `server.log`:
///   `2026-10-07 12:00:00,123 INFO  [org.jboss.as.server] (Controller Boot Thread) WFLYSRV0025: started`
/// - `wildfly-console`, WildFly on the console, time only and often coloured:
///   `12:00:00,123 INFO  [org.jboss.as] (MSC service thread 1-2) WFLYSRV0049: starting`
/// - `json`, one JSON object per line: `{"time":"2026-10-07T12:00:00.123Z","level":"INFO","msg":"Started"}`
/// - `logfmt`, `key=value` pairs: `time=2026-10-07T12:00:00.123Z level=INFO msg=Started`
/// - `raw`, no structure: every line is an entry of its own.
pub static BUILT_IN: &[Format] = &[
    text("logback", "Logback", &LOGBACK, true),
    text("logback-basic", "Logback basic", &LOGBACK_BASIC, false),
    text("spring3", "Spring Boot 3", &SPRING3, true),
    text("spring2", "Spring Boot 2", &SPRING2, true),
    text("wildfly", "WildFly", &WILDFLY, true),
    text(
        "wildfly-console",
        "WildFly console",
        &WILDFLY_CONSOLE,
        false,
    ),
    Format {
        id: "json",
        name: "JSON",
        kind: Kind::Json,
    },
    Format {
        id: "logfmt",
        name: "logfmt",
        kind: Kind::Logfmt,
    },
    Format {
        id: "raw",
        name: "Raw",
        kind: Kind::Raw,
    },
];

/// A text format that recognises its entries with `pattern`, which also says which
/// columns it has. Built-in formats are made with it, and so are the ones a test
/// defines for itself.
pub(crate) const fn text(
    id: &'static str,
    name: &'static str,
    pattern: &'static Pattern,
    has_date: bool,
) -> Format {
    Format {
        id,
        name,
        kind: Kind::Regex(Layout { pattern, has_date }),
    }
}

static LOGBACK: Pattern = Pattern::new(|| EntryPattern::new(DATE_TIME, LOGBACK_COLUMNS));
static LOGBACK_BASIC: Pattern = Pattern::new(|| EntryPattern::new(TIME, LOGBACK_COLUMNS));
static SPRING3: Pattern = Pattern::new(|| EntryPattern::new(ISO_DATE_TIME, SPRING3_COLUMNS));
static SPRING2: Pattern = Pattern::new(|| EntryPattern::new(DATE_TIME, SPRING2_COLUMNS));
static WILDFLY: Pattern = Pattern::new(|| EntryPattern::new(DATE_TIME, WILDFLY_COLUMNS));
static WILDFLY_CONSOLE: Pattern = Pattern::new(|| EntryPattern::new(TIME, WILDFLY_COLUMNS));

/// A column an entry's first line may have besides its message. A format has the
/// columns its pattern has a group for; `raw` has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Timestamp,
    Level,
    Thread,
    Logger,
}

impl Kind {
    /// How the lines of a structured format write their fields; `None` for text.
    pub fn syntax(&self) -> Option<Syntax> {
        match self {
            Kind::Json => Some(Syntax::Json),
            Kind::Logfmt => Some(Syntax::Logfmt),
            Kind::Regex(_) | Kind::Raw => None,
        }
    }
}

impl Format {
    /// Whether the format's entries have `column`. The table leaves out the columns
    /// a format doesn't have.
    pub fn has(&self, column: Column) -> bool {
        match &self.kind {
            Kind::Regex(layout) => layout.pattern.groups.has(column),
            // Which columns a file has depends on its mapping.
            Kind::Json | Kind::Logfmt => true,
            Kind::Raw => false,
        }
    }

    /// The built-in format whose [`id`](Self::id) is `id`, e.g. `logback-basic`.
    /// Letter case counts. `None` for any other name.
    pub fn by_id(id: &str) -> Option<&'static Format> {
        BUILT_IN.iter().find(|format| format.id == id)
    }

    /// What recognises this format's entry-start lines. `raw` has none: it doesn't
    /// group lines, so whoever reads a file under `raw` treats every line as an entry.
    pub fn entry_parser(&'static self) -> Option<EntryParser> {
        match &self.kind {
            Kind::Regex(layout) => Some(EntryParser {
                source: Source::Text {
                    layout,
                    pattern: layout.pattern,
                },
            }),
            Kind::Json => Some(EntryParser::json(Mapping::default())),
            Kind::Logfmt => Some(EntryParser::logfmt(Mapping::default())),
            Kind::Raw => None,
        }
    }
}

/// Formats are the same when their ids are.
impl PartialEq for Format {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Format {}

/// Only the id, so that a format in a test failure doesn't show its whole regex.
impl fmt::Debug for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Format").field(&self.id).finish()
    }
}

/// `--format` takes the ids of [`BUILT_IN`], in table order. Any other value is an
/// invalid value, and the error lists the valid ones.
impl ValueEnum for &'static Format {
    fn value_variants<'a>() -> &'a [Self] {
        // `&'static Format` is a pointer to the table's own elements, so a slice of
        // them is built once.
        static VARIANTS: LazyLock<Vec<&'static Format>> =
            LazyLock::new(|| BUILT_IN.iter().collect());
        &VARIANTS
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self.id))
    }
}

/// Recognises the first line of an entry in one format and finds its columns.
#[derive(Debug, Clone, Copy)]
pub struct EntryParser {
    source: Source,
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Text {
        layout: &'static Layout,
        // Forced once here, so that `parse` doesn't go through the LazyLock on every line.
        pattern: &'static EntryPattern,
    },
    Structured(Syntax, Mapping),
}

impl EntryParser {
    /// The parser of JSON lines read with `mapping`. A line that is a JSON object,
    /// starting with `{` after any spaces and tabs or a UTF-8 byte order mark, starts
    /// an entry; any other line continues one. The message is the value of the mapped
    /// message key, the text between its quotes for a string without a trailing `\n`
    /// escape, and the whole line when the line or the mapping has no message key.
    pub fn json(mapping: Mapping) -> EntryParser {
        EntryParser {
            source: Source::Structured(Syntax::Json, mapping),
        }
    }

    /// The parser of logfmt lines read with `mapping`. A line that starts with a key
    /// and its `=` at its first byte starts an entry; any other line continues one.
    /// The message is the value of the mapped message key, between its quotes if it
    /// has them, and the whole line when the line or the mapping has no message key.
    pub fn logfmt(mapping: Mapping) -> EntryParser {
        EntryParser {
            source: Source::Structured(Syntax::Logfmt, mapping),
        }
    }

    /// The columns of `line` if it starts an entry, else `None`. `line` has no line
    /// ending and is already ANSI-stripped; the spans are byte ranges into it.
    /// Only the layout counts: a line with an impossible date such as `2026-13-45`
    /// still starts an entry, and so does one whose level is a word [`Level`] doesn't
    /// know, such as `AUDIT`.
    ///
    /// [`Level`]: crate::format::Level
    pub fn parse(self, line: &[u8]) -> Option<EntryHead> {
        self.parse_with(line, &mut self.locations())
    }

    /// Room for [`parse_with`](Self::parse_with) to find the columns in.
    pub(crate) fn locations(self) -> CaptureLocations {
        match self.source {
            Source::Text { pattern, .. } => pattern.regex.capture_locations(),
            // Structured lines have no regex groups to find.
            Source::Structured(..) => NO_GROUPS.capture_locations(),
        }
    }

    /// [`parse`](Self::parse) in `locations`, which [`locations`](Self::locations)
    /// gave and which are reused from line to line. Fresh ones cost an allocation and
    /// an update of a reference count that every thread parsing with this format
    /// shares, which keeps parsing on many threads at once from getting any faster.
    pub(crate) fn parse_with(
        self,
        line: &[u8],
        locations: &mut CaptureLocations,
    ) -> Option<EntryHead> {
        match self.source {
            Source::Text { pattern, .. } => parse_text(pattern, line, locations),
            Source::Structured(syntax, mapping) => parse_structured(syntax, mapping, line),
        }
    }

    /// Whether the timestamp has a date. Without one (`logback-basic`,
    /// `wildfly-console`) it is only a time of day.
    pub fn has_date(self) -> bool {
        match self.source {
            Source::Text { layout, .. } => layout.has_date,
            Source::Structured(..) => true,
        }
    }

    /// What the timestamp span `span` (the bytes of [`EntryHead::timestamp`]) says, as
    /// [`time::read`] reads it.
    pub fn timestamp(self, span: &[u8], zone: &TimeZone) -> Option<LogTime> {
        time::read(span, zone)
    }
}

static NO_GROUPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("").expect("the empty regex is valid"));

fn parse_text(
    pattern: &EntryPattern,
    line: &[u8],
    locations: &mut CaptureLocations,
) -> Option<EntryHead> {
    let matched = pattern.regex.captures_read(locations, line)?;
    let span = |group: Option<usize>| {
        let (start, end) = locations.get(group?)?;
        Some(start..end)
    };
    let groups = &pattern.groups;
    Some(EntryHead {
        timestamp: span(groups.timestamp),
        level: span(groups.level),
        thread: span(groups.thread),
        logger: span(groups.logger),
        // Matching the message too would cost time proportional to its length on
        // every line, for a span that is known anyway.
        message: matched.end()..line.len(),
    })
}

/// Where a string ending at `text`'s end stops once a trailing `\n` escape is dropped,
/// as Docker's json-file driver keeps the newline the container printed. The `n` only
/// escapes when an odd number of backslashes precede it: `\\n` is a backslash and an `n`.
fn end_before_newline_escape(text: &[u8]) -> usize {
    let Some(rest) = text.strip_suffix(b"n") else {
        return text.len();
    };
    let backslashes = rest.iter().rev().take_while(|&&byte| byte == b'\\').count();
    if backslashes % 2 == 1 {
        rest.len() - 1
    } else {
        text.len()
    }
}

fn parse_structured(syntax: Syntax, mapping: Mapping, line: &[u8]) -> Option<EntryHead> {
    let at = syntax.start(line)?;
    let body = &line[at..];
    let message = mapping
        .message
        .and_then(|key| lookup_in(syntax, body, key))
        .map_or(0..line.len(), |field| {
            // Docker's json-file driver is the only one to keep the newline.
            let end = if field.string && syntax == Syntax::Json {
                end_before_newline_escape(&body[..field.value.end])
            } else {
                field.value.end
            };
            field.value.start + at..end + at
        });
    Some(EntryHead {
        timestamp: None,
        level: None,
        thread: None,
        logger: None,
        message,
    })
}

/// An [`EntryPattern`] compiled the first time it is used, so that a format can be
/// a `static`.
pub(crate) type Pattern = LazyLock<EntryPattern>;

/// A text format's entry-start regex and which of its groups holds each column.
#[derive(Debug)]
pub(crate) struct EntryPattern {
    regex: Regex,
    groups: ColumnGroups,
}

/// Group indices, so that a match isn't searched for the columns by name.
#[derive(Debug)]
struct ColumnGroups {
    timestamp: Option<usize>,
    level: Option<usize>,
    thread: Option<usize>,
    logger: Option<usize>,
}

impl ColumnGroups {
    fn of(regex: &Regex) -> ColumnGroups {
        let group = |name| regex.capture_names().position(|group| group == Some(name));
        ColumnGroups {
            timestamp: group("timestamp"),
            level: group("level"),
            thread: group("thread"),
            logger: group("logger"),
        }
    }

    fn has(&self, column: Column) -> bool {
        match column {
            Column::Timestamp => self.timestamp.is_some(),
            Column::Level => self.level.is_some(),
            Column::Thread => self.thread.is_some(),
            Column::Logger => self.logger.is_some(),
        }
    }
}

impl EntryPattern {
    /// The pattern of `regex`, a byte regex that matches an entry's first line from
    /// its start. Its named groups `timestamp`, `level`, `thread` and `logger` are the
    /// columns, each of them optional; the message is what follows the match.
    ///
    /// # Panics
    ///
    /// If `regex` isn't a valid regex.
    #[cfg(test)]
    pub(crate) fn from_regex(regex: &str) -> EntryPattern {
        let regex =
            Regex::new(regex).unwrap_or_else(|error| panic!("invalid entry regex: {error}"));
        let groups = ColumnGroups::of(&regex);
        EntryPattern { regex, groups }
    }

    fn new(timestamp: &str, columns: &str) -> EntryPattern {
        let regex = entry_regex(timestamp, columns);
        let groups = ColumnGroups::of(&regex);
        EntryPattern { regex, groups }
    }
}

// The patterns are byte patterns without Unicode (`-u`), so a line that isn't UTF-8
// still parses, and `.` takes any byte (`s`). Fractions of a second have 1 to 9
// digits after a `.` or a `,`. Dates and times are only checked for their shape:
// an impossible one still starts an entry.
const TIME: &str = r"\d{2}:\d{2}:\d{2}[.,]\d{1,9}";
const DATE_TIME: &str = r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}[.,]\d{1,9}";
const ISO_DATE_TIME: &str = r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[.,]\d{1,9}(?:Z|[+-]\d{2}:\d{2})";

// What lies between the timestamp and the separator before the message.

/// A thread in brackets runs up to the first `] `: it may hold a `]` of its own as
/// long as no space follows it. Spring 3 relies on this to tell the thread from the
/// groups around it.
macro_rules! bracketed_thread {
    () => {
        r"(?P<thread>(?:[^\]]|\]+[^\] ])+\]*)\] "
    };
}

// %d [%thread] %-5level %logger - %msg, with `--` in BasicConfigurator.
const LOGBACK_COLUMNS: &str = concat!(
    r" \[",
    bracketed_thread!(),
    r"(?P<level>[A-Za-z]+) +(?P<logger>\S+) +--?"
);
// %d %5p PID --- [app] [%15.15t] [correlation] %-40.40logger : %m
// Spring Boot 3.2 added the `[app]` group and, with tracing, the correlation group:
// `[trace-span] `, padded with spaces where an id is missing, all blank outside a
// span. The app group is lazy: without an app the thread's bracket would otherwise
// pass for one, and the thread can't swallow a following `] ` to make up for it.
const SPRING3_COLUMNS: &str = concat!(
    r" +(?P<level>[A-Za-z]+) \d+ --- (?:\[[^\]]*\] )??\[ *",
    bracketed_thread!(),
    r"(?:\[[0-9a-f ]{32}[- ][0-9a-f ]{16}\] )?(?P<logger>\S+) +:"
);
// %d %5p PID --- [%15.15t] %-40.40logger : %m
const SPRING2_COLUMNS: &str = concat!(
    r" +(?P<level>[A-Za-z]+) \d+ --- \[ *",
    bracketed_thread!(),
    r"(?P<logger>\S+) +:"
);
// %d %-5p [%c] (%t) %s%e. The thread is lazy: it ends at the first `) `.
const WILDFLY_COLUMNS: &str = r" (?P<level>[A-Za-z]+) +\[(?P<logger>.+?)\] \((?P<thread>.+?)\)";

/// An entry starts at column 0 with a timestamp. Its message starts after the space
/// that ends the match; an empty one may have lost that space to trimming.
fn entry_regex(timestamp: &str, columns: &str) -> Regex {
    let pattern = format!(r"(?s-u)^(?P<timestamp>{timestamp}){columns}(?: |$)");
    Regex::new(&pattern).expect("the built-in patterns are valid regexes")
}

/// The columns of an entry's first line, as byte ranges into that line. Padding
/// around a column (`INFO `, `[           main]`, a logger padded to 40) is not part
/// of it. A column the format doesn't have is `None`. The message runs to the end of
/// the line and may be empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryHead {
    pub timestamp: Option<Range<usize>>,
    pub level: Option<Range<usize>>,
    pub thread: Option<Range<usize>>,
    pub logger: Option<Range<usize>>,
    pub message: Range<usize>,
}
