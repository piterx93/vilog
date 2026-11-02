use std::collections::HashSet;
use std::ops::Range;

use proptest::prelude::*;

use super::*;
use crate::format::strip_ansi;
use crate::test_support::{EXAMPLES, built_in, no_thread_or_logger, no_timestamp, text_formats};

/// Spring Boot's default correlation group with a trace and a span: `traceId(32)`,
/// `spanId(16)`.
const TRACE_AND_SPAN: &str = "6bfb3c0e8f1d4a2b9c7e5d3f1a2b4c6d-1a2b4c6d8e0f2a4b";

/// The same group outside a span: blank, as wide as the ids and the space between.
fn blank_correlation() -> String {
    " ".repeat(32 + 1 + 16)
}

fn parser(preset: &'static Format) -> EntryParser {
    preset
        .entry_parser()
        .unwrap_or_else(|| panic!("{} has no entry parser", preset.id))
}

fn parse(preset: &'static Format, line: &[u8]) -> Option<EntryHead> {
    parser(preset).parse(line)
}

/// The columns `preset` finds in `line` as text: timestamp, level, thread, logger,
/// message.
fn columns(preset: &'static Format, line: &str) -> [String; 5] {
    let head = parse(preset, line.as_bytes())
        .unwrap_or_else(|| panic!("{} doesn't parse {line:?}", preset.id));
    [
        head.timestamp,
        head.level,
        head.thread,
        head.logger,
        Some(head.message),
    ]
    .map(|span| {
        let span = span.expect("the built-in formats have every column");
        String::from_utf8_lossy(&line.as_bytes()[span]).into_owned()
    })
}

/// What goes into an entry's first line, to write it the way each preset does.
#[derive(Debug, Clone)]
struct Parts {
    date: String,
    time: String,
    separator: char,
    fraction: String,
    /// Spring 3 only.
    offset: String,
    level: String,
    /// Spring only.
    pid: String,
    /// Spring 3 only.
    app: Option<String>,
    /// Spring 3 only: the correlation group Micrometer Tracing adds after the thread,
    /// without its brackets.
    correlation: Option<String>,
    thread: String,
    class: String,
    /// Logback only: `%-40logger` instead of `%logger`. Spring always pads to 40.
    pad_class: bool,
    /// Logback only: `-` or `--`.
    dashes: &'static str,
    message: String,
}

impl Default for Parts {
    fn default() -> Self {
        Self {
            date: "2026-10-07".into(),
            time: "12:00:00".into(),
            separator: '.',
            fraction: "123".into(),
            offset: "+02:00".into(),
            level: "INFO".into(),
            pid: "12345".into(),
            app: Some("myapp".into()),
            correlation: None,
            thread: "main".into(),
            class: "com.example.App".into(),
            pad_class: false,
            dashes: "-",
            message: "Started".into(),
        }
    }
}

/// A line being written, piece by piece.
#[derive(Default)]
struct Line(String);

impl Line {
    /// Appends `text` and returns where it landed.
    fn push(&mut self, text: &str) -> Range<usize> {
        let start = self.0.len();
        self.0.push_str(text);
        start..self.0.len()
    }
}

/// Spaces that pad `text` to `width` characters.
fn padding(text: &str, width: usize) -> String {
    " ".repeat(width.saturating_sub(text.chars().count()))
}

