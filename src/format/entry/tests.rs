use std::ops::Range;

use proptest::prelude::*;

use super::*;
use crate::format::{BUILT_IN, strip_ansi};
use crate::test_support::built_in;

// Logback lines, used wherever the preset doesn't matter.
const ONE: &str = "2026-10-07 12:00:00.001 [main] INFO  com.example.App - one";
const TWO: &str = "2026-10-07 12:00:00.002 [main] ERROR com.example.App - two";
const THREE: &str = "2026-10-07 12:00:00.003 [main] WARN  com.example.App - three";

const FRAME: &str = "\tat com.example.App.run(App.java:12)";

/// Feeds `lines` to a grouper for `preset` and returns every entry, the open one last.
fn group<L: AsRef<[u8]>>(preset: &'static Format, lines: &[L]) -> Vec<Entry> {
    let mut grouper = EntryGrouper::new(preset);
    let mut entries: Vec<Entry> = lines
        .iter()
        .filter_map(|line| grouper.push(line.as_ref()))
        .collect();
    entries.extend(grouper.open_entry().cloned());
    entries
}

fn logback(lines: &[&str]) -> Vec<Entry> {
    group(built_in("logback"), lines)
}

fn spans(entries: &[Entry]) -> Vec<Range<usize>> {
    entries.iter().map(|entry| entry.lines.clone()).collect()
}

/// The one entry in `entries`.
fn only(entries: &[Entry]) -> &Entry {
    match entries {
        [entry] => entry,
        _ => panic!("expected one entry: {entries:#?}"),
    }
}

fn exceptions(entries: &[Entry]) -> Vec<bool> {
    entries.iter().map(|entry| entry.has_exception).collect()
}

fn headless(entries: &[Entry]) -> Vec<bool> {
    entries.iter().map(|entry| entry.head.is_none()).collect()
}

fn parse(preset: &'static Format, line: &[u8]) -> Option<EntryHead> {
    preset
        .entry_parser()
        .unwrap_or_else(|| panic!("{preset:?} has no entry parser"))
        .parse(line)
}

fn is(line: &str) -> bool {
    is_exception_line(line.as_bytes())
}

// --- grouping ---

#[test]
fn nothing_is_open_before_the_first_line() {
    assert_eq!(EntryGrouper::new(built_in("logback")).open_entry(), None);
}

#[test]
fn every_entry_start_begins_a_new_entry() {
    assert_eq!(spans(&logback(&[ONE, TWO, THREE])), [0..1, 1..2, 2..3]);
}

#[test]
fn lines_that_do_not_start_an_entry_belong_to_the_previous_one() {
    let entries = logback(&[ONE, "  wrapped message", "more of it", TWO]);
    assert_eq!(spans(&entries), [0..3, 3..4]);
}

#[test]
fn entry_start_returns_the_entry_it_closes() {
    let mut grouper = EntryGrouper::new(built_in("logback"));
    grouper.push(ONE.as_bytes());
    grouper.push(FRAME.as_bytes());
    let closed = grouper
        .push(TWO.as_bytes())
        .expect("TWO closes ONE's entry");
    assert_eq!(closed.lines, 0..2);
    assert_eq!(closed.head, parse(built_in("logback"), ONE.as_bytes()));
    assert!(closed.has_exception);
}

#[test]
fn last_entry_stays_open() {
    let mut grouper = EntryGrouper::new(built_in("logback"));
    let closed: Vec<Entry> = [ONE, TWO, FRAME]
        .iter()
        .filter_map(|line| grouper.push(line.as_bytes()))
        .collect();
    assert_eq!(only(&closed).lines, 0..1);
    let open = grouper.open_entry().expect("TWO's entry is open");
    assert_eq!(open.lines, 1..3);
    assert!(open.has_exception);
}

#[test]
fn head_holds_the_columns_of_the_first_line() {
    let entries = logback(&[ONE, FRAME, TWO]);
    assert_eq!(entries[0].head, parse(built_in("logback"), ONE.as_bytes()));
    assert_eq!(entries[1].head, parse(built_in("logback"), TWO.as_bytes()));
}