/// `parts` written as `preset`'s pattern writes an entry's first line, with the
/// columns `parse` should find in it.
fn entry_line(preset: &'static Format, parts: &Parts) -> (String, EntryHead) {
    let p = parts;
    let time = format!("{}{}{}", p.time, p.separator, p.fraction);
    let mut line = Line::default();
    let head = match preset.id {
        // %d [%thread] %-5level %logger - %msg
        "logback" | "logback-basic" => {
            let timestamp = match preset.id {
                "logback" => line.push(&format!("{} {time}", p.date)),
                _ => line.push(&time),
            };
            line.push(" [");
            let thread = line.push(&p.thread);
            line.push("] ");
            let level = line.push(&p.level);
            line.push(&padding(&p.level, 5));
            line.push(" ");
            let class = line.push(&p.class);
            if p.pad_class {
                line.push(&padding(&p.class, 40));
            }
            line.push(&format!(" {} ", p.dashes));
            let message = line.push(&p.message);
            EntryHead {
                timestamp: Some(timestamp),
                level: Some(level),
                thread: Some(thread),
                logger: Some(class),
                message,
            }
        }
        // %d %5p PID --- [app] [%15.15t] [correlation] %-40.40logger : %m
        "spring3" | "spring2" => {
            let timestamp = match preset.id {
                "spring3" => line.push(&format!("{}T{time}{}", p.date, p.offset)),
                _ => line.push(&format!("{} {time}", p.date)),
            };
            line.push(" ");
            line.push(&padding(&p.level, 5));
            let level = line.push(&p.level);
            line.push(&format!(" {} --- ", p.pid));
            if let ("spring3", Some(app)) = (preset.id, &p.app) {
                line.push(&format!("[{app}] "));
            }
            line.push("[");
            line.push(&padding(&p.thread, 15));
            let thread = line.push(&p.thread);
            line.push("] ");
            if let ("spring3", Some(correlation)) = (preset.id, &p.correlation) {
                line.push(&format!("[{correlation}] "));
            }
            let class = line.push(&p.class);
            line.push(&padding(&p.class, 40));
            line.push(" : ");
            let message = line.push(&p.message);
            EntryHead {
                timestamp: Some(timestamp),
                level: Some(level),
                thread: Some(thread),
                logger: Some(class),
                message,
            }
        }
        // %d %-5p [%c] (%t) %s%e
        "wildfly" | "wildfly-console" => {
            let timestamp = match preset.id {
                "wildfly" => line.push(&format!("{} {time}", p.date)),
                _ => line.push(&time),
            };
            line.push(" ");
            let level = line.push(&p.level);
            line.push(&padding(&p.level, 5));
            line.push(" [");
            let class = line.push(&p.class);
            line.push("] (");
            let thread = line.push(&p.thread);
            line.push(") ");
            let message = line.push(&p.message);
            EntryHead {
                timestamp: Some(timestamp),
                level: Some(level),
                thread: Some(thread),
                logger: Some(class),
                message,
            }
        }
        other => panic!("no entry lines written for {other}"),
    };
    (line.0, head)
}

#[track_caller]
fn assert_parses(preset: &'static Format, parts: &Parts) {
    let (line, head) = entry_line(preset, parts);
    assert_eq!(
        parse(preset, line.as_bytes()),
        Some(head),
        "{}: {line:?}",
        preset.id
    );
}

#[track_caller]
fn assert_every_preset_parses(parts: &Parts) {
    for preset in text_formats() {
        assert_parses(preset, parts);
    }
}

// --- the table ---

const ALL_COLUMNS: [Column; 4] = [
    Column::Timestamp,
    Column::Level,
    Column::Thread,
    Column::Logger,
];

/// The columns `format` has, in the order of [`ALL_COLUMNS`].
fn columns_of(format: &'static Format) -> Vec<Column> {
    ALL_COLUMNS
        .into_iter()
        .filter(|&column| format.has(column))
        .collect()
}

#[test]
fn every_built_in_text_format_has_every_column() {
    for format in text_formats() {
        assert_eq!(columns_of(format), ALL_COLUMNS, "{format:?}");
    }
}

#[test]
fn raw_has_no_columns() {
    assert_eq!(columns_of(built_in("raw")), []);
}

#[test]
fn a_format_has_the_columns_its_pattern_has_groups_for() {
    assert_eq!(
        columns_of(no_thread_or_logger()),
        [Column::Timestamp, Column::Level]
    );
    assert_eq!(columns_of(no_timestamp()), [Column::Level, Column::Logger]);
}

#[test]
fn a_column_the_format_lacks_has_no_span() {
    let line = "WARNING:app.orders:Payment retry 1/3";
    assert_eq!(
        parse(no_timestamp(), line.as_bytes()),
        Some(EntryHead {
            timestamp: None,
            level: Some(0..7),
            thread: None,
            logger: Some(8..18),
            message: 19..line.len(),
        })
    );
}

#[test]
fn every_format_has_a_name_for_the_header() {
    let names: Vec<&str> = BUILT_IN.iter().map(|format| format.name).collect();
    assert_eq!(
        names,
        [
            "Logback",
            "Logback basic",
            "Spring Boot 3",
            "Spring Boot 2",
            "WildFly",
            "WildFly console",
            "JSON",
            "logfmt",
            "Raw",
        ]
    );
}

#[test]
fn raw_comes_last() {
    let last = BUILT_IN.last().expect("the table has formats");
    assert_eq!(last.id, "raw");
    assert!(matches!(last.kind, Kind::Raw));
}

#[test]
fn ids_are_unique() {
    let ids: HashSet<&str> = BUILT_IN.iter().map(|format| format.id).collect();
    assert_eq!(ids.len(), BUILT_IN.len());
}

#[test]
fn by_id_finds_every_built_in_format() {
    for format in BUILT_IN {
        let found = Format::by_id(format.id).map(|found| found.id);
        assert_eq!(found, Some(format.id));
    }
}

#[test]
fn by_id_of_a_name_not_in_the_table_is_none() {
    for id in ["log4j", "", "Logback", "logback ", "spring"] {
        assert!(Format::by_id(id).is_none(), "{id:?}");
    }
}

#[test]
fn formats_are_equal_only_to_themselves() {
    for (i, a) in BUILT_IN.iter().enumerate() {
        for (j, b) in BUILT_IN.iter().enumerate() {
            assert_eq!(a == b, i == j, "{} == {}", a.id, b.id);
        }
    }
}

#[test]
fn debug_shows_only_the_id() {
    assert_eq!(format!("{:?}", built_in("logback")), r#"Format("logback")"#);
}

#[test]
fn raw_has_no_entry_parser() {
    assert!(built_in("raw").entry_parser().is_none());
}

#[test]
fn only_logback_basic_and_wildfly_console_have_no_date() {
    let dated: Vec<(&str, bool)> = text_formats()
        .into_iter()
        .map(|format| (format.id, parser(format).has_date()))
        .collect();
    assert_eq!(
        dated,
        [
            ("logback", true),
            ("logback-basic", false),
            ("spring3", true),
            ("spring2", true),
            ("wildfly", true),
            ("wildfly-console", false),
        ]
    );
}

// --- the PRD examples ---

#[test]
fn parses_the_logback_example() {
    assert_eq!(
        columns(built_in("logback"), EXAMPLES[0].1),
        [
            "2026-10-07 12:00:00.123",
            "INFO",
            "http-nio-8080-exec-3",
            "com.example.OrderService",
            "Order 42 created",
        ]
    );
}

#[test]
fn parses_the_logback_basic_example() {
    assert_eq!(
        columns(built_in("logback-basic"), EXAMPLES[1].1),
        ["12:00:00.123", "INFO", "main", "com.example.App", "Started"]
    );
}

#[test]
fn parses_the_spring3_example() {
    assert_eq!(
        columns(built_in("spring3"), EXAMPLES[2].1),
        [
            "2026-10-07T12:00:00.123+02:00",
            "INFO",
            "main",
            "o.s.b.SpringApplication",
            "Started App in 2.1s",
        ]
    );
}

#[test]
fn parses_the_spring2_example() {
    assert_eq!(
        columns(built_in("spring2"), EXAMPLES[3].1),
        [
            "2026-10-07 12:00:00.123",
            "INFO",
            "main",
            "o.s.b.SpringApplication",
            "Started App",
        ]
    );
}

#[test]
fn parses_the_wildfly_example() {
    assert_eq!(
        columns(built_in("wildfly"), EXAMPLES[4].1),
        [
            "2026-10-07 12:00:00,123",
            "INFO",
            "Controller Boot Thread",
            "org.jboss.as.server",
            "WFLYSRV0025: WildFly Full 33.0.0.Final started",
        ]
    );
}

#[test]
fn parses_the_wildfly_console_example() {
    assert_eq!(
        columns(built_in("wildfly-console"), EXAMPLES[5].1),
        [
            "12:00:00,123",
            "INFO",
            "MSC service thread 1-2",
            "org.jboss.as",
            "WFLYSRV0049: ...",
        ]
    );
}

#[test]
fn coloured_wildfly_console_line_parses_once_stripped() {
    let coloured = b"\x1b[0m\x1b[33m12:00:00,123 WARN  [org.jboss.as.txn] (ServerService Thread Pool -- 76) WFLYTX0013: node-identifier\x1b[0m";
    let line = strip_ansi(coloured);
    let text = String::from_utf8(line.into_owned()).unwrap();
    assert_eq!(
        columns(built_in("wildfly-console"), &text),
        [
            "12:00:00,123",
            "WARN",
            "ServerService Thread Pool -- 76",
            "org.jboss.as.txn",
            "WFLYTX0013: node-identifier",
        ]
    );
}

#[test]
fn examples_start_entries_only_under_their_own_preset() {
    for (own, line) in EXAMPLES {
        for preset in text_formats().into_iter().filter(|p| p.id != own) {
            assert_eq!(
                parse(preset, line.as_bytes()),
                None,
                "{}: {line}",
                preset.id
            );
        }
    }
}

// --- timestamps ---

#[test]
fn millisecond_separator_may_be_a_dot_or_a_comma() {
    for separator in ['.', ','] {
        assert_every_preset_parses(&Parts {
            separator,
            ..Parts::default()
        });
    }
}

#[test]
fn fraction_may_have_one_to_nine_digits() {
    for fraction in ["1", "12", "123456", "123456789"] {
        assert_every_preset_parses(&Parts {
            fraction: fraction.into(),
            ..Parts::default()
        });
    }
}

#[test]
fn spring3_offset_may_be_z_or_negative() {
    for offset in ["Z", "-05:00", "+00:00", "+05:30"] {
        assert_parses(
            built_in("spring3"),
            &Parts {
                offset: offset.into(),
                ..Parts::default()
            },
        );
    }
}

#[test]
fn impossible_date_still_starts_an_entry() {
    assert_every_preset_parses(&Parts {
        date: "2026-13-45".into(),
        ..Parts::default()
    });
}

#[test]
fn impossible_time_still_starts_an_entry() {
    assert_every_preset_parses(&Parts {
        time: "25:61:99".into(),
        ..Parts::default()
    });
}

// --- levels ---

#[test]
fn every_level_name_parses() {
    for level in [
        "TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "WARNING",
    ] {
        assert_every_preset_parses(&Parts {
            level: level.into(),
            ..Parts::default()
        });
    }
}

/// The level column shows these as unknown (PRD §6.4); the line still starts an entry.
#[test]
fn unknown_level_word_still_starts_an_entry() {
    for level in ["AUDIT", "STATS", "VERB"] {
        assert_every_preset_parses(&Parts {
            level: level.into(),
            ..Parts::default()
        });
    }
}

// --- threads ---

#[test]
fn spring3_app_name_is_optional() {
    for app in [Some("myapp"), Some("order-service"), None] {
        assert_parses(
            built_in("spring3"),
            &Parts {
                app: app.map(Into::into),
                ..Parts::default()
            },
        );
    }
}

#[test]
fn spring3_without_app_name_takes_the_only_bracket_as_the_thread() {
    let line = "2026-10-07T12:00:00.123Z  INFO 1 --- [nio-8080-exec-1] com.example.App                          : x";
    assert_eq!(columns(built_in("spring3"), line)[2], "nio-8080-exec-1");
}

/// Spring Boot 3.2+ with Micrometer Tracing (`logging.pattern.correlation`).
#[test]
fn spring3_trace_and_span_ids_are_not_part_of_the_thread() {
    for app in [Some("myapp"), None] {
        assert_parses(
            built_in("spring3"),
            &Parts {
                app: app.map(Into::into),
                thread: "nio-8080-exec-1".into(),
                correlation: Some(TRACE_AND_SPAN.into()),
                ..Parts::default()
            },
        );
    }
}

#[test]
fn spring3_blank_correlation_group_is_not_part_of_the_thread() {
    for app in [Some("myapp"), None] {
        assert_parses(
            built_in("spring3"),
            &Parts {
                app: app.map(Into::into),
                correlation: Some(blank_correlation()),
                ..Parts::default()
            },
        );
    }
}

#[test]
fn spring3_line_with_trace_ids_reads_like_the_boot_docs() {
    let line = "2026-10-07T09:31:02.415+02:00  INFO 51877 --- [shop] [nio-8080-exec-2] \
                [6bfb3c0e8f1d4a2b9c7e5d3f1a2b4c6d-1a2b4c6d8e0f2a4b] \
                o.s.web.servlet.DispatcherServlet        : Completed initialization in 2 ms";
    assert_eq!(
        columns(built_in("spring3"), line),
        [
            "2026-10-07T09:31:02.415+02:00",
            "INFO",
            "nio-8080-exec-2",
            "o.s.web.servlet.DispatcherServlet",
            "Completed initialization in 2 ms",
        ]
    );
}

#[test]
fn thread_may_contain_spaces() {
    for thread in ["default task-12", "ServerService Thread Pool -- 76"] {
        assert_every_preset_parses(&Parts {
            thread: thread.into(),
            ..Parts::default()
        });
    }
}

#[test]
fn thread_may_contain_dashes_and_digits() {
    for thread in ["http-nio-8080-exec-3", "pool-1-thread-2"] {
        assert_every_preset_parses(&Parts {
            thread: thread.into(),
            ..Parts::default()
        });
    }
}

#[test]
fn thread_may_contain_parentheses() {
    assert_every_preset_parses(&Parts {
        thread: "RMI TCP Connection(2)-127.0.0.1".into(),
        ..Parts::default()
    });
}

#[test]
fn thread_may_contain_brackets() {
    assert_every_preset_parses(&Parts {
        thread: "worker [batch 7]".into(),
        ..Parts::default()
    });
}

#[test]
fn thread_may_contain_non_ascii_letters() {
    assert_every_preset_parses(&Parts {
        thread: "wątek-główny".into(),
        ..Parts::default()
    });
}

// --- classes ---

#[test]
fn class_may_name_an_inner_class() {
    assert_every_preset_parses(&Parts {
        class: "com.example.OrderService$Validator".into(),
        ..Parts::default()
    });
}

#[test]
fn class_may_name_a_lambda() {
    assert_every_preset_parses(&Parts {
        class: "com.example.OrderService$$Lambda$123/0x0000000800c4b040".into(),
        ..Parts::default()
    });
}

#[test]
fn class_may_contain_brackets() {
    // Tomcat names its loggers after the container path.
    assert_every_preset_parses(&Parts {
        class: "o.a.c.c.C.[Tomcat].[localhost].[/]".into(),
        ..Parts::default()
    });
}

#[test]
fn logback_class_may_be_padded_before_the_dash() {
    for preset in [built_in("logback"), built_in("logback-basic")] {
        assert_parses(
            preset,
            &Parts {
                pad_class: true,
                ..Parts::default()
            },
        );
    }
}

#[test]
fn spring_class_longer_than_its_column_still_parses() {
    for preset in [built_in("spring3"), built_in("spring2")] {
        assert_parses(
            preset,
            &Parts {
                class: "org.springframework.boot.web.embedded.tomcat.TomcatWebServer".into(),
                ..Parts::default()
            },
        );
    }
}

// --- messages ---

#[test]
fn logback_message_may_follow_one_or_two_dashes() {
    for preset in [built_in("logback"), built_in("logback-basic")] {
        for dashes in ["-", "--"] {
            assert_parses(
                preset,
                &Parts {
                    dashes,
                    ..Parts::default()
                },
            );
        }
    }
}

#[test]
fn message_may_contain_the_column_separators() {
    let messages = [
        "Retry 1/3 - gateway answered 503",
        "ok: 2 -- failed: 1",
        "key : value",
        "[POST /api/orders] done",
        "(user=anna) logged in",
        "a] b) c : d - e -- f",
        "--- [main] com.example.Other : nested",
        "12:00:00.123 [main] INFO  x - y",
    ];
    for message in messages {
        assert_every_preset_parses(&Parts {
            message: message.into(),
            ..Parts::default()
        });
    }
}

#[test]
fn message_may_start_with_a_dash_or_colon() {
    for message in ["- leading dash", "-- two", ": colon", ") paren"] {
        assert_every_preset_parses(&Parts {
            message: message.into(),
            ..Parts::default()
        });
    }
}

#[test]
fn message_keeps_its_own_spaces() {
    for message in ["  indented", "trailing  ", " "] {
        assert_every_preset_parses(&Parts {
            message: message.into(),
            ..Parts::default()
        });
    }
}

#[test]
fn message_may_be_empty() {
    assert_every_preset_parses(&Parts {
        message: String::new(),
        ..Parts::default()
    });
}

/// Editors and some log shippers trim trailing spaces, which leaves an empty message
/// without the space after its separator.
#[test]
fn empty_message_may_lack_the_space_after_its_separator() {
    for preset in text_formats() {
        let (line, head) = entry_line(
            preset,
            &Parts {
                message: String::new(),
                ..Parts::default()
            },
        );
        let trimmed = line.strip_suffix(' ').unwrap();
        let expected = EntryHead {
            message: trimmed.len()..trimmed.len(),
            ..head
        };
        assert_eq!(
            parse(preset, trimmed.as_bytes()),
            Some(expected),
            "{preset:?}: {trimmed:?}"
        );
    }
}

#[test]
fn message_may_contain_bytes_that_are_not_utf8() {
    for preset in text_formats() {
        let (line, head) = entry_line(
            preset,
            &Parts {
                message: String::new(),
                ..Parts::default()
            },
        );
        // Latin-2 and a stray byte.
        let mut bytes = line.into_bytes();
        bytes.extend_from_slice(b"Zam\xf3wienie \xff");
        let expected = EntryHead {
            message: head.message.start..bytes.len(),
            ..head
        };
        assert_eq!(parse(preset, &bytes), Some(expected), "{preset:?}");
    }
}

#[test]
fn message_may_contain_non_ascii_text() {
    assert_every_preset_parses(&Parts {
        message: "Zamówienie 42 utworzone — 日本".into(),
        ..Parts::default()
    });
}

// --- lines that don't start an entry ---

#[test]
fn continuation_lines_do_not_start_an_entry() {
    let lines = [
        "",
        "   ",
        "\tat com.example.OrderService.create(OrderService.java:42)",
        "    at com.example.OrderService.create(OrderService.java:42)",
        "\tat deployment.shop.war//com.example.Foo.bar(Foo.java:1) ~[classes/:na]",
        "Caused by: java.lang.IllegalStateException: boom",
        "\t... 12 more",
        "\t... 4 common frames omitted",
        "java.lang.IllegalStateException: boom",
        "com.example.payment.PaymentException: 2026-10-07 12:00:00.123 [main] INFO",
        "  Detail: Key (id)=(44) already exists.",
        "=========================================================================",
        "  JBoss Bootstrap Environment",
        " :: Spring Boot ::                (v3.3.4)",
        "{\"level\":\"INFO\",\"message\":\"json\"}",
        "INFO  com.example.App - no timestamp",
    ];
    for preset in text_formats() {
        for line in lines {
            assert_eq!(parse(preset, line.as_bytes()), None, "{preset:?}: {line:?}");
        }
    }
}

#[test]
fn entry_line_indented_by_whitespace_does_not_start_an_entry() {
    for preset in text_formats() {
        let (line, _) = entry_line(preset, &Parts::default());
        for indent in [" ", "\t"] {
            let indented = format!("{indent}{line}");
            assert_eq!(
                parse(preset, indented.as_bytes()),
                None,
                "{preset:?}: {indented:?}"
            );
        }
    }
}

#[test]
fn entry_line_cut_before_the_message_separator_does_not_start_an_entry() {
    for preset in text_formats() {
        let (line, head) = entry_line(preset, &Parts::default());
        // `- `, ` : ` or `) ` before the message: cut before its first character.
        let cut = head.message.start - 2;
        for end in 0..cut {
            let prefix = &line.as_bytes()[..end];
            assert_eq!(
                parse(preset, prefix),
                None,
                "{}: {:?}",
                preset.id,
                String::from_utf8_lossy(prefix)
            );
        }
    }
}

#[test]
fn level_that_is_not_a_word_does_not_start_an_entry() {
    let lines = [
        (
            built_in("logback"),
            "2026-10-07 12:00:00.123 [main] 42 com.example.App - x",
        ),
        (
            built_in("spring2"),
            "2026-10-07 12:00:00.123  --- 1 --- [main] com.example.App : x",
        ),
        (
            built_in("wildfly"),
            "2026-10-07 12:00:00,123 [INFO] [org.jboss.as] (main) x",
        ),
    ];
    for (preset, line) in lines {
        assert_eq!(parse(preset, line.as_bytes()), None, "{preset:?}: {line}");
    }
}

/// Deliberate trade-off: a bracketed thread ends at the first `] `, so the line can't be
/// read by letting the thread swallow columns, and a thread name that itself contains
/// `] ` is not supported.
#[test]
fn thread_never_runs_past_a_closing_bracket_and_space() {
    let lines = [
        (
            built_in("logback"),
            "2026-10-07 12:00:00.123 [main] 42 x - [y] INFO c - z",
        ),
        (
            built_in("logback"),
            "2026-10-07 12:00:00.123 [pool [a] 1] INFO c - z",
        ),
        (
            built_in("spring2"),
            "2026-10-07 12:00:00.123  INFO 12345 --- [pool [a] 1] c.e.App                                  : x",
        ),
    ];
    for (preset, line) in lines {
        assert_eq!(parse(preset, line.as_bytes()), None, "{preset:?}: {line}");
    }
}

// --- JSON lines ---

fn message_mapping() -> Mapping {
    Mapping {
        message: Some("msg"),
        ..Mapping::default()
    }
}

/// The message a JSON parser with `mapping` finds on `line`, which must start an entry.
fn json_message(mapping: Mapping, line: &str) -> &str {
    let head = EntryParser::json(mapping)
        .parse(line.as_bytes())
        .unwrap_or_else(|| panic!("{line:?} doesn't start an entry"));
    &line[head.message]
}

#[test]
fn json_message_is_the_text_of_the_mapped_key() {
    let line = r#"{"level":"info","msg":"Order \"42\" created","order":42}"#;
    assert_eq!(
        json_message(message_mapping(), line),
        r#"Order \"42\" created"#
    );
}

#[test]
fn json_message_ends_before_a_trailing_newline_escape() {
    // Docker's json-file driver keeps the newline the container printed.
    let mapping = Mapping {
        message: Some("log"),
        ..Mapping::default()
    };
    let line = r#"{"log":"Order 42 created\n","stream":"stdout"}"#;
    assert_eq!(json_message(mapping, line), "Order 42 created");
    // An escaped backslash followed by `n` is not a newline.
    let line = r#"{"log":"saved to C:\\n","stream":"stdout"}"#;
    assert_eq!(json_message(mapping, line), r"saved to C:\\n");
}

#[test]
fn json_line_without_the_mapped_message_key_has_the_whole_line_as_message() {
    let line = r#"{"level":"info","text":"Order 42 created"}"#;
    assert_eq!(json_message(message_mapping(), line), line);
}

#[test]
fn json_mapping_without_a_message_key_has_the_whole_line_as_message() {
    let line = r#"{"level":"info","msg":"Order 42 created"}"#;
    assert_eq!(json_message(Mapping::default(), line), line);
}

#[test]
fn json_object_after_spaces_or_a_byte_order_mark_starts_an_entry() {
    for line in ["  {\"msg\":\"a\"}", "\u{feff}{\"msg\":\"a\"}"] {
        assert_eq!(json_message(message_mapping(), line), "a", "{line:?}");
    }
}

#[test]
fn json_line_that_is_not_an_object_does_not_start_an_entry() {
    let parser = EntryParser::json(message_mapping());
    for line in [
        "\tat com.acme.OrderService.place(OrderService.java:88)",
        "Starting orders v1.4.2",
        "[1,2]",
        "\"msg\"",
        "",
    ] {
        assert_eq!(parser.parse(line.as_bytes()), None, "{line:?}");
    }
}

/// Only Docker's json-file driver keeps the newline a line ended with; a logfmt
/// message ending in `\n` keeps it.
#[test]
fn logfmt_message_keeps_a_trailing_newline_escape() {
    let line = r#"level=info msg="Order 42 created\n""#;
    let head = EntryParser::logfmt(message_mapping())
        .parse(line.as_bytes())
        .expect("a logfmt entry");
    assert_eq!(&line[head.message], r"Order 42 created\n");
}

// --- properties ---

fn level() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "WARNING", "SEVERE",
    ])
    .prop_map(String::from)
}