#[test]
fn colour_codes_around_an_entry_start_are_stripped_before_parsing() {
    let line =
        "\x1b[0m\x1b[31m12:00:01,233 ERROR [org.jboss.msc] (MSC service thread 1-1) failed\x1b[0m";
    let plain = "12:00:01,233 ERROR [org.jboss.msc] (MSC service thread 1-1) failed";
    let entries = group(built_in("wildfly-console"), &["banner", line]);
    assert_eq!(spans(&entries), [0..1, 1..2]);
    assert_eq!(
        entries[1].head,
        parse(built_in("wildfly-console"), plain.as_bytes())
    );
}

#[test]
fn blank_lines_inside_a_stack_trace_stay_in_the_entry() {
    let entries = logback(&[
        ONE,
        "",
        "java.lang.IllegalStateException: Order 43 is already closed",
        FRAME,
        "",
        "\tat com.example.App.main(App.java:5)",
        "",
        TWO,
    ]);
    assert_eq!(spans(&entries), [0..7, 7..8]);
}

#[test]
fn entry_with_ten_thousand_continuation_lines_is_one_entry() {
    let detail: Vec<String> = (1..10_000).map(|i| format!("  detail {i}")).collect();
    let mut lines = vec![ONE];
    lines.extend(detail.iter().map(String::as_str));
    lines.push("\t... 3 more");
    lines.push(TWO);
    let entries = logback(&lines);
    assert_eq!(spans(&entries), [0..10_001, 10_001..10_002]);
    assert_eq!(exceptions(&entries), [true, false]);
}

// --- headless leading entry ---

#[test]
fn lines_before_the_first_entry_start_form_a_headless_entry() {
    let entries = logback(&["", "  :: Spring Boot ::  (v3.3.4)", "", ONE, TWO]);
    assert_eq!(spans(&entries), [0..3, 3..4, 4..5]);
    assert_eq!(headless(&entries), [true, false, false]);
}

#[test]
fn no_headless_entry_when_the_first_line_starts_an_entry() {
    let entries = logback(&[ONE, "  wrapped"]);
    assert_eq!(only(&entries).lines, 0..2);
    assert_eq!(headless(&entries), [false]);
}

#[test]
fn single_blank_line_before_the_first_entry_is_a_headless_entry() {
    let entries = logback(&["", ONE]);
    assert_eq!(spans(&entries), [0..1, 1..2]);
    assert_eq!(headless(&entries), [true, false]);
}

#[test]
fn without_any_entry_start_all_lines_are_one_headless_entry() {
    let entries = logback(&["JAVA_OPTS: -Xmx512m", "", "starting"]);
    assert_eq!(only(&entries).lines, 0..3);
    assert_eq!(headless(&entries), [true]);
    assert_eq!(exceptions(&entries), [false]);
}

#[test]
fn file_that_starts_with_a_stack_trace_is_a_headless_entry_with_an_exception() {
    let entries = logback(&[
        "\tat com.example.App.run(App.java:12)",
        "\tat java.base/java.lang.Thread.run(Thread.java:1583)",
        "Caused by: java.io.IOException: disk full",
        "\t... 7 more",
        ONE,
    ]);
    assert_eq!(spans(&entries), [0..4, 4..5]);
    assert_eq!(headless(&entries), [true, false]);
    assert_eq!(exceptions(&entries), [true, false]);
}

/// A headless entry is the tail of an entry that isn't there, so its first line is a
/// continuation line too.
#[test]
fn first_line_of_a_headless_entry_is_checked() {
    let entries = logback(&["java.lang.IllegalStateException: boom", ONE]);
    assert_eq!(exceptions(&entries), [true, false]);
}

// --- exception flags ---

// Which lines count is up to `is_exception_line`, tested below; here only that a
// continuation line is checked at all.

#[test]
fn tab_indented_frame_marks_an_exception() {
    assert_eq!(exceptions(&logback(&[ONE, FRAME])), [true]);
}

#[test]
fn plain_continuation_lines_are_not_an_exception() {
    let entries = logback(&[
        ONE,
        "  Detail: Key (id)=(44) already exists.",
        "",
        "second line of the message",
    ]);
    assert_eq!(exceptions(&entries), [false]);
}

#[test]
fn single_line_entry_has_no_exception() {
    assert_eq!(exceptions(&logback(&[ONE, TWO])), [false, false]);
}

#[test]
fn stack_frame_after_the_timestamp_is_not_an_exception() {
    // WildFly logs every line a deployment prints to stderr as an entry of its own.
    let line = "2026-10-07 09:18:09,904 ERROR [stderr] (default task-9) \tat com.example.ReportDao.load(ReportDao.java:29)";
    let entries = group(built_in("wildfly"), &[line]);
    assert_eq!(headless(&entries), [false]);
    assert_eq!(exceptions(&entries), [false]);
}