fn thread() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec![
            "main",
            "http-nio-8080-exec-3",
            "pool-1-thread-2",
            "default task-12",
            "ServerService Thread Pool -- 76",
            "RMI TCP Connection(2)-127.0.0.1",
            "worker [batch 7]",
            "wątek-główny",
        ])
        .prop_map(String::from),
        "[A-Za-z0-9._-]{1,12}( [A-Za-z0-9._-]{1,12})?",
    ]
}

fn class() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec![
            "com.example.App",
            "o.s.b.SpringApplication",
            "com.example.OrderService$Validator",
            "com.example.OrderService$$Lambda$123/0x0000000800c4b040",
            "o.a.c.c.C.[Tomcat].[localhost].[/]",
            "stdout",
        ])
        .prop_map(String::from),
        "[a-z]{1,8}(\\.[a-z]{1,8}){0,3}\\.[A-Z][A-Za-z0-9]{0,15}(\\$[A-Za-z0-9]{1,8})?",
    ]
}

prop_compose! {
    fn timestamp_parts()(
        date in "[0-9]{4}-[0-9]{2}-[0-9]{2}",
        time in "[0-9]{2}:[0-9]{2}:[0-9]{2}",
        separator in prop::sample::select(vec!['.', ',']),
        fraction in "[0-9]{1,9}",
        offset in prop_oneof![Just("Z".to_string()), "[+-][0-9]{2}:[0-9]{2}"],
    ) -> Parts {
        Parts { date, time, separator, fraction, offset, ..Parts::default() }
    }
}

prop_compose! {
    fn parts()(
        timestamp in timestamp_parts(),
        level in level(),
        pid in "[0-9]{1,7}",
        app in prop::option::of("[a-z][a-z0-9-]{0,15}"),
        thread in thread(),
        class in class(),
        pad_class in any::<bool>(),
        dashes in prop::sample::select(vec!["-", "--"]),
        message in "[^\n]{0,40}",
    ) -> Parts {
        Parts { level, pid, app, thread, class, pad_class, dashes, message, ..timestamp }
    }
}

fn correlation() -> impl Strategy<Value = String> {
    prop_oneof!["[0-9a-f]{32}-[0-9a-f]{16}", Just(blank_correlation()),]
}

fn structured() -> impl Strategy<Value = &'static Format> {
    prop::sample::select(text_formats())
}

/// Bytes that log lines are made of, so random input gets far into a pattern.
fn log_soup() -> impl Strategy<Value = Vec<u8>> {
    let tokens: Vec<&[u8]> = vec![
        b"2026-10-07",
        b"12:00:00",
        b".123",
        b",123",
        b"T",
        b"+02:00",
        b" ",
        b"  ",
        b"INFO",
        b"[",
        b"]",
        b"(",
        b")",
        b"-",
        b"--- ",
        b":",
        b"main",
        b"com.example.App",
        b"12345",
    ];
    let token = prop_oneof![
        4 => prop::sample::select(tokens).prop_map(<[u8]>::to_vec),
        1 => prop::collection::vec(any::<u8>(), 1..4),
    ];
    prop::collection::vec(token, 0..24).prop_map(|tokens| tokens.concat())
}