#[test]
fn exception_header_after_the_timestamp_is_not_an_exception() {
    let line = "2026-10-07 12:00:00.001 [main] ERROR com.example.App - java.lang.IllegalStateException: boom";
    assert_eq!(exceptions(&logback(&[line])), [false]);
}

#[test]
fn exception_belongs_only_to_its_own_entry() {
    assert_eq!(
        exceptions(&logback(&[ONE, FRAME, TWO, "  wrapped", THREE])),
        [true, false, false]
    );
    assert_eq!(exceptions(&logback(&[ONE, TWO, FRAME])), [false, true]);
}

#[test]
fn exception_stays_flagged_after_later_plain_lines() {
    let entries = logback(&[ONE, FRAME, "", "  wrapped"]);
    assert_eq!(exceptions(&entries), [true]);
}

#[test]
fn continuation_lines_are_checked_after_stripping_colour_codes() {
    let frame = "\x1b[31m\tat com.example.App.run(App.java:12)\x1b[0m";
    assert_eq!(exceptions(&logback(&[ONE, frame])), [true]);
}

// --- Raw ---

#[test]
fn raw_closes_the_previous_line_with_every_line() {
    let mut grouper = EntryGrouper::new(built_in("raw"));
    assert_eq!(grouper.push(b"first"), None);
    let closed = grouper
        .push(FRAME.as_bytes())
        .expect("each line closes the last");
    assert_eq!(closed.lines, 0..1);
    assert_eq!(
        grouper.open_entry().map(|entry| entry.lines.clone()),
        Some(1..2)
    );
}

// --- is_exception_line: what matches ---

#[test]
fn tab_indented_frame_is_an_exception_line() {
    assert!(is("\tat java.base/java.lang.Thread.run(Thread.java:1583)"));
}

#[test]
fn space_indented_frame_is_an_exception_line() {
    assert!(is("    at com.example.App.run(App.java:12)"));
}

#[test]
fn doubly_indented_frame_is_an_exception_line() {
    assert!(is("\t\tat com.example.App.close(App.java:40)"));
}

#[test]
fn caused_by_is_an_exception_line() {
    assert!(is(
        "Caused by: java.net.SocketTimeoutException: Read timed out"
    ));
}

#[test]
fn more_frames_is_an_exception_line() {
    assert!(is("\t... 5 more"));
}

#[test]
fn common_frames_omitted_is_an_exception_line() {
    assert!(is("\t... 4 common frames omitted"));
    assert!(is("    ... 12 common frames omitted"));
}

#[test]
fn exception_header_with_a_message_is_an_exception_line() {
    assert!(is(
        "java.lang.IllegalStateException: Order 43 is already closed"
    ));
}

#[test]
fn exception_header_without_a_message_is_an_exception_line() {
    assert!(is("java.lang.IllegalStateException"));
}

#[test]
fn error_header_is_an_exception_line() {
    assert!(is(
        "java.lang.NoClassDefFoundError: javax/servlet/http/HttpServlet"
    ));
    assert!(is("java.lang.OutOfMemoryError"));
}

#[test]
fn nested_class_header_is_an_exception_line() {
    assert!(is("com.example.Outer$ValidationException: sku-9 unknown"));
}

#[test]
fn unqualified_header_is_an_exception_line() {
    assert!(is("PSQLException: duplicate key"));
}

#[test]
fn python_traceback_header_is_an_exception_line() {
    assert!(is("Traceback (most recent call last):"));
}

#[test]
fn python_file_line_is_an_exception_line() {
    assert!(is(r#"  File "/app/shop/orders.py", line 12, in load"#));
}

#[test]
fn python_exception_during_handling_is_an_exception_line() {
    assert!(is(
        "During handling of the above exception, another exception occurred:"
    ));
}

#[test]
fn python_direct_cause_is_an_exception_line() {
    assert!(is(
        "The above exception was the direct cause of the following exception:"
    ));
}

#[test]
fn go_panic_is_an_exception_line() {
    assert!(is(
        "panic: runtime error: index out of range [5] with length 3"
    ));
}

#[test]
fn go_goroutine_header_is_an_exception_line() {
    assert!(is("goroutine 1 [running]:"));
    assert!(is("goroutine 7 [chan receive, 2 minutes]:"));
}

#[test]
fn go_frame_is_an_exception_line() {
    assert!(is("\t/usr/local/go/src/runtime/panic.go:770 +0x132"));
}

#[test]
fn go_frame_without_an_offset_is_an_exception_line() {
    assert!(is("\t/app/cmd/shop/main.go:12"));
}

/// The unindented half of a Go frame: the function, with the argument words Go
/// prints in hex, `...` when it leaves them out, and `?` after a value it isn't sure of.
#[test]
fn go_function_line_is_an_exception_line() {
    assert!(is("main.main()"));
    assert!(is("main.lookup(...)"));
    assert!(is("main.worker(0xc000012345)"));
    assert!(is(
        "github.com/sirupsen/logrus.(*Entry).log(0xc0001b2000, 0x0, {0xc00001c0a0, 0x1b})"
    ));
    assert!(is("main.Map[...]({0xc000010000, 0x3, 0x3}, 0x4b2f10)"));
    assert!(is("panic({0x4a3b20?, 0xc000012345?})"));
    assert!(is("main.(*List[...]).Push(0xc000010000, 0x1)"));
    assert!(is("main.(*order_store).Get(0xc000010000)"));
    assert!(is("main.größe(0x1)"));
}

#[test]
fn go_created_by_line_is_an_exception_line() {
    assert!(is("created by main.main in goroutine 1"));
    assert!(is("created by net/http.(*Server).Serve"));
}

#[test]
fn go_signal_line_is_an_exception_line() {
    assert!(is(
        "[signal SIGSEGV: segmentation violation code=0x1 addr=0x0 pc=0x47a8b2]"
    ));
}

#[test]
fn go_elided_frames_line_is_an_exception_line() {
    assert!(is("...additional frames elided..."));
}

#[test]
fn dotnet_inner_exception_is_an_exception_line() {
    assert!(is(" ---> System.IO.IOException: The disk is full"));
}

#[test]
fn dotnet_end_of_inner_exception_is_an_exception_line() {
    assert!(is("   --- End of inner exception stack trace ---"));
}

#[test]
fn dotnet_end_of_stack_trace_from_previous_location_is_an_exception_line() {
    assert!(is("--- End of stack trace from previous location ---"));
}

// --- is_exception_line: what doesn't ---

#[test]
fn frame_without_indentation_is_not_an_exception_line() {
    assert!(!is("at com.example.App.run(App.java:12)"));
}

#[test]
fn indented_word_starting_with_at_is_not_a_frame() {
    assert!(!is("\tattempt 3 of 5 failed"));
}

#[test]
fn indented_caused_by_is_not_an_exception_line() {
    assert!(!is(
        "\tCaused by: java.lang.RuntimeException: WFLYSRV0177: Error getting reflective information"
    ));
}

#[test]
fn omitted_frames_without_indentation_are_not_an_exception_line() {
    assert!(!is("... 5 more"));
}

#[test]
fn omitted_frames_need_a_count() {
    assert!(!is("\t... more"));
}

#[test]
fn omitted_frames_need_more_or_common_frames_omitted() {
    assert!(!is("\t... 5 lines skipped"));
}

#[test]
fn header_needs_a_space_after_the_colon() {
    assert!(!is("java.lang.IllegalStateException:boom"));
}

#[test]
fn class_name_that_only_contains_exception_is_not_a_header() {
    assert!(!is("ExceptionHandler registered for /api"));
    assert!(!is("java.lang.IllegalStateExceptionally: x"));
}

#[test]
fn exception_named_mid_line_is_not_a_header() {
    assert!(!is("Request failed: java.lang.IllegalStateException: boom"));
}

#[test]
fn message_about_a_missing_file_is_not_a_python_file_line() {
    assert!(!is("File not found"));
    assert!(!is("  File not found: /data/inbox/orders.csv"));
}

#[test]
fn indented_path_that_is_not_a_go_file_is_not_a_go_frame() {
    assert!(!is("\t/data/inbox/orders.csv:12"));
}

#[test]
fn word_goroutine_without_a_number_and_state_is_not_a_goroutine_header() {
    assert!(!is("goroutine leak detected in worker pool"));
    assert!(!is("goroutine 12 finished"));
}

/// Go writes every argument of a function in hex, so a call with other arguments, a
/// Java frame without its `at` among them, isn't a Go function line.
#[test]
fn call_with_other_arguments_is_not_a_go_function_line() {
    assert!(!is("order.close(42)"));
    assert!(!is("com.example.App.run(App.java:12)"));
    assert!(!is("Started App in 2.1s (process running for 2.4)"));
}

#[test]
fn created_by_in_a_sentence_is_not_a_go_created_by_line() {
    assert!(!is("created by admin in the back office"));
}

/// After `created by` comes a function and, at most, ` in goroutine N`.
#[test]
fn created_by_with_something_else_is_not_a_go_created_by_line() {
    assert!(!is("created by everyone"));
    assert!(!is("created by main.main extra"));
    assert!(!is("created by main.main in goroutine "));
    assert!(!is("created by main.main in goroutine one"));
}

#[test]
fn argument_without_hex_digits_after_0x_is_not_a_go_argument() {
    assert!(!is("main.run(0x)"));
    assert!(!is("main.run(0xg1)"));
}

#[test]
fn receiver_that_is_not_an_identifier_is_not_a_go_receiver() {
    assert!(!is("main.(a b).run()"));
    assert!(!is("main.(*T#).run()"));
}

#[test]
fn bracketed_text_that_is_not_a_signal_is_not_an_exception_line() {
    assert!(!is("[signal handler installed]"));
}

/// A bare `foo.Bar()` or `foo.Bar(...)` on a line of its own is an accepted false
/// positive: a per-line rule can't tell it from Go's `main.main()`.
#[test]
fn text_that_only_resembles_a_go_function_line_is_not_one() {
    assert!(!is("see util.Parse(...)"));
    assert!(!is("Done (0x1)"));
    assert!(!is("main.run(0x1) extra"));
    assert!(!is("a,b.c()"));
    assert!(!is("com.Foo$Bar.run()"));
}

/// Like the Java lines, the lines of the other ecosystems count only at the start.
#[test]
fn ecosystem_markers_mid_line_are_not_exception_lines() {
    assert!(!is("worker said: Traceback (most recent call last):"));
    assert!(!is("recovered from panic: nil map"));
    assert!(!is("stock 12 ---> 11"));
    assert!(!is("note --- End of inner exception stack trace ---"));
}

#[test]
fn blank_lines_are_not_exception_lines() {
    assert!(!is(""));
    assert!(!is("    "));
    assert!(!is("\t"));
}

#[test]
fn ordinary_continuation_lines_are_not_exception_lines() {
    assert!(!is("  Detail: Key (id)=(44) already exists."));
    assert!(!is(
        "WFLYCTL0186:   Services which failed to start:      service jboss.deployment.unit"
    ));
    assert!(!is("second line of the message"));
}

// --- properties ---

/// An entry start for every preset but `Raw`.
const STARTS: [&str; 6] = [
    ONE,
    "12:00:00.123 [main] INFO  com.example.App -- Started",
    "2026-10-07T12:00:00.123+02:00  INFO 12345 --- [myapp] [           main] o.s.b.SpringApplication                  : Started App in 2.1s",
    "2026-10-07 12:00:00.123  INFO 12345 --- [           main] o.s.b.SpringApplication                  : Started App",
    "2026-10-07 12:00:00,123 INFO  [org.jboss.as.server] (Controller Boot Thread) WFLYSRV0025: started",
    "12:00:00,123 INFO  [org.jboss.as] (MSC service thread 1-2) WFLYSRV0049: starting",
];

const CONTINUATIONS: [&str; 12] = [
    FRAME,
    "    at com.example.App.main(App.java:5)",
    "Caused by: java.io.IOException: disk full",
    "\t... 3 more",
    "\t... 12 common frames omitted",
    "java.lang.IllegalStateException: boom",
    "java.lang.OutOfMemoryError",
    "\tCaused by: java.lang.RuntimeException: x",
    "  Detail: Key (id)=(44) already exists.",
    "",
    "plain text",
    "\tattempt 2",
];

/// Lines a log might have: entry starts of every preset, stack-trace lines, other
/// continuation lines, any of those in colour, and arbitrary bytes.
fn log_line() -> impl Strategy<Value = Vec<u8>> {
    let known = || {
        prop::sample::select([STARTS.as_slice(), CONTINUATIONS.as_slice()].concat())
            .prop_map(|line| line.as_bytes().to_vec())
    };
    prop_oneof![
        6 => known(),
        2 => known().prop_map(|line| [b"\x1b[0m\x1b[31m".as_slice(), &line, b"\x1b[0m"].concat()),
        1 => prop::collection::vec(any::<u8>(), 0..48),
    ]
}

fn log_lines() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(log_line(), 0..40)
}

fn any_preset() -> impl Strategy<Value = &'static Format> {
    prop::sample::select(BUILT_IN.iter().collect::<Vec<_>>())
}