proptest! {
    #[test]
    fn parse_never_panics_on_arbitrary_bytes(
        preset in structured(),
        bytes in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        parser(preset).parse(&bytes);
    }

    #[test]
    fn parse_never_panics_on_log_like_bytes(preset in structured(), bytes in log_soup()) {
        parser(preset).parse(&bytes);
    }

    #[test]
    fn json_parse_never_panics_and_keeps_the_message_in_the_line(
        rest in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let bytes = [b"{".as_slice(), &rest].concat();
        if let Some(head) = EntryParser::json(message_mapping()).parse(&bytes) {
            prop_assert!(head.message.start <= head.message.end && head.message.end <= bytes.len());
        }
    }

    #[test]
    fn entry_lines_parse_with_exact_spans(preset in structured(), parts in parts()) {
        let (line, head) = entry_line(preset, &parts);
        prop_assert_eq!(parse(preset, line.as_bytes()), Some(head), "{:?}", line);
    }

    /// Detection (#9) relies on this: a file in one preset scores nothing in another.
    #[test]
    fn entry_lines_start_entries_only_under_their_own_preset(
        own in structured(),
        parts in parts(),
    ) {
        let (line, _) = entry_line(own, &parts);
        for other in text_formats().into_iter().filter(|&other| other != own) {
            prop_assert_eq!(parse(other, line.as_bytes()), None, "{}: {:?}", other.id, line);
        }
    }

    #[test]
    fn spring3_lines_with_a_correlation_group_parse_with_exact_spans(
        parts in parts(),
        correlation in correlation(),
    ) {
        let parts = Parts { correlation: Some(correlation), ..parts };
        let (line, head) = entry_line(built_in("spring3"), &parts);
        prop_assert_eq!(parse(built_in("spring3"), line.as_bytes()), Some(head), "{:?}", line);
    }

    #[test]
    fn spring3_lines_with_a_correlation_group_start_entries_only_as_spring3(
        parts in parts(),
        correlation in correlation(),
    ) {
        let parts = Parts { correlation: Some(correlation), ..parts };
        let (line, _) = entry_line(built_in("spring3"), &parts);
        for other in text_formats().into_iter().filter(|&other| other != built_in("spring3")) {
            prop_assert_eq!(parse(other, line.as_bytes()), None, "{}: {:?}", other.id, line);
        }
    }

    #[test]
    fn any_match_has_disjoint_columns_and_the_message_last(
        preset in structured(),
        parts in parts(),
        at in any::<prop::sample::Index>(),
        byte in prop_oneof![Just(b' '), Just(b']'), Just(b')'), Just(b'-'), Just(b':'), any::<u8>()],
    ) {
        let (line, _) = entry_line(preset, &parts);
        let mut bytes = line.into_bytes();
        let i = at.index(bytes.len());
        bytes[i] = byte;
        if let Some(head) = parse(preset, &bytes) {
            let columns = [head.timestamp, head.level, head.thread, head.logger];
            let mut spans: Vec<_> = columns.into_iter().flatten().chain([head.message.clone()]).collect();
            spans.sort_by_key(|span| span.start);
            prop_assert!(spans.windows(2).all(|pair| pair[0].end <= pair[1].start), "{:?}", spans);
            prop_assert_eq!(spans.last(), Some(&head.message));
            prop_assert_eq!(head.message.end, bytes.len());
        }
    }
}