/// Whether `line` starts an entry under `preset`, as the preset's parser sees it.
fn starts_entry(preset: &'static Format, line: &[u8]) -> bool {
    preset
        .entry_parser()
        .is_none_or(|parser| parser.parse(&strip_ansi(line)).is_some())
}

proptest! {
    #[test]
    fn concatenated_entries_give_back_every_line_in_order(
        preset in any_preset(),
        lines in log_lines(),
    ) {
        let entries = group(preset, &lines);
        prop_assert!(entries.iter().all(|entry| !entry.lines.is_empty()), "{:?}", entries);
        let regrouped: Vec<&Vec<u8>> = entries
            .iter()
            .flat_map(|entry| &lines[entry.lines.clone()])
            .collect();
        prop_assert_eq!(regrouped, lines.iter().collect::<Vec<_>>());
    }

    #[test]
    fn entries_begin_at_the_first_line_and_at_every_entry_start(
        preset in any_preset(),
        lines in log_lines(),
    ) {
        let entries = group(preset, &lines);
        let firsts: Vec<usize> = entries.iter().map(|entry| entry.lines.start).collect();
        let expected: Vec<usize> = (0..lines.len())
            .filter(|&i| i == 0 || starts_entry(preset, &lines[i]))
            .collect();
        prop_assert_eq!(firsts, expected);
    }

    #[test]
    fn head_is_the_first_line_parsed_after_stripping(
        preset in any_preset(),
        lines in log_lines(),
    ) {
        for entry in group(preset, &lines) {
            let first = strip_ansi(&lines[entry.lines.start]);
            let expected = preset.entry_parser().and_then(|parser| parser.parse(&first));
            prop_assert_eq!(&entry.head, &expected, "{:?}", entry);
        }
    }

    #[test]
    fn entry_has_an_exception_iff_a_checked_line_is_an_exception_line(
        preset in any_preset(),
        lines in log_lines(),
    ) {
        for entry in group(preset, &lines) {
            // Under Raw every entry lacks a head, but none is headless.
            let headless = entry.head.is_none() && preset != built_in("raw");
            let checked = if headless {
                entry.lines.clone()
            } else {
                entry.lines.start + 1..entry.lines.end
            };
            let expected = lines[checked]
                .iter()
                .any(|line| is_exception_line(&strip_ansi(line)));
            prop_assert_eq!(entry.has_exception, expected, "{:?}", entry);
        }
    }

    #[test]
    fn push_returns_exactly_the_entry_that_was_open(
        preset in any_preset(),
        lines in log_lines(),
    ) {
        let mut grouper = EntryGrouper::new(preset);
        for (i, line) in lines.iter().enumerate() {
            let before = grouper.open_entry().cloned();
            match grouper.push(line) {
                Some(closed) => {
                    prop_assert_eq!(Some(closed), before);
                    let open = grouper.open_entry().expect("a new entry is open");
                    prop_assert_eq!(open.lines.clone(), i..i + 1);
                }
                None => {
                    let open = grouper.open_entry().expect("an entry is open");
                    prop_assert_eq!(open.lines.end, i + 1);
                    if let Some(before) = before {
                        prop_assert_eq!(open.lines.start, before.lines.start);
                        prop_assert_eq!(&open.head, &before.head);
                        prop_assert!(open.has_exception || !before.has_exception);
                    }
                }
            }
        }
    }

    #[test]
    fn raw_makes_one_entry_per_line_without_exceptions(lines in log_lines()) {
        let entries = group(built_in("raw"), &lines);
        prop_assert_eq!(entries.len(), lines.len());
        for (i, entry) in entries.iter().enumerate() {
            prop_assert_eq!(entry.lines.clone(), i..i + 1);
            prop_assert_eq!(&entry.head, &None);
            prop_assert!(!entry.has_exception);
        }
    }

    #[test]
    fn grouping_never_panics_on_arbitrary_bytes(
        preset in any_preset(),
        lines in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..64), 0..16),
    ) {
        group(preset, &lines);
    }

    #[test]
    fn is_exception_line_never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..128)
    ) {
        is_exception_line(&bytes);
    }
}
