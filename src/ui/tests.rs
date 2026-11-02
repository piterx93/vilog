use std::ffi::{OsStr, OsString};
use std::fs;
use std::ops::RangeInclusive;
use std::os::unix::ffi::OsStringExt;
use std::time::{Duration, Instant};

use insta::assert_snapshot;
use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Cell;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use super::*;
use crate::app::{Action, FileChange};
use crate::config::Config;
use crate::filter::{Base, Classes, Filters, Levels, TimeFilter, TimeSpec};
use crate::format::{BUILT_IN, Format};
use crate::matcher::{self, Matcher};
use crate::mouse::HitMap;
use crate::test_support::built_in;
use crate::test_support::{
    self, EXAMPLES, LARGE_INDEX_DEADLINE, action, app, app_in, char_key, ctrl, detected_app,
    following_app, hide_line_numbers, key, named_app, no_logger, no_thread_or_logger, no_timestamp,
    numbered_log, row, spike_log, unindexed_app, wait_for_filter, wait_for_index,
    wait_for_index_within, wait_for_search,
};

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App, theme: Theme) {
    terminal.draw(|frame| render(frame, app, theme)).unwrap();
}

/// `app` drawn at `width`×`height` without line numbers; the gutter's tests show them.
fn drawn(app: &mut App, width: u16, height: u16, theme: Theme) -> Terminal<TestBackend> {
    hide_line_numbers(app);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    draw(&mut terminal, app, theme);
    terminal
}

/// The 80×24 terminal after pressing `keys`, drawn after each.
fn terminal_after(app: &mut App, keys: &[KeyEvent]) -> Terminal<TestBackend> {
    let mut terminal = drawn(app, 80, 24, Theme::Styled);
    for &key in keys {
        app.apply(action(key).unwrap());
        draw(&mut terminal, app, Theme::Styled);
    }
    terminal
}

/// The 80×24 screen after pressing `keys`, as text.
fn screen_after(app: &mut App, keys: &[KeyEvent]) -> String {
    terminal_after(app, keys).backend().to_string()
}

/// Screen lines `lines` of the 80×24 screen after pressing `keys`, as [`row`] shows
/// them, one per line.
fn lines_after(app: &mut App, keys: &[KeyEvent], lines: RangeInclusive<u16>) -> String {
    let terminal = terminal_after(app, keys);
    let rows: Vec<String> = lines.map(|y| row(&terminal, y)).collect();
    rows.join("\n")
}

/// How `numbered_log` shows line `n` (1-based) on screen, `▶` when it is selected.
fn shown(n: usize, selected: bool) -> String {
    let gutter = if selected { '▶' } else { ' ' };
    format!("{gutter}12:00:00.{n:03} [main] INFO  com.example.App - step {n}")
}

fn hundred_lines() -> App {
    app(numbered_log(100).as_bytes())
}

// --- whole screens ---

#[test]
fn first_screen_shows_the_top_of_the_file() {
    assert_snapshot!(screen_after(&mut hundred_lines(), &[]), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [main] INFO  com.example.App - step 1                             "
    " 12:00:00.002 [main] INFO  com.example.App - step 2                             "
    " 12:00:00.003 [main] INFO  com.example.App - step 3                             "
    " 12:00:00.004 [main] INFO  com.example.App - step 4                             "
    " 12:00:00.005 [main] INFO  com.example.App - step 5                             "
    " 12:00:00.006 [main] INFO  com.example.App - step 6                             "
    " 12:00:00.007 [main] INFO  com.example.App - step 7                             "
    " 12:00:00.008 [main] INFO  com.example.App - step 8                             "
    " 12:00:00.009 [main] INFO  com.example.App - step 9                             "
    " 12:00:00.010 [main] INFO  com.example.App - step 10                            "
    " 12:00:00.011 [main] INFO  com.example.App - step 11                            "
    " 12:00:00.012 [main] INFO  com.example.App - step 12                            "
    " 12:00:00.013 [main] INFO  com.example.App - step 13                            "
    " 12:00:00.014 [main] INFO  com.example.App - step 14                            "
    " 12:00:00.015 [main] INFO  com.example.App - step 15                            "
    " 12:00:00.016 [main] INFO  com.example.App - step 16                            "
    " 12:00:00.017 [main] INFO  com.example.App - step 17                            "
    " 12:00:00.018 [main] INFO  com.example.App - step 18                            "
    " 12:00:00.019 [main] INFO  com.example.App - step 19                            "
    " 12:00:00.020 [main] INFO  com.example.App - step 20                            "
    " 12:00:00.021 [main] INFO  com.example.App - step 21                            "
    " 12:00:00.022 [main] INFO  com.example.App - step 22                            "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn capital_g_shows_the_last_screen() {
    let out = screen_after(&mut hundred_lines(), &[char_key('G')]);
    assert_snapshot!(out, @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ FOLLOW ●                                       "
    " 12:00:00.079 [main] INFO  com.example.App - step 79                            "
    " 12:00:00.080 [main] INFO  com.example.App - step 80                            "
    " 12:00:00.081 [main] INFO  com.example.App - step 81                            "
    " 12:00:00.082 [main] INFO  com.example.App - step 82                            "
    " 12:00:00.083 [main] INFO  com.example.App - step 83                            "
    " 12:00:00.084 [main] INFO  com.example.App - step 84                            "
    " 12:00:00.085 [main] INFO  com.example.App - step 85                            "
    " 12:00:00.086 [main] INFO  com.example.App - step 86                            "
    " 12:00:00.087 [main] INFO  com.example.App - step 87                            "
    " 12:00:00.088 [main] INFO  com.example.App - step 88                            "
    " 12:00:00.089 [main] INFO  com.example.App - step 89                            "
    " 12:00:00.090 [main] INFO  com.example.App - step 90                            "
    " 12:00:00.091 [main] INFO  com.example.App - step 91                            "
    " 12:00:00.092 [main] INFO  com.example.App - step 92                            "
    " 12:00:00.093 [main] INFO  com.example.App - step 93                            "
    " 12:00:00.094 [main] INFO  com.example.App - step 94                            "
    " 12:00:00.095 [main] INFO  com.example.App - step 95                            "
    " 12:00:00.096 [main] INFO  com.example.App - step 96                            "
    " 12:00:00.097 [main] INFO  com.example.App - step 97                            "
    " 12:00:00.098 [main] INFO  com.example.App - step 98                            "
    " 12:00:00.099 [main] INFO  com.example.App - step 99                            "
    "▶12:00:00.100 [main] INFO  com.example.App - step 100                           "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn page_down_shows_the_next_screen() {
    let out = screen_after(&mut hundred_lines(), &[key(KeyCode::PageDown)]);
    assert_snapshot!(out, @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.023 [main] INFO  com.example.App - step 23                            "
    " 12:00:00.024 [main] INFO  com.example.App - step 24                            "
    " 12:00:00.025 [main] INFO  com.example.App - step 25                            "
    " 12:00:00.026 [main] INFO  com.example.App - step 26                            "
    " 12:00:00.027 [main] INFO  com.example.App - step 27                            "
    " 12:00:00.028 [main] INFO  com.example.App - step 28                            "
    " 12:00:00.029 [main] INFO  com.example.App - step 29                            "
    " 12:00:00.030 [main] INFO  com.example.App - step 30                            "
    " 12:00:00.031 [main] INFO  com.example.App - step 31                            "
    " 12:00:00.032 [main] INFO  com.example.App - step 32                            "
    " 12:00:00.033 [main] INFO  com.example.App - step 33                            "
    " 12:00:00.034 [main] INFO  com.example.App - step 34                            "
    " 12:00:00.035 [main] INFO  com.example.App - step 35                            "
    " 12:00:00.036 [main] INFO  com.example.App - step 36                            "
    " 12:00:00.037 [main] INFO  com.example.App - step 37                            "
    " 12:00:00.038 [main] INFO  com.example.App - step 38                            "
    " 12:00:00.039 [main] INFO  com.example.App - step 39                            "
    " 12:00:00.040 [main] INFO  com.example.App - step 40                            "
    " 12:00:00.041 [main] INFO  com.example.App - step 41                            "
    " 12:00:00.042 [main] INFO  com.example.App - step 42                            "
    " 12:00:00.043 [main] INFO  com.example.App - step 43                            "
    " 12:00:00.044 [main] INFO  com.example.App - step 44                            "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn empty_file_shows_an_empty_body() {
    let mut app = named_app("/var/log/empty.log", b"");
    assert_snapshot!(screen_after(&mut app, &[]), @r#"
    "vilog ─ empty.log ─ 0 B ─ Raw ─ PAUSED ●                                        "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn no_matching_entries_shows_a_notice_in_place_of_the_rows() {
    let mut app = app(numbered_log(3).as_bytes());
    app.set_filters(Filters {
        text: Some(Matcher::new("nothing", matcher::Options::default()).unwrap()),
        ..Filters::default()
    });
    wait_for_filter(&mut app);
    assert_snapshot!(screen_after(&mut app, &[]), @r#"
    "vilog ─ app.log ─ 153 B ─ Raw ─ PAUSED ●                                        "
    "Filters: text[nothing]                                                          "
    "no entries match the current filters                                            "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn level_panel_shows_a_checkbox_per_level_over_the_rows() {
    let keys = [
        key(KeyCode::F(2)),
        char_key('j'),
        char_key('j'),
        char_key('4'),
    ];
    assert_snapshot!(screen_after(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [main] INFO  com.example.App - step 1                             "
    " 12:00:00.002 [main] INFO  com.example.App - step 2                             "
    " 12:00:00.003 [main] INFO  com.example.App - step 3                             "
    " 12:00:00.004 [main] INFO  com.example.App - step 4                             "
    " 12:00:00.005 [main] INFO  com.example.App - step 5                             "
    " 12:00:00.006 [main] INFO  com.example.App - step 6                             "
    " 12:00:00.007 [main] INFO  com.example.App - step 7                             "
    " 12:00:00.008 [main] INFO  com.e┌ Levels ──────┐p 8                             "
    " 12:00:00.009 [main] INFO  com.e│  [x] 1 TRACE │p 9                             "
    " 12:00:00.010 [main] INFO  com.e│  [x] 2 DEBUG │p 10                            "
    " 12:00:00.011 [main] INFO  com.e│ ▶[x] 3 INFO  │p 11                            "
    " 12:00:00.012 [main] INFO  com.e│  [ ] 4 WARN  │p 12                            "
    " 12:00:00.013 [main] INFO  com.e│  [x] 5 ERROR │p 13                            "
    " 12:00:00.014 [main] INFO  com.e│  [x] 6 FATAL │p 14                            "
    " 12:00:00.015 [main] INFO  com.e└──────────────┘p 15                            "
    " 12:00:00.016 [main] INFO  com.example.App - step 16                            "
    " 12:00:00.017 [main] INFO  com.example.App - step 17                            "
    " 12:00:00.018 [main] INFO  com.example.App - step 18                            "
    " 12:00:00.019 [main] INFO  com.example.App - step 19                            "
    " 12:00:00.020 [main] INFO  com.example.App - step 20                            "
    " 12:00:00.021 [main] INFO  com.example.App - step 21                            "
    " 12:00:00.022 [main] INFO  com.example.App - step 22                            "
    "j/k Move  Space/1-6 Toggle  Enter Apply  Esc Cancel                             "
    "#);
}

/// The 80×24 screen after pressing `keys` as the event loop hands them over
/// ([`App::key`]), drawn after each.
fn screen_after_pressing(app: &mut App, keys: &[KeyEvent]) -> String {
    terminal_after_pressing(app, keys).backend().to_string()
}

/// The 80×24 terminal after pressing `keys` as [`screen_after_pressing`] does.
fn terminal_after_pressing(app: &mut App, keys: &[KeyEvent]) -> Terminal<TestBackend> {
    let mut terminal = drawn(app, 80, 24, Theme::Styled);
    for &key in keys {
        app.key(key);
        draw(&mut terminal, app, Theme::Styled);
    }
    terminal
}

/// The keys that open the class panel and type `lines` into it, one per line.
fn class_panel_keys(lines: &[&str]) -> Vec<KeyEvent> {
    let mut keys = vec![key(KeyCode::F(6))];
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            keys.push(key(KeyCode::Down));
        }
        keys.extend(line.chars().map(char_key));
    }
    keys
}

#[test]
fn class_panel_shows_one_rule_per_line_over_the_rows() {
    let keys = class_panel_keys(&["com.myco.*", "!org.hibernate"]);
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [main] INFO  com.example.App - step 1                             "
    " 12:00:00.002 [main] INFO  com.example.App - step 2                             "
    " 12:00:00.003 [main] INFO  com.example.App - step 3                             "
    " 12:00:00.004 [main] INFO  com.example.App - step 4                             "
    " 12:00:00.005 [main] INFO  com.example.App - step 5                             "
    " 12:00:00.006 [main] INFO  com.example.App - step 6                             "
    " 12:00:00.007 [mai┌ Loggers ─────────────────────────────────┐                  "
    " 12:00:00.008 [mai│  com.myco.*                              │                  "
    " 12:00:00.009 [mai│ ▶!org.hibernate                          │                  "
    " 12:00:00.010 [mai│                                          │                  "
    " 12:00:00.011 [mai│                                          │                  "
    " 12:00:00.012 [mai│                                          │                  "
    " 12:00:00.013 [mai│                                          │                  "
    " 12:00:00.014 [mai│                                          │                  "
    " 12:00:00.015 [mai│                                          │                  "
    " 12:00:00.016 [mai└──────────────────────────────────────────┘                  "
    " 12:00:00.017 [main] INFO  com.example.App - step 17                            "
    " 12:00:00.018 [main] INFO  com.example.App - step 18                            "
    " 12:00:00.019 [main] INFO  com.example.App - step 19                            "
    " 12:00:00.020 [main] INFO  com.example.App - step 20                            "
    " 12:00:00.021 [main] INFO  com.example.App - step 21                            "
    " 12:00:00.022 [main] INFO  com.example.App - step 22                            "
    "↑/↓ Line (↓ at the end adds one)  Enter Apply  Esc Cancel                       "
    "#);
}

#[test]
fn class_panel_error_replaces_the_key_hints_in_the_footer() {
    // The second error is longer than the panel is wide.
    for rule in ["!", "com.*.Order"] {
        let mut keys = class_panel_keys(&["Order", rule]);
        keys.push(key(KeyCode::Enter));
        let terminal = terminal_after_pressing(&mut hundred_lines(), &keys);
        let error = Classes::parse(["Order", rule]).unwrap_err();
        assert_eq!(row(&terminal, 23), error, "{rule}");
        assert_eq!(
            row(&terminal, 16),
            format!(" 12:00:00.016 [mai└{}┘", "─".repeat(42)),
            "{rule}"
        );
    }
}

/// The keys that open the time panel and type `spec` into it.
fn time_panel_keys(spec: &str) -> Vec<KeyEvent> {
    let mut keys = vec![key(KeyCode::F(5))];
    keys.extend(spec.chars().map(char_key));
    keys
}

#[test]
fn time_panel_shows_the_spec_and_the_filter_it_makes_over_the_rows() {
    let mut keys = time_panel_keys("10m");
    keys.push(key(KeyCode::Tab));
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [main] INFO  com.example.App - step 1                             "
    " 12:00:00.002 [main] INFO  com.example.App - step 2                             "
    " 12:00:00.003 [main] INFO  com.example.App - step 3                             "
    " 12:00:00.004 [main] INFO  com.example.App - step 4                             "
    " 12:00:00.005 [main] INFO  com.example.App - step 5                             "
    " 12:00:00.006 [main] INFO  com.example.App - step 6                             "
    " 12:00:00.007 [main] INFO  com.example.App - step 7                             "
    " 12:00:00.008 [main] INFO  com.example.App - step 8                             "
    " 12:00:00.009 [main] INFO  com.example.App - step 9                             "
    " 12:00:00.010 [main]┌ Time ────────────────────────────────┐                    "
    " 12:00:00.011 [main]│ ▶10m                                 │                    "
    " 12:00:00.012 [main]│  last 10m (now)                      │                    "
    " 12:00:00.013 [main]└──────────────────────────────────────┘                    "
    " 12:00:00.014 [main] INFO  com.example.App - step 14                            "
    " 12:00:00.015 [main] INFO  com.example.App - step 15                            "
    " 12:00:00.016 [main] INFO  com.example.App - step 16                            "
    " 12:00:00.017 [main] INFO  com.example.App - step 17                            "
    " 12:00:00.018 [main] INFO  com.example.App - step 18                            "
    " 12:00:00.019 [main] INFO  com.example.App - step 19                            "
    " 12:00:00.020 [main] INFO  com.example.App - step 20                            "
    " 12:00:00.021 [main] INFO  com.example.App - step 21                            "
    " 12:00:00.022 [main] INFO  com.example.App - step 22                            "
    "↑/↓ Preset  Tab File/now  Enter Apply  Esc Cancel                               "
    "#);
}

#[test]
fn time_panel_error_replaces_the_key_hints_in_the_footer() {
    let mut keys = time_panel_keys("soon");
    keys.push(key(KeyCode::Enter));
    let terminal = terminal_after_pressing(&mut hundred_lines(), &keys);
    assert_eq!(row(&terminal, 23), TimeSpec::parse("soon").unwrap_err());
}

/// The keys that open the text filter prompt, type `pattern` into it and toggle
/// regex.
fn regex_prompt_keys(pattern: &str) -> Vec<KeyEvent> {
    let mut keys = vec![key(KeyCode::F(4))];
    keys.extend(pattern.chars().map(char_key));
    keys.push(ctrl('r'));
    keys
}

#[test]
fn text_prompt_shows_the_pattern_and_its_options_over_the_rows() {
    let keys = regex_prompt_keys("time.*out");
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [main] INFO  com.example.App - step 1                             "
    " 12:00:00.002 [main] INFO  com.example.App - step 2                             "
    " 12:00:00.003 [main] INFO  com.example.App - step 3                             "
    " 12:00:00.004 [main] INFO  com.example.App - step 4                             "
    " 12:00:00.005 [main] INFO  com.example.App - step 5                             "
    " 12:00:00.006 [main] INFO  com.example.App - step 6                             "
    " 12:00:00.007 [main] INFO  com.example.App - step 7                             "
    " 12:00:00.008 [main] INFO  com.example.App - step 8                             "
    " 12:00:00.009 [main] INFO  com.example.App - step 9                             "
    " 12:00:00.┌ Text filter ─────────────────────────────────────────────┐          "
    " 12:00:00.│ ▶time.*out                                               │          "
    " 12:00:00.│  [x] regex  [ ] ignore case                              │          "
    " 12:00:00.└──────────────────────────────────────────────────────────┘          "
    " 12:00:00.014 [main] INFO  com.example.App - step 14                            "
    " 12:00:00.015 [main] INFO  com.example.App - step 15                            "
    " 12:00:00.016 [main] INFO  com.example.App - step 16                            "
    " 12:00:00.017 [main] INFO  com.example.App - step 17                            "
    " 12:00:00.018 [main] INFO  com.example.App - step 18                            "
    " 12:00:00.019 [main] INFO  com.example.App - step 19                            "
    " 12:00:00.020 [main] INFO  com.example.App - step 20                            "
    " 12:00:00.021 [main] INFO  com.example.App - step 21                            "
    " 12:00:00.022 [main] INFO  com.example.App - step 22                            "
    "Ctrl-r Regex  Tab Ignore case  Enter Apply  Esc Cancel                          "
    "#);
}

#[test]
fn text_prompt_error_replaces_the_key_hints_in_the_footer() {
    let mut keys = regex_prompt_keys("(");
    keys.push(key(KeyCode::Enter));
    let terminal = terminal_after_pressing(&mut hundred_lines(), &keys);
    let options = matcher::Options {
        regex: true,
        ignore_case: false,
    };
    assert_eq!(row(&terminal, 23), Matcher::new("(", options).unwrap_err());
}

// --- level counters and the filter bar ---

/// Entries of four levels and three classes, five minutes apart at most, most of
/// them about a timeout.
const FILTER_LOG: &str = concat!(
    "2026-10-07 12:04:00.000 [main] WARN  com.myco.pay.PayClient - Retry 1/3 after timeout\n",
    "2026-10-07 12:05:00.000 [main] ERROR com.myco.pay.PayClient - Payment timeout\n",
    "2026-10-07 12:06:00.000 [main] INFO  com.myco.pay.PayClient - Payment retried after timeout\n",
    "2026-10-07 12:07:00.000 [main] DEBUG com.myco.db.Sql - select timeout\n",
    "2026-10-07 12:08:00.000 [main] ERROR com.myco.order.Orders - Order 43 failed\n",
    "2026-10-07 12:09:00.000 [main] WARN  org.hibernate.SQL - Slow query timeout\n",
    "2026-10-07 12:10:00.000 [main] WARN  com.myco.pay.PayClient - Retry 2/3 after timeout\n",
);

#[test]
fn without_filters_the_header_shows_the_level_counters_and_there_is_no_filter_bar() {
    let mut app = app_in(built_in("logback"), FILTER_LOG.as_bytes());
    assert_snapshot!(screen_after(&mut app, &[]), @r#"
    "vilog ─ app.log ─ 565 B ─ Logback ─ PAUSED ●                     E:2 W:3 I:1 D:1"
    "▶12:04:00.000 WARN  com.myco.pay.PayClient   Retry 1/3 after timeout            "
    " 12:05:00.000 ERROR com.myco.pay.PayClient   Payment timeout                    "
    " 12:06:00.000 INFO  com.myco.pay.PayClient   Payment retried after timeout      "
    " 12:07:00.000 DEBUG com.myco.db.Sql          select timeout                     "
    " 12:08:00.000 ERROR com.myco.order.Orders    Order 43 failed                    "
    " 12:09:00.000 WARN  org.hibernate.SQL        Slow query timeout                 "
    " 12:10:00.000 WARN  com.myco.pay.PayClient   Retry 2/3 after timeout            "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

/// `-l crit,wrn` on entries written with level aliases: the FATAL and WARN entries
/// stay, under their canonical names.
#[test]
fn level_aliases_of_the_level_option_show_the_entries_under_canonical_names() {
    let log = "\
2026-10-07 12:00:00.000 [main] Information com.myco.App - Started
2026-10-07 12:00:01.000 [main] wrn com.myco.App - Pool almost full
2026-10-07 12:00:02.000 [main] ERR com.myco.App - Payment failed
2026-10-07 12:00:03.000 [main] CRITICAL com.myco.App - Out of memory
";
    let mut app = app_in(built_in("logback"), log.as_bytes());
    app.set_filters(Filters {
        levels: crate::cli::parse_levels("crit,wrn").unwrap(),
        ..Filters::default()
    });
    wait_for_filter(&mut app);
    assert_snapshot!(lines_after(&mut app, &[], 1..=3), @r"
    Filters: level[W F]
    ▶12:00:01.000 WARN  com.myco.App             Pool almost full
     12:00:03.000 FATAL com.myco.App             Out of memory
    ");
}

/// The counters count the INFO entry the level filter hides, but none of the entries
/// the other filters hide.
#[test]
fn filter_bar_shows_every_active_filter_under_the_header() {
    let mut app = app_in(built_in("logback"), FILTER_LOG.as_bytes());
    app.set_filters(Filters {
        levels: Levels::only(&[Level::Warn, Level::Error, Level::Fatal]),
        classes: Classes::parse(["com.myco.*", "!Sql"]).unwrap(),
        text: Some(Matcher::new("time", matcher::Options::default()).unwrap()),
        ..Filters::default()
    });
    app.set_time_filter(Some(TimeFilter {
        spec: TimeSpec::parse("5m").unwrap(),
        base: Base::File,
    }));
    wait_for_filter(&mut app);
    assert_snapshot!(screen_after(&mut app, &[]), @r#"
    "vilog ─ app.log ─ 565 B ─ Logback ─ PAUSED ●                         E:1 W:1 I:1"
    "Filters: level[W E F]  logger[com.myco.* !Sql]  time[last 5m (file)]  text[time]"
    "▶12:05:00.000 ERROR com.myco.pay.PayClient   Payment timeout                    "
    " 12:10:00.000 WARN  com.myco.pay.PayClient   Retry 2/3 after timeout            "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn filter_bar_names_the_options_of_the_text_filter() {
    let mut app = hundred_lines();
    let options = matcher::Options {
        regex: true,
        ignore_case: true,
    };
    app.set_filters(Filters {
        text: Some(Matcher::new("Time.*out", options).unwrap()),
        ..Filters::default()
    });
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "Filters: text[Time.*out (regex, ignore case)]"
    );
}

#[test]
fn the_footer_shows_a_jump_prompt_as_typed() {
    for (opener, footer) in [(':', ":42"), ('t', "t 42")] {
        let keys = [char_key(opener), char_key('4'), char_key('2')];
        let terminal = terminal_after_pressing(&mut hundred_lines(), &keys);
        assert_eq!(row(&terminal, 23), footer);
    }
}

#[test]
fn a_jump_prompt_error_follows_its_text_in_the_footer() {
    let keys = [':', 'a', 'b', 'c'].map(char_key);
    let terminal = terminal_after_pressing(
        &mut hundred_lines(),
        &[&keys[..], &[key(KeyCode::Enter)]].concat(),
    );
    assert_eq!(row(&terminal, 23), ":abc ─ 'abc' is not a line number");
}

#[test]
fn the_footer_shows_the_export_prompt_as_typed() {
    let terminal = terminal_after_pressing(&mut hundred_lines(), &[char_key('e')]);
    assert_eq!(row(&terminal, 23), "export to: app.log.filtered.log");
}

#[test]
fn the_export_prompt_shows_control_characters_in_the_path_as_replacement_characters() {
    let mut app = named_app("/var/log/a\x1bb.log", numbered_log(100).as_bytes());
    let terminal = terminal_after_pressing(&mut app, &[char_key('e')]);
    assert_eq!(row(&terminal, 23), "export to: a\u{fffd}b.log.filtered.log");
}

/// [`hundred_lines`] opened from `app.log` in `dir`, where they are written, with
/// `dir` as its current directory.
fn exporting_hundred_lines(dir: &tempfile::TempDir) -> App {
    let path = dir.path().join("app.log");
    fs::write(&path, numbered_log(100)).unwrap();
    let mut app = named_app(&path, numbered_log(100).as_bytes());
    app.set_current_dir(dir.path());
    app
}

#[test]
fn the_footer_asks_before_overwriting_an_export() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("app.log.filtered.log"), "old\n").unwrap();
    let keys = [char_key('e'), key(KeyCode::Enter)];
    let terminal = terminal_after_pressing(&mut exporting_hundred_lines(&dir), &keys);
    assert_eq!(row(&terminal, 23), "overwrite app.log.filtered.log? y/n");
}

#[test]
fn an_export_prompt_error_follows_its_path_in_the_footer() {
    let dir = tempfile::TempDir::new().unwrap();
    let suggested = "app.log.filtered.log".len();
    let keys = [
        vec![char_key('e')],
        vec![key(KeyCode::Backspace); suggested],
        "./app.log".chars().map(char_key).collect(),
        vec![key(KeyCode::Enter)],
    ]
    .concat();
    let terminal = terminal_after_pressing(&mut exporting_hundred_lines(&dir), &keys);
    assert_eq!(
        row(&terminal, 23),
        "export to: ./app.log ─ cannot export to the file being viewed"
    );
}

#[test]
fn header_ends_with_the_notice_about_the_last_key() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 24, Theme::Styled);
    app.key(char_key('c'));
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ● ─ the selected entry has no logger"
    );
}

#[test]
fn notice_the_viewer_opens_with_is_in_the_header_until_the_first_key() {
    let mut app = hundred_lines();
    app.set_notice(
        "config read from ~/.config/jlv/config.toml, move it to ~/.config/vilog/".to_string(),
    );
    let mut terminal = drawn(&mut app, 120, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ● ─ config read from ~/.config/jlv/config.toml, move it to ~/.config/vilog/"
    );
    app.key(char_key('j'));
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●"
    );
}

/// A headless entry, selected, then an entry of each level from TRACE to ERROR.
const HEADLESS_FIRST_LOG: &str = concat!(
    "starting up\n",
    "2026-10-07 12:00:00.000 [main] TRACE com.example.App - trace\n",
    "2026-10-07 12:00:01.000 [main] DEBUG com.example.App - debug\n",
    "2026-10-07 12:00:02.000 [main] INFO  com.example.App - info\n",
    "2026-10-07 12:00:03.000 [main] WARN  com.example.App - warn\n",
    "2026-10-07 12:00:04.000 [main] ERROR com.example.App - error\n",
);

/// The header at `width` columns after `c` on the headless entry, which has no logger.
fn header_with_a_notice(width: u16) -> String {
    let mut app = app_in(built_in("logback"), HEADLESS_FIRST_LOG.as_bytes());
    let mut terminal = drawn(&mut app, width, 24, Theme::Styled);
    app.key(char_key('c'));
    draw(&mut terminal, &mut app, Theme::Styled);
    row(&terminal, 0)
}

#[test]
fn level_counters_take_the_columns_the_title_leaves_after_a_space() {
    assert_eq!(
        header_with_a_notice(99),
        "vilog ─ app.log ─ 315 B ─ Logback ─ PAUSED ● ─ the selected entry has no logger E:1 W:1 I:1 D:1 T:1"
    );
}

#[test]
fn level_counters_that_do_not_fit_after_the_title_are_left_out() {
    assert_eq!(
        header_with_a_notice(98),
        "vilog ─ app.log ─ 315 B ─ Logback ─ PAUSED ● ─ the selected entry has no logger"
    );
}

#[test]
fn header_shows_the_indexing_progress_until_the_index_is_complete() {
    let mut app = unindexed_app(built_in("raw"), numbered_log(100).as_bytes());
    assert_snapshot!(screen_after(&mut app, &[]), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ● ─ indexing 0%                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn header_names_the_preset_the_file_is_read_in() {
    let mut app = app_in(
        built_in("logback"),
        b"2026-10-07 12:00:00.123 [main] INFO  com.example.App - Started\n",
    );
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 63 B ─ Logback ─ PAUSED ●                                  I:1"
    );
}

#[test]
fn empty_file_shows_no_indexing_progress() {
    let mut app = unindexed_app(built_in("raw"), b"");
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(row(&terminal, 0), "vilog ─ app.log ─ 0 B ─ Raw ─ PAUSED ●");
}

#[test]
fn too_small_terminal_says_so_instead_of_showing_lines() {
    let terminal = drawn(&mut hundred_lines(), 60, 9, Theme::Styled);
    assert_snapshot!(terminal.backend().to_string(), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                   "
    "terminal too small (80×24 needed)                           "
    "                                                            "
    "                                                            "
    "                                                            "
    "                                                            "
    "                                                            "
    "                                                            "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom"
    "#);
}

#[test]
fn too_small_terminal_shows_no_filter_bar() {
    let mut app = hundred_lines();
    app.set_filters(Filters {
        text: Some(Matcher::new("step", matcher::Options::default()).unwrap()),
        ..Filters::default()
    });
    let terminal = drawn(&mut app, 60, 9, Theme::Styled);
    assert_eq!(row(&terminal, 1), TOO_SMALL);
}

/// The buffer's debug view lists every change of style: the header in bold reverse
/// video with the dot green on it, the selected line and the footer in reverse video,
/// the rest plain.
#[test]
fn styled_theme_marks_header_selection_and_footer() {
    let terminal = drawn(&mut hundred_lines(), 80, 24, Theme::Styled);
    assert_snapshot!(format!("{:?}", terminal.backend().buffer()), @r#"
    Buffer {
        area: Rect { x: 0, y: 0, width: 80, height: 24 },
        content: [
            "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       ",
            "▶12:00:00.001 [main] INFO  com.example.App - step 1                             ",
            " 12:00:00.002 [main] INFO  com.example.App - step 2                             ",
            " 12:00:00.003 [main] INFO  com.example.App - step 3                             ",
            " 12:00:00.004 [main] INFO  com.example.App - step 4                             ",
            " 12:00:00.005 [main] INFO  com.example.App - step 5                             ",
            " 12:00:00.006 [main] INFO  com.example.App - step 6                             ",
            " 12:00:00.007 [main] INFO  com.example.App - step 7                             ",
            " 12:00:00.008 [main] INFO  com.example.App - step 8                             ",
            " 12:00:00.009 [main] INFO  com.example.App - step 9                             ",
            " 12:00:00.010 [main] INFO  com.example.App - step 10                            ",
            " 12:00:00.011 [main] INFO  com.example.App - step 11                            ",
            " 12:00:00.012 [main] INFO  com.example.App - step 12                            ",
            " 12:00:00.013 [main] INFO  com.example.App - step 13                            ",
            " 12:00:00.014 [main] INFO  com.example.App - step 14                            ",
            " 12:00:00.015 [main] INFO  com.example.App - step 15                            ",
            " 12:00:00.016 [main] INFO  com.example.App - step 16                            ",
            " 12:00:00.017 [main] INFO  com.example.App - step 17                            ",
            " 12:00:00.018 [main] INFO  com.example.App - step 18                            ",
            " 12:00:00.019 [main] INFO  com.example.App - step 19                            ",
            " 12:00:00.020 [main] INFO  com.example.App - step 20                            ",
            " 12:00:00.021 [main] INFO  com.example.App - step 21                            ",
            " 12:00:00.022 [main] INFO  com.example.App - step 22                            ",
            "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            ",
        ],
        styles: [
            x: 0, y: 0, fg: Reset, bg: Reset, underline: Reset, modifier: BOLD | REVERSED,
            x: 40, y: 0, fg: Reset, bg: Green, underline: Reset, modifier: BOLD | REVERSED,
            x: 41, y: 0, fg: Reset, bg: Reset, underline: Reset, modifier: BOLD | REVERSED,
            x: 0, y: 1, fg: Reset, bg: Reset, underline: Reset, modifier: REVERSED,
            x: 0, y: 2, fg: Reset, bg: Reset, underline: Reset, modifier: NONE,
            x: 0, y: 23, fg: Reset, bg: Reset, underline: Reset, modifier: REVERSED,
        ]
    }
    "#);
}

/// The buffer's debug view lists every change of style; a single entry at 0,0 means
/// the whole screen is in the terminal's default style.
#[test]
fn plain_theme_draws_the_screen_without_any_style() {
    let terminal = drawn(&mut hundred_lines(), 80, 24, Theme::Plain);
    assert_snapshot!(format!("{:?}", terminal.backend().buffer()), @r#"
    Buffer {
        area: Rect { x: 0, y: 0, width: 80, height: 24 },
        content: [
            "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       ",
            "▶12:00:00.001 [main] INFO  com.example.App - step 1                             ",
            " 12:00:00.002 [main] INFO  com.example.App - step 2                             ",
            " 12:00:00.003 [main] INFO  com.example.App - step 3                             ",
            " 12:00:00.004 [main] INFO  com.example.App - step 4                             ",
            " 12:00:00.005 [main] INFO  com.example.App - step 5                             ",
            " 12:00:00.006 [main] INFO  com.example.App - step 6                             ",
            " 12:00:00.007 [main] INFO  com.example.App - step 7                             ",
            " 12:00:00.008 [main] INFO  com.example.App - step 8                             ",
            " 12:00:00.009 [main] INFO  com.example.App - step 9                             ",
            " 12:00:00.010 [main] INFO  com.example.App - step 10                            ",
            " 12:00:00.011 [main] INFO  com.example.App - step 11                            ",
            " 12:00:00.012 [main] INFO  com.example.App - step 12                            ",
            " 12:00:00.013 [main] INFO  com.example.App - step 13                            ",
            " 12:00:00.014 [main] INFO  com.example.App - step 14                            ",
            " 12:00:00.015 [main] INFO  com.example.App - step 15                            ",
            " 12:00:00.016 [main] INFO  com.example.App - step 16                            ",
            " 12:00:00.017 [main] INFO  com.example.App - step 17                            ",
            " 12:00:00.018 [main] INFO  com.example.App - step 18                            ",
            " 12:00:00.019 [main] INFO  com.example.App - step 19                            ",
            " 12:00:00.020 [main] INFO  com.example.App - step 20                            ",
            " 12:00:00.021 [main] INFO  com.example.App - step 21                            ",
            " 12:00:00.022 [main] INFO  com.example.App - step 22                            ",
            "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            ",
        ],
        styles: [
            x: 0, y: 0, fg: Reset, bg: Reset, underline: Reset, modifier: NONE,
        ]
    }
    "#);
}

// --- the table ---

/// An entry of every level, one of them with a stack trace, a thread name longer than
/// the thread column and a class name that is abbreviated to fit.
const TABLE_LOG: &str = concat!(
    "2026-10-07 12:00:00.100 [main] TRACE com.example.App - Tick\n",
    "2026-10-07 12:00:00.200 [main] DEBUG com.example.App - Config loaded\n",
    "2026-10-07 12:00:00.300 [http-nio-8080-exec-3] INFO  com.example.order.OrderService - Order 42 created\n",
    "2026-10-07 12:00:01.000 [main] WARN  com.example.App - Retry 1/3\n",
    "2026-10-07 12:00:02.000 [main] ERROR com.example.App - Failed\n",
    "java.lang.IllegalStateException: boom\n",
    "\tat com.example.App.main(App.java:10)\n",
    "2026-10-07 12:00:03.000 [main] FATAL com.example.App - Giving up\n",
);

fn table() -> App {
    app_in(built_in("logback"), TABLE_LOG.as_bytes())
}

/// The buffer's debug view lists every change of style; a single entry at 0,0 means
/// the whole screen is in the terminal's default style.
#[test]
fn plain_theme_draws_the_table_without_any_style() {
    let terminal = drawn(&mut table(), 80, 24, Theme::Plain);
    assert_snapshot!(format!("{:?}", terminal.backend().buffer()), @r#"
    Buffer {
        area: Rect { x: 0, y: 0, width: 80, height: 24 },
        content: [
            "vilog ─ app.log ─ 500 B ─ Logback ─ PAUSED ●             F:1 E:1 W:1 I:1 D:1 T:1",
            "▶12:00:00.100 TRACE com.example.App          Tick                               ",
            " 12:00:00.200 DEBUG com.example.App          Config loaded                      ",
            " 12:00:00.300 INFO  c.e.order.OrderService   Order 42 created                   ",
            " 12:00:01.000 WARN  com.example.App          Retry 1/3                          ",
            " 12:00:02.000 ERROR com.example.App          Failed [+2]                        ",
            " 12:00:03.000 FATAL com.example.App          Giving up                          ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "                                                                                ",
            "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            ",
        ],
        styles: [
            x: 0, y: 0, fg: Reset, bg: Reset, underline: Reset, modifier: NONE,
        ]
    }
    "#);
}

// Where the columns start at 80 columns, after the `▶` gutter.
const TIMESTAMP_COLUMN: u16 = 1;
const LEVEL_COLUMN: u16 = 14;
const MESSAGE_COLUMN: u16 = 45;

#[test]
fn styled_table_colours_only_the_level_column() {
    let terminal = drawn(&mut table(), 80, 24, Theme::Styled);
    let buffer = terminal.backend().buffer();
    let levels = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
        Level::Fatal,
    ];
    for (y, level) in (1..).zip(levels) {
        let style = Config::default().colors.level(Some(level));
        let cell = &buffer[(LEVEL_COLUMN, y)];
        assert_eq!(
            (cell.fg, cell.bg),
            (
                style.fg.unwrap_or(Color::Reset),
                style.bg.unwrap_or(Color::Reset)
            ),
            "{level:?}"
        );
        assert!(cell.modifier.contains(style.add_modifier), "{level:?}");
        for x in [TIMESTAMP_COLUMN, MESSAGE_COLUMN] {
            assert_eq!(buffer[(x, y)].fg, Color::Reset, "column {x} of {level:?}");
        }
    }
}

#[test]
fn capital_t_shows_the_thread_column() {
    let out = lines_after(&mut table(), &[char_key('T')], 1..=6);
    assert_snapshot!(out, @r"
    ▶12:00:00.100 TRACE main            com.example.App          Tick
     12:00:00.200 DEBUG main            com.example.App          Config loaded
     12:00:00.300 INFO  …io-8080-exec-3 c.e.order.OrderService   Order 42 created
     12:00:01.000 WARN  main            com.example.App          Retry 1/3
     12:00:02.000 ERROR main            com.example.App          Failed [+2]
     12:00:03.000 FATAL main            com.example.App          Giving up
    ");
}

#[test]
fn capital_t_twice_hides_the_thread_column_again() {
    let toggled = screen_after(&mut table(), &[char_key('T'), char_key('T')]);
    assert_eq!(toggled, screen_after(&mut table(), &[]));
}

#[test]
fn timestamps_show_the_date_from_100_columns() {
    let terminal = drawn(&mut table(), 100, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "▶2026-10-07 12:00:00.100 TRACE com.example.App                Tick"
    );
}

#[test]
fn class_column_grows_to_at_most_40_columns() {
    let terminal = drawn(&mut table(), 200, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "▶2026-10-07 12:00:00.100 TRACE com.example.App                          Tick"
    );
}

#[test]
fn level_column_names_the_level_however_the_line_spells_it() {
    let mut app = app_in(
        built_in("logback"),
        b"2026-10-07 12:00:00.100 [main] warning com.example.App - Retry\n",
    );
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "▶12:00:00.100 WARN  com.example.App          Retry"
    );
}

// --- configuration ---

/// [`table`] configured with the `[ui]` lines `ui` and the sections `sections`, the
/// histogram hidden unless `ui` shows it.
fn configured_table(ui: &str, sections: &str) -> App {
    let text = format!("[ui]\nshow_histogram = false\n{ui}\n{sections}");
    let mut app = table();
    app.configure(&Config::parse(&text).unwrap());
    app
}

const DAY_FIRST: &str = "timestamp_format = \"%d.%m. %H:%M:%S\"";

#[test]
fn configured_timestamp_format_is_shown_from_100_columns() {
    let terminal = drawn(&mut configured_table(DAY_FIRST, ""), 100, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "▶07.10. 12:00:00 TRACE com.example.App                Tick"
    );
}

#[test]
fn below_100_columns_timestamps_show_the_time_of_day_whatever_the_format() {
    let terminal = drawn(&mut configured_table(DAY_FIRST, ""), 99, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "▶12:00:00.100 TRACE com.example.App               Tick"
    );
}

#[test]
fn configured_level_colours_paint_the_level_column() {
    let colors = concat!(
        "[colors]\n",
        "trace = \"blue\"\n",
        "debug = \"magenta\"\n",
        "info = \"lightred\"\n",
        "warn = \"white\"\n",
        "error = \"lightcyan\"\n",
        "fatal = { fg = \"black\", bg = \"lightyellow\" }\n",
    );
    let terminal = drawn(&mut configured_table("", colors), 80, 24, Theme::Styled);
    let buffer = terminal.backend().buffer();
    let expected = [
        (Color::Blue, Color::Reset),
        (Color::Magenta, Color::Reset),
        (Color::LightRed, Color::Reset),
        (Color::White, Color::Reset),
        (Color::LightCyan, Color::Reset),
        (Color::Black, Color::LightYellow),
    ];
    for (y, colours) in (1..).zip(expected) {
        let cell = &buffer[(LEVEL_COLUMN, y)];
        assert_eq!((cell.fg, cell.bg), colours, "line {y}");
    }
}

#[test]
fn configured_line_number_colour_paints_the_gutter() {
    let mut app = configured_table("", "[colors]\nline_number = \"lightblue\"\n");
    let terminal = numbered_terminal(&mut app, 80, &[]);
    let buffer = terminal.backend().buffer();
    for y in 2..=6 {
        assert_eq!(buffer[(3, y)].fg, Color::LightBlue, "number on line {y}");
    }
}

// --- long messages ---

/// A message of 62 columns, longer than the 35 of the message column at 80 columns,
/// and a short one.
const LONG_MESSAGE_LOG: &str = concat!(
    "2026-10-07 12:00:00.100 [main] INFO  com.example.App - ",
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\n",
    "2026-10-07 12:00:00.200 [main] WARN  com.example.App - short message\n",
);

fn long_messages() -> App {
    app_in(built_in("logback"), LONG_MESSAGE_LOG.as_bytes())
}

#[test]
fn right_arrow_scrolls_the_messages_by_8_columns() {
    let out = lines_after(&mut long_messages(), &[key(KeyCode::Right)], 1..=2);
    assert_snapshot!(out, @r"
    ▶12:00:00.100 INFO  com.example.App          89abcdefghijklmnopqrstuvwxyzABCDEFG
     12:00:00.200 WARN  com.example.App          ssage
    ");
}

#[test]
fn scrolling_right_stops_once_the_longest_message_on_screen_ends_at_the_edge() {
    let right = key(KeyCode::Right);
    let out = lines_after(&mut long_messages(), &[right; 4], 1..=1);
    assert_eq!(
        out,
        "▶12:00:00.100 INFO  com.example.App          rstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
    );
}

#[test]
fn scrolling_left_after_the_end_moves_back_8_columns_from_there() {
    let right = key(KeyCode::Right);
    let keys = [right, right, right, right, key(KeyCode::Left)];
    let out = lines_after(&mut long_messages(), &keys, 1..=1);
    assert_eq!(
        out,
        "▶12:00:00.100 INFO  com.example.App          jklmnopqrstuvwxyzABCDEFGHIJKLMNOPQR"
    );
}

#[test]
fn raw_lines_scroll_like_messages() {
    let line = "0123456789".repeat(10);
    let out = lines_after(&mut app(line.as_bytes()), &[key(KeyCode::Right)], 1..=1);
    assert_eq!(out, format!("▶{}", &line[8..87]));
}

#[test]
fn w_wraps_long_messages_onto_the_next_lines_of_the_message_column() {
    let out = lines_after(&mut long_messages(), &[char_key('w')], 1..=3);
    assert_snapshot!(out, @r"
    ▶12:00:00.100 INFO  com.example.App          0123456789abcdefghijklmnopqrstuvwxy
                                                 zABCDEFGHIJKLMNOPQRSTUVWXYZ
     12:00:00.200 WARN  com.example.App          short message
    ");
}

#[test]
fn w_twice_cuts_long_messages_again() {
    let toggled = screen_after(&mut long_messages(), &[char_key('w'), char_key('w')]);
    assert_eq!(toggled, screen_after(&mut long_messages(), &[]));
}

/// 30 entries of two screen lines each at 80 columns, 11 of them on the 22 lines of
/// the body.
fn two_line_entries() -> App {
    let log: String = (1..=30)
        .map(|i| {
            let padding = "x".repeat(40);
            format!(
                "2026-10-07 12:00:00.{i:03} [main] INFO  com.example.App - step {i:02} {padding}\n"
            )
        })
        .collect();
    app_in(built_in("logback"), log.as_bytes())
}

/// The first 13 columns of screen lines `lines` after `w` and then `j` `downs` times.
fn wrapped_starts_after_moving_down(downs: usize, lines: [u16; 3]) -> [String; 3] {
    let mut keys = vec![char_key('w')];
    keys.extend(iter::repeat_n(char_key('j'), downs));
    let terminal = terminal_after(&mut two_line_entries(), &keys);
    lines.map(|y| row(&terminal, y).chars().take(13).collect())
}

#[test]
fn moving_down_within_a_screen_of_wrapped_entries_does_not_scroll() {
    assert_eq!(
        wrapped_starts_after_moving_down(1, [1, 2, 3]),
        [" 12:00:00.001", " ".repeat(13).as_str(), "▶12:00:00.002"]
    );
}

#[test]
fn moving_down_through_wrapped_entries_keeps_the_selected_one_on_screen() {
    assert_eq!(
        wrapped_starts_after_moving_down(11, [1, 21, 22]),
        [" 12:00:00.002", "▶12:00:00.012", " ".repeat(13).as_str()]
    );
}

/// 40 entries, the first 20 of two screen lines at 80 columns and the rest of one,
/// or the other way round when `two_lines_first` is false.
fn mixed_height_entries(two_lines_first: bool) -> App {
    let log: String = (1..=40)
        .map(|i| {
            let padding = if (i <= 20) == two_lines_first {
                "x".repeat(40)
            } else {
                String::new()
            };
            format!(
                "2026-10-07 12:00:00.{i:03} [main] INFO  com.example.App - step {i:02} {padding}\n"
            )
        })
        .collect();
    app_in(built_in("logback"), log.as_bytes())
}

/// The first 13 columns of screen lines `lines` after pressing `keys`.
fn starts_after(app: &mut App, keys: &[KeyEvent], lines: [u16; 3]) -> [String; 3] {
    let terminal = terminal_after(app, keys);
    lines.map(|y| row(&terminal, y).chars().take(13).collect())
}

#[test]
fn bottom_of_wrapped_entries_fills_the_body_up_to_the_last_line() {
    // The last screen holds the last 20 entries and the two-line one before them.
    let keys = [char_key('w'), char_key('G')];
    assert_eq!(
        starts_after(&mut mixed_height_entries(true), &keys, [1, 3, 22]),
        [" 12:00:00.020", " 12:00:00.021", "▶12:00:00.040"]
    );
}

#[test]
fn moving_down_through_the_first_screen_of_wrapped_entries_does_not_scroll() {
    // The first screen holds the 20 one-line entries and the first two-line one.
    let mut keys = vec![char_key('w')];
    keys.extend([char_key('j'); 11]);
    assert_eq!(
        starts_after(&mut mixed_height_entries(false), &keys, [1, 12, 21]),
        [" 12:00:00.001", "▶12:00:00.012", " 12:00:00.021"]
    );
}

#[test]
fn wrapped_messages_continue_under_the_message_column_next_to_the_thread() {
    let keys = [char_key('T'), char_key('w')];
    let out = lines_after(&mut long_messages(), &keys, 1..=5);
    assert_snapshot!(out, @r"
    ▶12:00:00.100 INFO  main            com.example.App          0123456789abcdefghi
                                                                 jklmnopqrstuvwxyzAB
                                                                 CDEFGHIJKLMNOPQRSTU
                                                                 VWXYZ
     12:00:00.200 WARN  main            com.example.App          short message
    ");
}

// --- formats without some columns ---

/// Entries in [`no_thread_or_logger`], one of them with a message longer than the
/// screen and one with a stack trace.
const NO_THREAD_LOG: &str = concat!(
    "2026-10-07 12:00:00.123 [INF] Order 42 created\n",
    "2026-10-07 12:00:01.000 [WRN] Payment retry 1/3 for order 42 after the gateway timed out, waiting 2 s before the next attempt\n",
    "2026-10-07 12:00:02.000 [ERR] Payment failed\n",
    "System.TimeoutException: The operation has timed out.\n",
    "   at Shop.Payments.Gateway.Charge()\n",
);

fn no_thread_table() -> App {
    app_in(no_thread_or_logger(), NO_THREAD_LOG.as_bytes())
}

/// Entries in [`no_timestamp`], one with a logger name abbreviated to fit.
const NO_TIMESTAMP_LOG: &str = concat!(
    "WARNING:app.orders:Payment retry 1/3\n",
    "INFO:app.orders.api.handlers.checkout:Order 42 created\n",
    "ERROR:root:Payment failed\n",
);

fn no_timestamp_table() -> App {
    app_in(no_timestamp(), NO_TIMESTAMP_LOG.as_bytes())
}

/// PRD §7.2: the message takes the width of the columns that aren't there.
#[test]
fn a_format_without_thread_and_logger_shows_neither_column() {
    assert_snapshot!(screen_after(&mut no_thread_table(), &[]), @r#"
    "vilog ─ app.log ─ 309 B ─ No thread or logger ─ PAUSED ●             E:1 W:1 I:1"
    "▶12:00:00.123 INFO  Order 42 created                                            "
    " 12:00:01.000 WARN  Payment retry 1/3 for order 42 after the gateway timed out, "
    " 12:00:02.000 ERROR Payment failed [+2]                                         "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn an_entry_with_a_level_but_no_timestamp_shows_no_timestamp_column() {
    assert_snapshot!(lines_after(&mut no_timestamp_table(), &[], 1..=3), @r"
    ▶WARN  app.orders               Payment retry 1/3
     INFO  a.o.a.handlers.checkout  Order 42 created
     ERROR root                     Payment failed
    ");
}

#[test]
fn capital_t_on_a_format_without_threads_adds_no_column() {
    assert_eq!(
        lines_after(&mut no_thread_table(), &[char_key('T')], 1..=3),
        lines_after(&mut no_thread_table(), &[], 1..=3)
    );
}

#[test]
fn details_pane_of_an_entry_without_logger_and_thread_starts_with_its_message() {
    let keys = [char_key('j'), char_key('j'), key(KeyCode::Enter)];
    assert_snapshot!(lines_after(&mut no_thread_table(), &keys, DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ Payment failed                                                               │
    │ System.TimeoutException: The operation has timed out.                        │
    │    at Shop.Payments.Gateway.Charge()                                         │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn details_pane_of_an_entry_without_a_thread_shows_the_logger_alone() {
    let keys = [key(KeyCode::Enter)];
    assert_snapshot!(lines_after(&mut no_timestamp_table(), &keys, DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ app.orders                                                                   │
    │ Payment retry 1/3                                                            │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

/// The thread keeps its parentheses, so that it doesn't read as a logger name.
#[test]
fn details_pane_of_an_entry_without_a_logger_shows_the_thread_in_parentheses() {
    let log = "2026-10-07 12:00:00.123 [1234] [INFO] Booting worker\n";
    let mut app = app_in(no_logger(), log.as_bytes());
    assert_snapshot!(lines_after(&mut app, &[key(KeyCode::Enter)], DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ (1234)                                                                       │
    │ Booting worker                                                               │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

// --- format detection ---

#[test]
fn unrecognized_format_is_shown_raw_with_a_notice_in_the_header() {
    assert_snapshot!(screen_after(&mut detected_app(b"hello\nworld\n"), &[]), @r#"
    "vilog ─ app.log ─ 12 B ─ Raw ─ PAUSED ● ─ format not recognized, showing raw    "
    "▶hello                                                                          "
    " world                                                                          "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn cycled_preset_replaces_the_notice_in_the_header() {
    let mut app = detected_app(b"hello\nworld\n");
    let terminal = terminal_after(&mut app, &[char_key('P')]);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 12 B ─ Logback ─ PAUSED ● ─ indexing 0%"
    );
}

// --- folding ---

/// Entry `i`, counted from 1, says `step i` and has `frames[i - 1]` stack frames.
fn entries_with_frames(frames: &[usize]) -> App {
    let log: String = (1..)
        .zip(frames)
        .map(|(i, &n)| {
            let frames: String = (1..=n)
                .map(|f| format!("\tat App.step(App.java:{f})\n"))
                .collect();
            format!("2026-10-07 12:00:00.{i:03} [main] INFO  com.example.App - step {i}\n{frames}")
        })
        .collect();
    app_in(built_in("logback"), log.as_bytes())
}

fn traces() -> App {
    entries_with_frames(&[0, 2, 1, 0])
}

#[test]
fn space_unfolds_the_selected_entry_under_its_message_column() {
    let out = lines_after(&mut traces(), &[char_key('j'), char_key(' ')], 1..=6);
    assert_snapshot!(out, @r"
     12:00:00.001 INFO  com.example.App          step 1
    ▶12:00:00.002 INFO  com.example.App          step 2
                                                     at App.step(App.java:1)
                                                     at App.step(App.java:2)
     12:00:00.003 INFO  com.example.App          step 3 [+1]
     12:00:00.004 INFO  com.example.App          step 4
    ");
}

/// The marker is on the entry's first line only, and the selected row shows `▶`
/// whether or not it is bookmarked.
#[test]
fn bookmarked_entries_have_a_marker_in_the_gutter() {
    // Bookmarks step 2, unfolded, then step 3 and the selected step 4.
    let keys: Vec<KeyEvent> = "j mjjjmjm".chars().map(char_key).collect();
    let out = lines_after(&mut traces(), &keys, 1..=6);
    assert_snapshot!(out, @r"
     12:00:00.001 INFO  com.example.App          step 1
    ◆12:00:00.002 INFO  com.example.App          step 2
                                                     at App.step(App.java:1)
                                                     at App.step(App.java:2)
    ◆12:00:00.003 INFO  com.example.App          step 3 [+1]
    ▶12:00:00.004 INFO  com.example.App          step 4
    ");
}

#[test]
fn capital_z_unfolds_every_entry() {
    let out = lines_after(&mut traces(), &[char_key('Z')], 1..=7);
    assert_snapshot!(out, @r"
    ▶12:00:00.001 INFO  com.example.App          step 1
     12:00:00.002 INFO  com.example.App          step 2
                                                     at App.step(App.java:1)
                                                     at App.step(App.java:2)
     12:00:00.003 INFO  com.example.App          step 3
                                                     at App.step(App.java:1)
     12:00:00.004 INFO  com.example.App          step 4
    ");
}

#[test]
fn z_folds_every_entry_again() {
    let keys = [char_key('Z'), char_key('z')];
    assert_eq!(
        screen_after(&mut traces(), &keys),
        screen_after(&mut traces(), &[])
    );
}

#[test]
fn space_on_a_single_line_entry_changes_nothing() {
    let toggled = screen_after(&mut traces(), &[char_key(' ')]);
    assert_eq!(toggled, screen_after(&mut traces(), &[]));
}

/// 30 entries of three lines, more than the 22 lines of the body once unfolded.
fn thirty_traces() -> App {
    entries_with_frames(&[2; 30])
}

#[test]
fn unfolding_all_keeps_the_selected_entry_on_its_screen_line() {
    let mut keys = vec![char_key('j'); 5];
    keys.push(char_key('Z'));
    let out = lines_after(&mut thirty_traces(), &keys, 5..=7);
    assert_snapshot!(out, @r"
                                                     at App.step(App.java:2)
    ▶12:00:00.006 INFO  com.example.App          step 6
                                                     at App.step(App.java:1)
    ");
}

#[test]
fn folding_from_a_stack_frame_selects_its_entry_on_the_same_screen_line() {
    // `j` 40 times selects the first frame of entry 14 on the last line.
    let mut keys = vec![char_key('Z')];
    keys.extend([char_key('j'); 40]);
    keys.push(char_key(' '));
    let out = lines_after(&mut thirty_traces(), &keys, 21..=22);
    assert_snapshot!(out, @r"
                                                     at App.step(App.java:2)
    ▶12:00:00.014 INFO  com.example.App          step 14 [+2]
    ");
}

#[test]
fn folding_the_last_entry_at_the_bottom_keeps_it_on_the_bottom_line() {
    let keys = [char_key('Z'), char_key('G'), char_key(' ')];
    let out = lines_after(&mut thirty_traces(), &keys, 21..=22);
    assert_snapshot!(out, @r"
                                                     at App.step(App.java:2)
    ▶12:00:00.030 INFO  com.example.App          step 30 [+2]
    ");
}

/// 30 single-line entries and a last one of 31 lines, taller than the body.
fn tall_last_entry() -> App {
    let mut frames = vec![0; 30];
    frames.push(30);
    entries_with_frames(&frames)
}

#[test]
fn unfolding_all_at_the_bottom_keeps_an_entry_taller_than_the_screen_on_the_bottom_line() {
    let out = lines_after(
        &mut tall_last_entry(),
        &[char_key('G'), char_key('Z')],
        21..=22,
    );
    assert_snapshot!(out, @r"
     12:00:00.030 INFO  com.example.App          step 30
    ▶12:00:00.031 INFO  com.example.App          step 31
    ");
}

#[test]
fn bottom_after_unfolding_all_selects_the_last_frame_of_an_entry_taller_than_the_screen() {
    let keys = [char_key('G'), char_key('Z'), char_key('G')];
    let out = lines_after(&mut tall_last_entry(), &keys, 21..=22);
    assert_snapshot!(out, @r"
                                                     at App.step(App.java:29)
    ▶                                                at App.step(App.java:30)
    ");
}

#[test]
fn unfolding_all_keeps_a_selected_stack_frame_on_its_screen_line() {
    // Entry 20 unfolded on a scrolled screen, its first frame selected on line 13.
    let mut keys = vec![char_key('G')];
    keys.extend([char_key('k'); 10]);
    keys.extend([char_key(' '), char_key('j'), char_key('Z')]);
    let out = lines_after(&mut thirty_traces(), &keys, 12..=13);
    assert_snapshot!(out, @r"
     12:00:00.020 INFO  com.example.App          step 20
    ▶                                                at App.step(App.java:1)
    ");
}

#[test]
fn wrapped_bottom_of_an_unfolded_entry_taller_than_the_screen_shows_its_last_frames() {
    let mut frames = vec![0; 30];
    frames.push(100);
    let keys = [char_key('w'), char_key('G'), char_key('Z'), char_key('G')];
    let terminal = terminal_after(&mut entries_with_frames(&frames), &keys);
    let frame = |n: usize| format!("    at App.step(App.java:{n})");
    assert_eq!(
        [row(&terminal, 1), row(&terminal, 22)],
        [
            format!(" {:44}{}", "", frame(79)),
            format!("▶{:44}{}", "", frame(100))
        ]
    );
}

// --- selection ---

#[test]
fn moving_down_moves_the_marker_without_scrolling() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 24, Theme::Styled);
    app.apply(Action::Down);
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!(
        [row(&terminal, 1), row(&terminal, 2)],
        [shown(1, false), shown(2, true)]
    );
}

#[test]
fn styled_theme_highlights_the_selected_line() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 24, Theme::Styled);
    app.apply(Action::Down);
    draw(&mut terminal, &mut app, Theme::Styled);
    let style = |y: u16| terminal.backend().buffer()[(1, y)].style();
    assert_ne!(style(2), style(1), "selected line looks like the one above");
    assert_ne!(style(2), style(3), "selected line looks like the one below");
}

// --- files shorter than the screen ---

#[test]
fn file_shorter_than_the_screen_leaves_the_rest_of_the_body_blank() {
    let terminal = drawn(&mut app(b"one\ntwo\nthree\n"), 80, 24, Theme::Styled);
    let body: Vec<String> = (1..=22).map(|y| row(&terminal, y)).collect();
    let mut expected = vec!["▶one".to_string(), " two".into(), " three".into()];
    expected.resize(22, String::new());
    assert_eq!(body, expected);
}

#[test]
fn bottom_of_a_file_shorter_than_the_screen_does_not_scroll() {
    let mut app = app(b"one\ntwo\nthree\n");
    let mut terminal = drawn(&mut app, 80, 24, Theme::Styled);
    app.apply(Action::Bottom);
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!(
        [row(&terminal, 1), row(&terminal, 3)],
        [" one".to_string(), "▶three".to_string()]
    );
}

// --- what a line looks like ---

#[test]
fn stack_trace_lines_start_with_four_columns_for_the_tab() {
    let mut app =
        app(b"java.lang.IllegalStateException: boom\n\tat com.example.App.main(App.java:10)\n");
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 2),
        "     at com.example.App.main(App.java:10)"
    );
}

#[test]
fn escape_sequences_are_shown_as_replacement_characters() {
    let mut app = app(b"\x1b[31mred\x1b[0m and a bell\x07\n");
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 1),
        "▶\u{FFFD}[31mred\u{FFFD}[0m and a bell\u{FFFD}"
    );
}

#[test]
fn wide_characters_take_two_columns_up_to_the_tab_stop() {
    let mut app = app("日本語のログ\tok\n".as_bytes());
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(row(&terminal, 1), "▶日本語のログ    ok");
}

#[test]
fn wide_character_that_does_not_fit_on_the_line_is_left_out() {
    let mut app = app("日".repeat(40).as_bytes());
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(row(&terminal, 1), format!("▶{}", "日".repeat(39)));
}

#[test]
fn long_line_is_cut_at_the_right_edge() {
    let mut app = app("a".repeat(100).as_bytes());
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(row(&terminal, 1), format!("▶{}", "a".repeat(79)));
}

#[test]
fn windows_line_endings_are_not_shown() {
    let terminal = drawn(&mut app(b"one\r\ntwo\r\n"), 80, 24, Theme::Styled);
    assert_eq!([row(&terminal, 1), row(&terminal, 2)], ["▶one", " two"]);
}

// --- header ---

#[test]
fn control_characters_in_the_file_name_are_replaced() {
    let mut app = named_app("/var/log/a\x1bb.log", b"x\n");
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ a\u{FFFD}b.log ─ 2 B ─ Raw ─ PAUSED ●"
    );
}

// --- follow mode ---

/// A viewer opened at the end of `numbered_log(3)`, on the 22 lines an 80×24
/// screen has for rows, with the whole file indexed.
fn three_lines_followed() -> App {
    let mut app = following_app(built_in("raw"), numbered_log(3).as_bytes());
    app.resize(22);
    wait_for_index(&mut app);
    app
}

/// `app` after the file changed as `change` says, with what changed indexed.
fn changed(mut app: App, change: FileChange) -> App {
    app.file_changed(change);
    wait_for_index(&mut app);
    app
}

#[test]
fn following_shows_follow_in_the_header() {
    let out = lines_after(&mut three_lines_followed(), &[], 0..=3);
    assert_snapshot!(out, @r"
    vilog ─ app.log ─ 153 B ─ Raw ─ FOLLOW ●
     12:00:00.001 [main] INFO  com.example.App - step 1
     12:00:00.002 [main] INFO  com.example.App - step 2
    ▶12:00:00.003 [main] INFO  com.example.App - step 3
    ");
}

/// Ten seconds after the growth, the file is quiet.
#[test]
fn paused_shows_the_new_entries_and_the_dot_in_the_header() {
    let start = Instant::now();
    let mut app = three_lines_followed();
    app.tick(start);
    app.apply(Action::Up);
    let grown = FileChange::Grew(test_support::source(numbered_log(5).as_bytes()));
    let mut app = changed(app, grown);
    app.tick(start + Duration::from_secs(10));
    let out = lines_after(&mut app, &[], 0..=5);
    assert_snapshot!(out, @r"
    vilog ─ app.log ─ 255 B ─ Raw ─ PAUSED +2 new ◐
     12:00:00.001 [main] INFO  com.example.App - step 1
    ▶12:00:00.002 [main] INFO  com.example.App - step 2
     12:00:00.003 [main] INFO  com.example.App - step 3
     12:00:00.004 [main] INFO  com.example.App - step 4
     12:00:00.005 [main] INFO  com.example.App - step 5
    ");
}

#[test]
fn truncation_row_starts_the_new_content() {
    let truncated = FileChange::Truncated(test_support::source(numbered_log(2).as_bytes()));
    let out = lines_after(&mut changed(three_lines_followed(), truncated), &[], 0..=3);
    assert_snapshot!(out, @r"
    vilog ─ app.log ─ 102 B ─ Raw ─ FOLLOW ●
     ── file truncated ──
     12:00:00.001 [main] INFO  com.example.App - step 1
    ▶12:00:00.002 [main] INFO  com.example.App - step 2
    ");
}

#[test]
fn missing_file_shows_an_empty_table_and_waiting_for_file_in_the_header() {
    let mut app = changed(three_lines_followed(), FileChange::Gone);
    assert_snapshot!(screen_after(&mut app, &[]), @r#"
    "vilog ─ app.log ─ 0 B ─ Raw ─ WAITING FOR FILE ○                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

/// The header's cell with the dot. The header is in reverse video, so the dot's
/// background is the colour it shows in.
fn dot(terminal: &Terminal<TestBackend>) -> &Cell {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.width)
        .map(|x| &buffer[(x, 0)])
        .find(|cell| ["●", "◐", "○"].contains(&cell.symbol()))
        .expect("no dot in the header")
}

#[test]
fn dot_is_green_then_yellow_then_grey() {
    let start = Instant::now();
    let mut app = three_lines_followed();
    for (after, colour) in [
        (Duration::ZERO, Color::Green),
        (Duration::from_secs(10), Color::Yellow),
        (Duration::from_secs(300), Color::DarkGray),
    ] {
        app.tick(start + after);
        let terminal = drawn(&mut app, 80, 24, Theme::Styled);
        assert_eq!(dot(&terminal).bg, colour, "after {after:?}");
    }
}

#[test]
fn plain_theme_shows_the_dot_by_its_shape_alone() {
    let start = Instant::now();
    let mut app = three_lines_followed();
    app.tick(start);
    app.tick(start + Duration::from_secs(10));
    let terminal = drawn(&mut app, 80, 24, Theme::Plain);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 153 B ─ Raw ─ FOLLOW ◐"
    );
    assert_eq!(dot(&terminal).style(), Cell::EMPTY.style());
}

// --- gzip files ---

/// A viewer over the gzip file `gzip` on an 80×24 screen, opened at the end, with the
/// whole file indexed.
fn gzipped(gzip: &[u8]) -> App {
    let mut app = test_support::gzip_app(gzip);
    app.resize(22);
    wait_for_index(&mut app);
    app
}

/// Going to the bottom doesn't follow a gzip file, so the header doesn't say so.
#[test]
fn gzip_file_is_read_only_in_the_header_also_at_the_bottom() {
    let mut app = gzipped(&test_support::gzip(numbered_log(3).as_bytes()));
    let out = lines_after(&mut app, &[char_key('G')], 0..=3);
    assert_snapshot!(out, @r"
    vilog ─ app.log.gz ─ 153 B ─ Raw ─ READ-ONLY ○
     12:00:00.001 [main] INFO  com.example.App - step 1
     12:00:00.002 [main] INFO  com.example.App - step 2
    ▶12:00:00.003 [main] INFO  com.example.App - step 3
    ");
}

#[test]
fn truncated_gzip_file_has_a_notice_in_the_header() {
    let gzip = test_support::gzip(numbered_log(3).as_bytes());
    // without the trailer: the CRC-32 and the length, 4 bytes each
    let mut app = gzipped(&gzip[..gzip.len() - 8]);
    let out = lines_after(&mut app, &[], 0..=0);
    assert_snapshot!(out, @"vilog ─ app.log.gz ─ 153 B ─ Raw ─ READ-ONLY ○ ─ truncated .gz");
}

// --- terminal size ---

#[test]
fn one_column_short_of_80_is_too_small() {
    let terminal = drawn(&mut hundred_lines(), 79, 24, Theme::Styled);
    assert_eq!(row(&terminal, 1), "terminal too small (80×24 needed)");
}

#[test]
fn one_line_short_of_24_is_too_small() {
    let terminal = drawn(&mut hundred_lines(), 80, 23, Theme::Styled);
    assert_eq!(row(&terminal, 1), "terminal too small (80×24 needed)");
}

#[test]
fn body_fills_the_lines_between_header_and_footer() {
    let mut app = hundred_lines();
    drawn(&mut app, 80, 30, Theme::Styled);
    assert_eq!(app.viewport().height(), 28);
}

#[test]
fn growing_the_terminal_at_the_bottom_fills_it_with_earlier_lines() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 24, Theme::Styled);
    app.apply(Action::Bottom);
    draw(&mut terminal, &mut app, Theme::Styled);
    terminal.backend_mut().resize(80, 30);
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!(
        [row(&terminal, 1), row(&terminal, 28)],
        [shown(73, false), shown(100, true)]
    );
}

#[test]
fn shrinking_the_terminal_at_the_bottom_keeps_the_last_line_selected_on_screen() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 30, Theme::Styled);
    app.apply(Action::Bottom);
    draw(&mut terminal, &mut app, Theme::Styled);
    terminal.backend_mut().resize(80, 24);
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!(
        [row(&terminal, 1), row(&terminal, 22)],
        [shown(79, false), shown(100, true)]
    );
}

#[test]
fn briefly_too_small_terminal_keeps_the_scroll_position() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 24, Theme::Styled);
    for _ in 0..30 {
        app.apply(Action::Down);
    }
    draw(&mut terminal, &mut app, Theme::Styled);
    let before = (app.viewport().top(), app.viewport().selected());
    terminal.backend_mut().resize(60, 9);
    draw(&mut terminal, &mut app, Theme::Styled);
    terminal.backend_mut().resize(80, 24);
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_eq!((app.viewport().top(), app.viewport().selected()), before);
}

// --- NO_COLOR ---

#[test]
fn unset_no_color_allows_styling() {
    assert_eq!(Theme::from_no_color(None), Theme::Styled);
}

#[test]
fn empty_no_color_allows_styling() {
    assert_eq!(Theme::from_no_color(Some(OsStr::new(""))), Theme::Styled);
}

#[test]
fn no_color_set_to_1_turns_styling_off() {
    assert_eq!(Theme::from_no_color(Some(OsStr::new("1"))), Theme::Plain);
}

#[test]
fn no_color_turns_styling_off_whatever_its_value() {
    assert_eq!(Theme::from_no_color(Some(OsStr::new("0"))), Theme::Plain);
    assert_eq!(
        Theme::from_no_color(Some(OsStr::new("false"))),
        Theme::Plain
    );
}

#[test]
fn non_utf8_no_color_turns_styling_off() {
    let value = OsString::from_vec(vec![0xff]);
    assert_eq!(Theme::from_no_color(Some(&value)), Theme::Plain);
}

// --- sizes ---

#[test]
fn zero_bytes() {
    assert_eq!(human_size(0), "0 B");
}

#[test]
fn single_byte() {
    assert_eq!(human_size(1), "1 B");
}

#[test]
fn largest_size_in_bytes() {
    assert_eq!(human_size(1023), "1023 B");
}

#[test]
fn one_kilobyte_is_1024_bytes() {
    assert_eq!(human_size(1024), "1.0 KB");
}

#[test]
fn kilobytes_show_one_decimal() {
    assert_eq!(human_size(1536), "1.5 KB");
}

#[test]
fn tenths_round_down_below_the_half() {
    // 1996 B = 1.949 KB
    assert_eq!(human_size(1996), "1.9 KB");
}

#[test]
fn tenths_round_up_from_above_the_half() {
    // 1997 B = 1.950 2 KB
    assert_eq!(human_size(1997), "2.0 KB");
}

#[test]
fn ten_kilobytes_and_more_keep_the_decimal() {
    assert_eq!(human_size(10 * 1024), "10.0 KB");
}

#[test]
fn largest_size_in_kilobytes() {
    // 1 048 524 B = 1023.949 KB
    assert_eq!(human_size(1_048_524), "1023.9 KB");
}

#[test]
fn size_that_rounds_to_1024_kilobytes_is_a_megabyte() {
    // 1 048 525 B = 1023.950 2 KB
    assert_eq!(human_size(1_048_525), "1.0 MB");
}

#[test]
fn one_byte_short_of_a_megabyte_is_a_megabyte() {
    assert_eq!(human_size(1024 * 1024 - 1), "1.0 MB");
}

#[test]
fn megabytes_show_one_decimal() {
    // 5.5 × 1024² = 5 767 168
    assert_eq!(human_size(5_767_168), "5.5 MB");
}

#[test]
fn whole_kilobytes_just_below_a_megabyte_stay_in_kilobytes() {
    assert_eq!(human_size(1023 * 1024), "1023.0 KB");
}

#[test]
fn largest_size_in_gigabytes() {
    // 1023.9 × 1024³ is about 1 099 404 820 070
    assert_eq!(human_size(1_099_404_820_070), "1023.9 GB");
}

#[test]
fn gigabytes() {
    // 1.8 × 1024³ = 1 932 735 283.2
    assert_eq!(human_size(1_932_735_283), "1.8 GB");
}

#[test]
fn terabytes() {
    assert_eq!(human_size(1 << 40), "1.0 TB");
}

#[test]
fn petabytes() {
    assert_eq!(human_size(1 << 50), "1.0 PB");
}

#[test]
fn exabytes() {
    assert_eq!(human_size(1 << 60), "1.0 EB");
}

#[test]
fn largest_size() {
    assert_eq!(human_size(u64::MAX), "16.0 EB");
}

/// The size `human_size` stands for, in bytes, to compare two of them.
fn bytes_meant(size: &str) -> f64 {
    let (number, unit) = size.split_once(' ').unwrap();
    let power = ["B", "KB", "MB", "GB", "TB", "PB", "EB"]
        .iter()
        .position(|&u| u == unit)
        .unwrap();
    number.parse::<f64>().unwrap() * 1024f64.powi(power as i32)
}

/// Byte counts spread evenly over the units: the bit length is picked first, so plain
/// `any::<u64>()` doesn't land in EB nearly every time.
fn byte_count() -> impl Strategy<Value = u64> {
    (0u32..64, any::<u64>()).prop_map(|(shift, bits)| bits >> shift)
}

proptest! {
    #[test]
    fn size_is_whole_bytes_or_one_decimal_below_1024_of_a_unit(bytes in byte_count()) {
        let size = human_size(bytes);
        prop_assert!(is_well_formed_size(&size), "{} gave {:?}", bytes, size);
    }

    #[test]
    fn bigger_files_never_show_a_smaller_size(
        small in byte_count(),
        step in prop_oneof![0u64..4096, byte_count()],
    ) {
        let big = small.saturating_add(step);
        prop_assert!(
            bytes_meant(&human_size(small)) <= bytes_meant(&human_size(big)),
            "{} shows as {:?}, {} as {:?}", small, human_size(small), big, human_size(big)
        );
    }
}

/// `<0–1023> B` or `<1–1023>.<digit> <unit>`.
fn is_well_formed_size(size: &str) -> bool {
    let Some((number, unit)) = size.split_once(' ') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let no_leading_zero = |s: &str| s == "0" || !s.starts_with('0');
    match number.split_once('.') {
        None => {
            unit == "B"
                && digits(number)
                && no_leading_zero(number)
                && number.parse::<u32>().unwrap() < 1024
        }
        Some((whole, tenth)) => {
            ["KB", "MB", "GB", "TB", "PB", "EB"].contains(&unit)
                && digits(whole)
                && no_leading_zero(whole)
                && tenth.len() == 1
                && digits(tenth)
                && (1..1024).contains(&whole.parse::<u32>().unwrap())
        }
    }
}

// --- printable text ---

#[test]
fn text_without_tabs_or_control_characters_is_unchanged() {
    let text = "12:00:00.123 [main] INFO  Zürich 日本 – done";
    assert!(matches!(printable(text), Cow::Borrowed(t) if t == text));
}

#[test]
fn leading_tab_becomes_four_spaces() {
    assert_eq!(printable("\tat com.example.App"), "    at com.example.App");
}

#[test]
fn tab_moves_to_the_next_multiple_of_four_columns() {
    assert_eq!(printable("ab\tc"), "ab  c");
}

#[test]
fn tab_right_after_three_columns_is_one_space() {
    assert_eq!(printable("key\tvalue"), "key value");
}

#[test]
fn tab_on_a_tab_stop_is_four_spaces() {
    assert_eq!(printable("abcd\te"), "abcd    e");
}

#[test]
fn consecutive_tabs_each_move_to_the_next_stop() {
    assert_eq!(printable("\t\tx"), "        x");
}

#[test]
fn trailing_tab_still_expands() {
    assert_eq!(printable("ab\t"), "ab  ");
}

#[test]
fn wide_characters_count_two_columns_before_a_tab() {
    assert_eq!(printable("日\tx"), "日  x");
    assert_eq!(printable("日本\tx"), "日本    x");
}

#[test]
fn escape_becomes_a_replacement_character() {
    assert_eq!(
        printable("\x1b[31mred\x1b[0m"),
        "\u{FFFD}[31mred\u{FFFD}[0m"
    );
}

#[test]
fn other_c0_control_characters_become_replacement_characters() {
    assert_eq!(
        printable("a\0b\x07c\x08d\re\x0bf\x0cg\x1fh"),
        "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}e\u{FFFD}f\u{FFFD}g\u{FFFD}h"
    );
}

#[test]
fn delete_becomes_a_replacement_character() {
    assert_eq!(printable("a\x7fb"), "a\u{FFFD}b");
}

#[test]
fn c1_control_characters_become_replacement_characters() {
    // U+009B is the one-character CSI that some terminals still obey.
    assert_eq!(printable("a\u{9b}31mb\u{85}c"), "a\u{FFFD}31mb\u{FFFD}c");
}

#[test]
fn replaced_character_counts_one_column_before_a_tab() {
    assert_eq!(printable("\x1b\tx"), "\u{FFFD}   x");
}

#[test]
fn tab_after_a_replaced_character_mid_stop_fills_to_the_stop() {
    assert_eq!(printable("ab\x1b\tx"), "ab\u{FFFD} x");
}

#[test]
fn tab_after_a_wide_character_mid_stop_fills_to_the_stop() {
    assert_eq!(printable("a日\tx"), "a日 x");
}

#[test]
fn wide_character_across_a_tab_stop_moves_the_tab_to_the_next_one() {
    // `日` takes columns 3 and 4, so the tab goes from column 5 to 8
    assert_eq!(printable("abc日\tx"), "abc日   x");
}

#[test]
fn second_tab_counts_from_where_the_first_one_ended() {
    assert_eq!(printable("a\tb\tc"), "a   b   c");
}

#[test]
fn tabs_after_text_of_different_lengths_line_up() {
    assert_eq!(printable("id\tname\tlevel"), "id  name    level");
}

#[test]
fn combining_marks_take_no_column_before_a_tab() {
    assert_eq!(printable("e\u{301}\tx"), "e\u{301}   x");
}

proptest! {
    #[test]
    fn printable_text_has_no_control_characters(text in any::<String>()) {
        let shown = printable(&text);
        prop_assert!(!shown.chars().any(char::is_control), "{:?} gave {:?}", text, shown);
    }

    #[test]
    fn printable_keeps_text_without_control_characters(text in "[^\\p{Cc}]*") {
        prop_assert_eq!(printable(&text), text.as_str());
    }

    #[test]
    fn text_after_a_tab_starts_on_a_tab_stop(
        before in "[a-z日本 ]{0,10}",
        after in "[a-z]{1,5}",
    ) {
        let text = format!("{before}\t{after}");
        let shown = printable(&text);
        let start = shown.len() - after.len();
        prop_assert_eq!(&shown[start..], after.as_str());
        let column = shown[..start].width();
        prop_assert_eq!(column % 4, 0, "{:?}", shown);
        prop_assert!(column > before.width() && column <= before.width() + 4, "{:?}", shown);
    }
}

// --- line numbers ---

/// The `width`×24 terminal with line numbers, drawn, then drawn again after each of
/// `actions`.
fn numbered_terminal(app: &mut App, width: u16, actions: &[Action]) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    draw(&mut terminal, app, Theme::Styled);
    for &action in actions {
        app.apply(action);
        draw(&mut terminal, app, Theme::Styled);
    }
    terminal
}

/// Screen lines `lines` of [`numbered_terminal`], as [`row`] shows them, each in
/// quotes so that the snapshots keep the blanks they start with.
fn numbered(app: &mut App, width: u16, actions: &[Action], lines: RangeInclusive<u16>) -> String {
    let terminal = numbered_terminal(app, width, actions);
    let rows: Vec<String> = lines.map(|y| format!("{:?}", row(&terminal, y))).collect();
    rows.join("\n")
}

/// Numbers are right-aligned, at least 4 wide, before the `▶` marker. A folded entry
/// hides its lines' numbers, so the next entry shows its own.
#[test]
fn gutter_shows_the_file_line_of_each_entry() {
    assert_snapshot!(numbered(&mut table(), 80, &[], 1..=6), @r#"
    "   1 ▶12:00:00.100 TRACE com.example.App          Tick"
    "   2  12:00:00.200 DEBUG com.example.App          Config loaded"
    "   3  12:00:00.300 INFO  c.e.order.OrderService   Order 42 created"
    "   4  12:00:01.000 WARN  com.example.App          Retry 1/3"
    "   5  12:00:02.000 ERROR com.example.App          Failed [+2]"
    "   8  12:00:03.000 FATAL com.example.App          Giving up"
    "#);
}

#[test]
fn hash_hides_the_gutter() {
    let actions = [action(char_key('#')).unwrap()];
    assert_snapshot!(numbered(&mut table(), 80, &actions, 1..=2), @r#"
    "▶12:00:00.100 TRACE com.example.App          Tick"
    " 12:00:00.200 DEBUG com.example.App          Config loaded"
    "#);
}

#[test]
fn unfolded_stack_trace_lines_have_their_own_line_numbers() {
    let out = numbered(&mut traces(), 80, &[Action::UnfoldAll], 1..=7);
    assert_snapshot!(out, @r#"
    "   1 ▶12:00:00.001 INFO  com.example.App          step 1"
    "   2  12:00:00.002 INFO  com.example.App          step 2"
    "   3                                                  at App.step(App.java:1)"
    "   4                                                  at App.step(App.java:2)"
    "   5  12:00:00.003 INFO  com.example.App          step 3"
    "   6                                                  at App.step(App.java:1)"
    "   7  12:00:00.004 INFO  com.example.App          step 4"
    "#);
}

/// At 80 columns, the narrowest screen, 7-digit numbers keep their width and the
/// message column is left with 27.
#[test]
fn seven_digit_line_numbers_take_their_width_from_the_message_column() {
    let entry = |n: u8, message: &str| {
        format!("2026-10-07 12:00:00.00{n} [main] INFO  com.example.App - {message}\n")
    };
    let log = [
        entry(1, "the first message, long enough to be cut"),
        "\n".repeat(999_998),
        entry(2, "the second message, long enough to be cut"),
        entry(3, "done"),
    ]
    .concat();
    let mut app = unindexed_app(built_in("logback"), log.as_bytes());
    wait_for_index_within(&mut app, LARGE_INDEX_DEADLINE);
    assert_snapshot!(numbered(&mut app, 80, &[], 1..=3), @r#"
    "      1 ▶12:00:00.001 INFO  com.example.App          the first message [+999998]"
    "1000000  12:00:00.002 INFO  com.example.App          the second message, long en"
    "1000001  12:00:00.003 INFO  com.example.App          done"
    "#);
}

/// The gutter is as wide as the file's last line number needs, whatever lines are on
/// screen, so it widens only when the file reaches another power of ten.
#[test]
fn gutter_widens_when_the_file_reaches_10000_lines() {
    let mut app = app("x\n".repeat(9_999).as_bytes());
    assert_eq!(numbered(&mut app, 80, &[], 1..=1), r#""   1 ▶x""#);
    let grown = test_support::source("x\n".repeat(10_000).as_bytes());
    let mut app = changed(app, FileChange::Grew(grown));
    assert_eq!(numbered(&mut app, 80, &[], 1..=1), r#""    1 ▶x""#);
}

/// Before the index is complete the rows are the file's last entries, whose line
/// numbers aren't known yet.
#[test]
fn rows_read_from_the_end_of_the_file_have_blank_line_numbers() {
    let mut app = following_app(built_in("raw"), numbered_log(30).as_bytes());
    assert_snapshot!(numbered(&mut app, 80, &[], 21..=22), @r#"
    "      12:00:00.029 [main] INFO  com.example.App - step 29"
    "     ▶12:00:00.030 [main] INFO  com.example.App - step 30"
    "#);
}

/// Every number is dark gray, whatever the level of its row, and the timestamps
/// after them keep the default colour.
#[test]
fn line_numbers_are_dark_gray() {
    let terminal = numbered_terminal(&mut table(), 80, &[]);
    let buffer = terminal.backend().buffer();
    for y in 2..=6 {
        assert_eq!(buffer[(3, y)].fg, Color::DarkGray, "number on line {y}");
        assert_eq!(buffer[(6, y)].fg, Color::Reset, "timestamp on line {y}");
    }
}

// --- robustness ---

fn any_action() -> impl Strategy<Value = Action> {
    prop::sample::select(vec![
        Action::Up,
        Action::Down,
        Action::PageUp,
        Action::PageDown,
        Action::HalfPageUp,
        Action::HalfPageDown,
        Action::Top,
        Action::Bottom,
        Action::ScrollLeft,
        Action::ScrollRight,
        Action::ToggleThread,
        Action::ToggleLineNumbers,
        Action::ToggleWrap,
        Action::CyclePreset,
        Action::ToggleFollow,
    ])
}

/// The built-in formats and the ones without some columns.
fn any_preset() -> impl Strategy<Value = &'static Format> {
    let mut formats: Vec<_> = BUILT_IN.iter().collect();
    formats.extend([no_thread_or_logger(), no_timestamp()]);
    prop::sample::select(formats)
}

/// Lines that start an entry in some preset mixed with arbitrary bytes.
fn any_log() -> impl Strategy<Value = Vec<u8>> {
    let line = prop_oneof![
        prop::sample::select(EXAMPLES.map(|(_, line)| line.as_bytes().to_vec()).to_vec()),
        prop::collection::vec(any::<u8>(), 0..100),
    ];
    prop::collection::vec(line, 0..6).prop_map(|lines| lines.join(&b'\n'))
}

/// `app` drawn at `width`×`height` in `theme`, then again after each of `actions`.
fn drawn_after(
    app: &mut App,
    width: u16,
    height: u16,
    theme: Theme,
    actions: &[Action],
) -> Terminal<TestBackend> {
    let mut terminal = drawn(app, width, height, theme);
    for &action in actions {
        app.apply(action);
        draw(&mut terminal, app, theme);
    }
    terminal
}

proptest! {
    // A case whose file never finishes indexing fails only at the deadline, and so
    // would every step of shrinking it. Shrinking stops after two seconds instead.
    #![proptest_config(ProptestConfig {
        max_shrink_time: 2_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn any_file_in_any_preset_at_any_size_draws_only_printable_cells(
        preset in any_preset(),
        bytes in any_log(),
        width in 0u16..140,
        height in 0u16..40,
        actions in prop::collection::vec(any_action(), 0..12),
    ) {
        let mut app = app_in(preset, &bytes);
        let terminal = drawn_after(&mut app, width, height, Theme::Styled, &actions);
        for cell in &terminal.backend().buffer().content {
            prop_assert!(!cell.symbol().chars().any(char::is_control), "{:?}", cell.symbol());
        }
    }

    #[test]
    fn plain_theme_never_styles_a_cell(
        preset in any_preset(),
        bytes in any_log(),
        width in 0u16..140,
        height in 0u16..40,
        actions in prop::collection::vec(any_action(), 0..12),
    ) {
        let mut app = app_in(preset, &bytes);
        let terminal = drawn_after(&mut app, width, height, Theme::Plain, &actions);
        for cell in &terminal.backend().buffer().content {
            prop_assert_eq!(cell.style(), Cell::EMPTY.style());
        }
    }
}

// --- search ---

/// The search prompt with the keys that open it, type `pattern` and toggle regex.
fn regex_search_keys(opener: char, pattern: &str) -> Vec<KeyEvent> {
    let mut keys = vec![char_key(opener), ctrl('r')];
    keys.extend(pattern.chars().map(char_key));
    keys
}

/// The 80×24 terminal after searching `app` for the regex `pattern` with `/` and
/// counting its matches, then pressing `keys`; drawn after each key.
fn searched_terminal(app: &mut App, pattern: &str, keys: &[KeyEvent]) -> Terminal<TestBackend> {
    let mut search = regex_search_keys('/', pattern);
    search.push(key(KeyCode::Enter));
    let mut terminal = terminal_after_pressing(app, &search);
    wait_for_search(app);
    draw(&mut terminal, app, Theme::Styled);
    for &key in keys {
        app.key(key);
        draw(&mut terminal, app, Theme::Styled);
    }
    terminal
}

/// Screen lines `lines` as [`row`] shows them, each followed by a line with `^`
/// under its highlighted cells, those on a yellow background, if it has any.
fn with_highlights(terminal: &Terminal<TestBackend>, lines: RangeInclusive<u16>) -> String {
    let buffer = terminal.backend().buffer();
    let mut out = Vec::new();
    for y in lines {
        out.push(row(terminal, y));
        let marks: String = (0..buffer.area.width)
            .map(|x| {
                if buffer[(x, y)].bg == Color::Yellow {
                    '^'
                } else {
                    ' '
                }
            })
            .collect();
        if marks.contains('^') {
            out.push(marks.trim_end().to_string());
        }
    }
    out.join("\n")
}

/// Four lines, three of them about orders, the third about two.
const ORDERS: &str = concat!(
    "order 1 created\n",
    "payment for order 1 failed\n",
    "order 2 created, order 1 retried\n",
    "shipping\n",
);

#[test]
fn search_prompt_shows_an_invalid_regex_error_in_the_footer() {
    let mut keys = regex_search_keys('/', "(");
    keys.push(key(KeyCode::Enter));
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [main] INFO  com.example.App - step 1                             "
    " 12:00:00.002 [main] INFO  com.example.App - step 2                             "
    " 12:00:00.003 [main] INFO  com.example.App - step 3                             "
    " 12:00:00.004 [main] INFO  com.example.App - step 4                             "
    " 12:00:00.005 [main] INFO  com.example.App - step 5                             "
    " 12:00:00.006 [main] INFO  com.example.App - step 6                             "
    " 12:00:00.007 [main] INFO  com.example.App - step 7                             "
    " 12:00:00.008 [main] INFO  com.example.App - step 8                             "
    " 12:00:00.009 [main] INFO  com.example.App - step 9                             "
    " 12:00:00.┌ Search ──────────────────────────────────────────────────┐          "
    " 12:00:00.│ ▶(                                                       │          "
    " 12:00:00.│  [x] regex  [ ] ignore case                              │          "
    " 12:00:00.└──────────────────────────────────────────────────────────┘          "
    " 12:00:00.014 [main] INFO  com.example.App - step 14                            "
    " 12:00:00.015 [main] INFO  com.example.App - step 15                            "
    " 12:00:00.016 [main] INFO  com.example.App - step 16                            "
    " 12:00:00.017 [main] INFO  com.example.App - step 17                            "
    " 12:00:00.018 [main] INFO  com.example.App - step 18                            "
    " 12:00:00.019 [main] INFO  com.example.App - step 19                            "
    " 12:00:00.020 [main] INFO  com.example.App - step 20                            "
    " 12:00:00.021 [main] INFO  com.example.App - step 21                            "
    " 12:00:00.022 [main] INFO  com.example.App - step 22                            "
    "invalid regex: unclosed group                                                   "
    "#);
}

#[test]
fn search_prompt_footer_shows_its_keys() {
    let terminal = terminal_after_pressing(&mut hundred_lines(), &[char_key('/')]);
    assert_eq!(
        row(&terminal, 23),
        "Ctrl-r Regex  Tab Ignore case  Enter Search  Esc Cancel"
    );
}

#[test]
fn backward_search_prompt_is_titled_search_backward() {
    let terminal = terminal_after_pressing(&mut hundred_lines(), &[char_key('?')]);
    assert!(
        row(&terminal, 10).contains("┌ Search backward ─"),
        "{}",
        row(&terminal, 10)
    );
}

#[test]
fn matches_are_highlighted_in_the_visible_rows() {
    let terminal = searched_terminal(&mut app(ORDERS.as_bytes()), "order", &[]);
    assert_snapshot!(with_highlights(&terminal, 1..=4), @r"
     order 1 created
     ^^^^^
    ▶payment for order 1 failed
                 ^^^^^
     order 2 created, order 1 retried
     ^^^^^            ^^^^^
     shipping
    ");
}

#[test]
fn a_match_in_folded_lines_highlights_the_fold_marker() {
    let terminal = searched_terminal(&mut traces(), r"App\.java:2", &[]);
    assert_snapshot!(with_highlights(&terminal, 1..=4), @r"
     12:00:00.001 INFO  com.example.App          step 1
    ▶12:00:00.002 INFO  com.example.App          step 2 [+2]
                                                        ^^^^
     12:00:00.003 INFO  com.example.App          step 3 [+1]
     12:00:00.004 INFO  com.example.App          step 4
    ");
}

#[test]
fn matches_in_unfolded_stack_trace_lines_are_highlighted() {
    let terminal = searched_terminal(&mut traces(), r"App\.java:2", &[char_key(' ')]);
    assert_snapshot!(with_highlights(&terminal, 1..=6), @r"
     12:00:00.001 INFO  com.example.App          step 1
    ▶12:00:00.002 INFO  com.example.App          step 2
                                                     at App.step(App.java:1)
                                                     at App.step(App.java:2)
                                                                 ^^^^^^^^^^
     12:00:00.003 INFO  com.example.App          step 3 [+1]
     12:00:00.004 INFO  com.example.App          step 4
    ");
}

#[test]
fn header_shows_the_match_count_before_the_notice() {
    let keys = [char_key('n'), char_key('n')];
    let terminal = searched_terminal(&mut app(ORDERS.as_bytes()), "order", &keys);
    assert_eq!(
        row(&terminal, 0),
        "vilog ─ app.log ─ 85 B ─ Raw ─ PAUSED ● ─ match 1/3 ─ search wrapped"
    );
}

/// Three Logback entries: `App - step` runs from the class column into the message,
/// a tab comes before `done`, and the last message wraps inside `done`.
const COLUMNS_LOG: &str = concat!(
    "2026-10-07 12:00:00.001 [main] INFO  com.example.App - step 1\n",
    "2026-10-07 12:00:00.002 [main] INFO  com.example.App - \tstep 2 done\n",
    "2026-10-07 12:00:00.003 [main] INFO  com.example.App - aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa done\n",
);

/// Only the message column is highlighted: of `App - step`, the `step` in it. A tab
/// before a match moves its highlight by the columns the tab takes, and a wrapped
/// match is highlighted on both screen lines.
#[test]
fn highlights_stay_in_the_message_column_and_follow_tabs_and_wrapping() {
    let mut app = app_in(built_in("logback"), COLUMNS_LOG.as_bytes());
    let terminal = searched_terminal(&mut app, "App - step|done", &[char_key('w')]);
    assert_snapshot!(with_highlights(&terminal, 1..=4), @r"
     12:00:00.001 INFO  com.example.App          step 1
                                                 ^^^^
    ▶12:00:00.002 INFO  com.example.App              step 2 done
                                                            ^^^^
     12:00:00.003 INFO  com.example.App          aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa d
                                                                                   ^
                                                 one
                                                 ^^^
    ");
}

/// A match past the 64 KiB a line is shown up to is never on screen, even scrolled
/// as far right as the line goes.
#[test]
fn a_match_past_the_shown_part_of_a_huge_line_is_not_highlighted() {
    let huge = format!("{} needle\nshort\n", "a".repeat(100_000));
    let mut app = app(huge.as_bytes());
    let mut terminal = searched_terminal(&mut app, "needle", &[]);
    for _ in 0..10_000 {
        app.apply(Action::ScrollRight);
    }
    draw(&mut terminal, &mut app, Theme::Styled);
    assert!(row(&terminal, 1).ends_with('…'), "{}", row(&terminal, 1));
    assert_eq!(with_highlights(&terminal, 1..=1), row(&terminal, 1));
}

/// Text between matches keeps its tab stops: the tab after `ab` takes two columns.
#[test]
fn a_tab_after_a_match_keeps_its_tab_stop() {
    let terminal = searched_terminal(&mut app(b"ab\tdone\n"), "b|done", &[]);
    assert_snapshot!(with_highlights(&terminal, 1..=1), @r"
    ▶ab  done
      ^  ^^^^
    ");
}

/// A regex over bytes can match half of a character, which no cell shows alone.
#[test]
fn a_match_of_half_a_character_is_not_highlighted() {
    let terminal = searched_terminal(&mut app("café\n".as_bytes()), r"(?-u)\xC3", &[]);
    assert_eq!(with_highlights(&terminal, 1..=1), "▶café");
}

// --- details pane ---

/// A Logback entry whose stack trace takes 12 lines, more than the details pane shows.
fn long_trace() -> App {
    let trace: String = (1..=12)
        .map(|i| format!("\tat com.example.Step{i}.run(Step.java:{i})\n"))
        .collect();
    let log = format!(
        "2026-10-07 12:00:00.000 [main] ERROR com.example.App - Failed\n\
         java.lang.IllegalStateException: boom\n{trace}"
    );
    app_in(built_in("logback"), log.as_bytes())
}

/// The screen lines of the details pane at 80×24.
const DETAILS_LINES: RangeInclusive<u16> = 15..=22;

#[test]
fn details_pane_shows_the_selected_entry_with_its_stack_trace() {
    let keys = [
        char_key('j'),
        char_key('j'),
        char_key('j'),
        char_key('j'),
        key(KeyCode::Enter),
    ];
    assert_snapshot!(screen_after(&mut table(), &keys), @r#"
    "vilog ─ app.log ─ 500 B ─ Logback ─ PAUSED ●             F:1 E:1 W:1 I:1 D:1 T:1"
    " 12:00:00.100 TRACE com.example.App          Tick                               "
    " 12:00:00.200 DEBUG com.example.App          Config loaded                      "
    " 12:00:00.300 INFO  c.e.order.OrderService   Order 42 created                   "
    " 12:00:01.000 WARN  com.example.App          Retry 1/3                          "
    "▶12:00:02.000 ERROR com.example.App          Failed [+2]                        "
    " 12:00:03.000 FATAL com.example.App          Giving up                          "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "                                                                                "
    "┌ Details ─────────────────────────────────────────────────────────────────────┐"
    "│ com.example.App (main)                                                       │"
    "│ Failed                                                                       │"
    "│ java.lang.IllegalStateException: boom                                        │"
    "│     at com.example.App.main(App.java:10)                                     │"
    "│                                                                              │"
    "│                                                                              │"
    "└──────────────────────────────────────────────────────────────────────────────┘"
    "j/k Move  PgUp/PgDn Page  Ctrl-u/d Half page  g/G Top/Bottom  q Quit            "
    "#);
}

#[test]
fn details_pane_has_a_thick_border_while_focused() {
    let keys = [
        char_key('j'),
        char_key('j'),
        char_key('j'),
        char_key('j'),
        key(KeyCode::Enter),
        key(KeyCode::Tab),
    ];
    assert_snapshot!(lines_after(&mut table(), &keys, DETAILS_LINES), @r"
    ┏ Details ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
    ┃ com.example.App (main)                                                       ┃
    ┃ Failed                                                                       ┃
    ┃ java.lang.IllegalStateException: boom                                        ┃
    ┃     at com.example.App.main(App.java:10)                                     ┃
    ┃                                                                              ┃
    ┃                                                                              ┃
    ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
    ");
}

#[test]
fn details_pane_scrolls_through_an_entry_longer_than_itself() {
    let keys = [
        key(KeyCode::Enter),
        key(KeyCode::Tab),
        char_key('j'),
        char_key('j'),
        char_key('j'),
    ];
    assert_snapshot!(lines_after(&mut long_trace(), &keys, DETAILS_LINES), @r"
    ┏ Details ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
    ┃     at com.example.Step1.run(Step.java:1)                                    ┃
    ┃     at com.example.Step2.run(Step.java:2)                                    ┃
    ┃     at com.example.Step3.run(Step.java:3)                                    ┃
    ┃     at com.example.Step4.run(Step.java:4)                                    ┃
    ┃     at com.example.Step5.run(Step.java:5)                                    ┃
    ┃     at com.example.Step6.run(Step.java:6)                                    ┃
    ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
    ");
}

#[test]
fn details_pane_scrolls_no_further_than_its_last_line() {
    let page = key(KeyCode::PageDown);
    let keys = [key(KeyCode::Enter), key(KeyCode::Tab), page, page];
    assert_snapshot!(lines_after(&mut long_trace(), &keys, DETAILS_LINES), @r"
    ┏ Details ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
    ┃     at com.example.Step7.run(Step.java:7)                                    ┃
    ┃     at com.example.Step8.run(Step.java:8)                                    ┃
    ┃     at com.example.Step9.run(Step.java:9)                                    ┃
    ┃     at com.example.Step10.run(Step.java:10)                                  ┃
    ┃     at com.example.Step11.run(Step.java:11)                                  ┃
    ┃     at com.example.Step12.run(Step.java:12)                                  ┃
    ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
    ");
}

#[test]
fn details_pane_leaves_the_rows_above_it() {
    let keys = [key(KeyCode::Enter), char_key('G')];
    assert_snapshot!(lines_after(&mut hundred_lines(), &keys, 13..=15), @r"
     12:00:00.099 [main] INFO  com.example.App - step 99
    ▶12:00:00.100 [main] INFO  com.example.App - step 100
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    ");
}

#[test]
fn details_pane_shows_the_headless_entry_as_it_is() {
    let mut app = app_in(built_in("logback"), HEADLESS_FIRST_LOG.as_bytes());
    assert_snapshot!(lines_after(&mut app, &[key(KeyCode::Enter)], DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ starting up                                                                  │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn details_pane_shows_a_raw_entry_as_it_is() {
    assert_snapshot!(lines_after(&mut hundred_lines(), &[key(KeyCode::Enter)], DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ 12:00:00.001 [main] INFO  com.example.App - step 1                           │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn details_pane_wraps_a_long_message_at_its_inner_width() {
    let log = format!(
        "2026-10-07 12:00:00.000 [main] INFO  com.example.App - {}\n",
        "0123456789".repeat(10)
    );
    let mut app = app_in(built_in("logback"), log.as_bytes());
    assert_snapshot!(lines_after(&mut app, &[key(KeyCode::Enter)], DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ com.example.App (main)                                                       │
    │ 0123456789012345678901234567890123456789012345678901234567890123456789012345 │
    │ 678901234567890123456789                                                     │
    │                                                                              │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn details_pane_shows_the_lines_appended_to_its_entry() {
    let log = concat!(
        "2026-10-07 12:00:00.000 [main] ERROR com.example.App - Failed\n",
        "java.lang.IllegalStateException: boom\n",
        "\tat com.example.App.main(App.java:10)\n",
    );
    let mut app = following_app(built_in("logback"), log.as_bytes());
    wait_for_index(&mut app);
    let mut terminal = terminal_after(&mut app, &[key(KeyCode::Enter)]);
    let longer = format!("{log}\tat java.lang.Thread.run(Thread.java:1583)\n");
    let mut app = changed(
        app,
        FileChange::Grew(test_support::source(longer.as_bytes())),
    );
    draw(&mut terminal, &mut app, Theme::Styled);
    let lines: Vec<String> = DETAILS_LINES.map(|y| row(&terminal, y)).collect();
    assert_snapshot!(lines.join("\n"), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ com.example.App (main)                                                       │
    │ Failed                                                                       │
    │ java.lang.IllegalStateException: boom                                        │
    │     at com.example.App.main(App.java:10)                                     │
    │     at java.lang.Thread.run(Thread.java:1583)                                │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

/// The last line of the details pane, focused and scrolled to the bottom, on a
/// Logback entry of exactly `len` bytes whose stack trace ends in a line of `x`.
fn last_details_line(len: usize) -> String {
    let head = "2026-10-07 12:00:00.000 [main] ERROR com.example.App - Failed\n";
    let line = "\tat com.example.Step.run(Step.java:1)\n";
    let lines = (len - head.len()) / line.len() - 1;
    let rest = len - head.len() - lines * line.len();
    let log = format!(
        "{head}{}\tat {}\n",
        line.repeat(lines),
        "x".repeat(rest - 5)
    );
    assert_eq!(log.len(), len);
    let mut app = unindexed_app(built_in("logback"), log.as_bytes());
    wait_for_index_within(&mut app, LARGE_INDEX_DEADLINE);
    let mut terminal = terminal_after(&mut app, &[key(KeyCode::Enter), key(KeyCode::Tab)]);
    loop {
        let scroll = app.details_scroll();
        app.apply(Action::PageDown);
        if app.details_scroll() == scroll {
            break;
        }
    }
    draw(&mut terminal, &mut app, Theme::Styled);
    let last = row(&terminal, *DETAILS_LINES.end() - 1);
    last.trim_matches(['┃', ' ']).to_owned()
}

#[test]
fn details_pane_ends_an_entry_longer_than_its_cap_with_an_ellipsis() {
    assert_eq!(last_details_line(super::details::DETAILS_CAP + 1), "…");
}

#[test]
fn details_pane_shows_an_entry_as_long_as_its_cap_whole() {
    let last = last_details_line(super::details::DETAILS_CAP);
    assert!(last.ends_with('x'), "{last}");
}

// --- histogram ---

fn spike() -> App {
    app_in(built_in("logback"), spike_log().as_bytes())
}

/// The keys that show the histogram, focus it and select its third bucket.
fn third_bucket_keys() -> [KeyEvent; 4] {
    [
        char_key('H'),
        key(KeyCode::Tab),
        key(KeyCode::Right),
        key(KeyCode::Right),
    ]
}

#[test]
fn histogram_row_shows_a_spike_of_errors_under_the_header() {
    assert_snapshot!(lines_after(&mut spike(), &[char_key('H')], 1..=2), @r"
      ▁      ▂  █▅    ▁                                                     max 9/5m
    ▶10:00:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn histogram_without_warnings_or_errors_is_blank_but_for_its_scale() {
    let log = concat!(
        "2026-10-07 12:00:00.123 [main] INFO  com.example.App - Started\n",
        "2026-10-07 12:00:01.000 [main] INFO  com.example.App - Working\n",
        "2026-10-07 12:00:02.000 [main] INFO  com.example.App - Done\n",
    );
    let mut app = app_in(built_in("logback"), log.as_bytes());
    assert_snapshot!(lines_after(&mut app, &[char_key('H')], 1..=2), @r"
                                                                            max 0/1s
    ▶12:00:00.123 INFO  com.example.App          Started
    ");
}

#[test]
fn focused_histogram_names_the_count_and_start_of_the_selected_bucket() {
    assert_snapshot!(lines_after(&mut spike(), &third_bucket_keys(), 1..=2), @r"
      ▁      ▂  █▅    ▁                                                1 at 10:10:00
    ▶10:00:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn focused_histogram_shows_the_selected_bucket_in_reverse_video() {
    let terminal = terminal_after(&mut spike(), &third_bucket_keys());
    let buffer = terminal.backend().buffer();
    let reversed = |x: u16| buffer[(x, 1)].modifier.contains(Modifier::REVERSED);
    assert_eq!(
        [reversed(1), reversed(2), reversed(3)],
        [false, true, false]
    );
}

#[test]
fn histogram_without_timestamps_is_hidden_with_a_notice() {
    assert_snapshot!(lines_after(&mut hundred_lines(), &[char_key('H')], 0..=1), @r"
    vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ● ─ no entry has a timestamp
    ▶12:00:00.001 [main] INFO  com.example.App - step 1
    ");
}

#[test]
fn too_small_terminal_shows_no_histogram() {
    let mut app = spike();
    app.apply(Action::ToggleHistogram);
    let terminal = drawn(&mut app, 79, 24, Theme::Styled);
    assert_eq!(row(&terminal, 1), TOO_SMALL);
}

/// A Logback log of one entry per `(timestamp, level)`.
fn logback_log(entries: impl IntoIterator<Item = (&'static str, &'static str)>) -> String {
    entries
        .into_iter()
        .map(|(at, level)| format!("{at}.000 [main] {level:<5} com.example.App - Tick\n"))
        .collect()
}

/// A Logback viewer over one entry per `(timestamp, level)`.
fn logback(entries: impl IntoIterator<Item = (&'static str, &'static str)>) -> App {
    app_in(built_in("logback"), logback_log(entries).as_bytes())
}

/// Three days in 6 h buckets: 3 warnings on the 7th from 12:00, 6 on the 8th from
/// 06:00 and 1 on the 9th from 18:00.
fn three_days() -> App {
    let warns = |at, n| std::iter::repeat_n((at, "WARN"), n);
    let entries = [("2026-10-07 00:00:00", "INFO")]
        .into_iter()
        .chain(warns("2026-10-07 13:00:00", 3))
        .chain(warns("2026-10-08 07:00:00", 6))
        .chain(warns("2026-10-09 19:00:00", 1))
        .chain([("2026-10-09 23:00:00", "INFO")]);
    logback(entries)
}

/// `n` errors at `at` between entries at `first` and `last`.
fn spike_between(first: &'static str, at: &'static str, n: usize, last: &'static str) -> App {
    let errors = std::iter::repeat_n((at, "ERROR"), n);
    let entries = [(first, "INFO")]
        .into_iter()
        .chain(errors)
        .chain([(last, "INFO")]);
    // A six-digit spike is some 6 MB of log.
    let mut app = unindexed_app(built_in("logback"), logback_log(entries).as_bytes());
    wait_for_index_within(&mut app, LARGE_INDEX_DEADLINE);
    app
}

/// 12345 errors at 12:00 on the 7th, between entries that span 57 hours: at 80
/// columns, 1 h buckets would leave 57 columns for the 23 of `12345 at 10-07 12:00:00`,
/// no room for a gap between the bars and the label.
fn five_digit_spike() -> App {
    spike_between(
        "2026-10-07 00:00:00",
        "2026-10-07 12:00:00",
        12_345,
        "2026-10-09 08:00:00",
    )
}

#[test]
fn focused_histogram_over_several_days_names_the_date_of_the_selected_bucket() {
    assert_snapshot!(lines_after(&mut three_days(), &third_bucket_keys(), 1..=2), @r"
      ▄  █     ▂                                                 3 at 10-07 12:00:00
    ▶00:00:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn focused_histogram_across_midnight_names_the_date_of_the_selected_bucket() {
    let mut app = logback([
        ("2026-10-07 23:59:00", "INFO"),
        ("2026-10-07 23:59:12", "WARN"),
        ("2026-10-08 00:00:30", "ERROR"),
        ("2026-10-08 00:00:30", "ERROR"),
        ("2026-10-08 00:00:59", "INFO"),
    ]);
    assert_snapshot!(lines_after(&mut app, &third_bucket_keys(), 1..=2), @r"
      ▄               █                                          1 at 10-07 23:59:10
    ▶23:59:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn histogram_label_column_widens_to_fit_a_five_digit_count_with_a_date() {
    assert_snapshot!(lines_after(&mut five_digit_spike(), &third_bucket_keys(), 1..=2), @r"
      █                                                      12345 at 10-07 12:00:00
    ▶00:00:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn focused_histogram_ending_on_the_first_buckets_date_shows_only_the_time() {
    // 5 m buckets from 22:00 to 23:55; the bucket after the last starts the next day.
    let mut app = logback([
        ("2026-10-07 22:00:00", "INFO"),
        ("2026-10-07 22:10:00", "WARN"),
        ("2026-10-07 23:59:59", "INFO"),
    ]);
    assert_snapshot!(lines_after(&mut app, &third_bucket_keys(), 1..=2), @r"
      █                                                                1 at 22:10:00
    ▶22:00:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn histogram_label_column_widens_to_fit_a_six_digit_count_with_a_whole_date() {
    // Sixty days in 1 d buckets would leave 60 columns for the 20 of
    // `100000 at 2026-10-04`, no room for a gap, so the buckets grow to 2 d.
    let mut app = spike_between(
        "2026-10-01 00:00:00",
        "2026-10-05 12:00:00",
        100_000,
        "2026-11-29 00:00:00",
    );
    assert_snapshot!(lines_after(&mut app, &third_bucket_keys(), 1..=2), @r"
      █                                                         100000 at 2026-10-04
    ▶00:00:00.000 INFO  com.example.App          Tick
    ");
}

#[test]
fn histogram_label_column_is_as_wide_unfocused_as_focused() {
    assert_snapshot!(lines_after(&mut five_digit_spike(), &[char_key('H')], 1..=2), @r"
      █                                                                 max 12345/6h
    ▶00:00:00.000 INFO  com.example.App          Tick
    ");
}

/// The span of `five_digit_spike` without its errors, which arrive after a draw.
#[test]
fn histogram_label_column_widens_when_new_entries_need_it() {
    let log = logback_log([
        ("2026-10-07 00:00:00", "INFO"),
        ("2026-10-09 08:00:00", "INFO"),
    ]);
    let mut app = app_in(built_in("logback"), log.as_bytes());
    let mut terminal = terminal_after(&mut app, &[char_key('H')]);
    let errors = std::iter::repeat_n(("2026-10-07 12:00:00", "ERROR"), 12_345);
    let grown = format!("{log}{}", logback_log(errors));
    let mut app = changed(
        app,
        FileChange::Grew(test_support::source(grown.as_bytes())),
    );
    draw(&mut terminal, &mut app, Theme::Styled);
    assert_snapshot!(row(&terminal, 1), @"  █                                                                 max 12345/6h");
}

// --- hit map ---

/// The map of an 80×24 screen whose rows are drawn from line `top` on, `height`
/// lines high.
fn rows_map(top: u16, height: u16, lines: impl IntoIterator<Item = usize>) -> HitMap {
    HitMap {
        rows: Rect::new(0, top, 80, height),
        lines: lines.into_iter().collect(),
        buckets: None,
        details: None,
    }
}

#[test]
fn hit_map_has_the_row_drawn_on_each_line() {
    let mut app = hundred_lines();
    app.apply(Action::Bottom);
    drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(*app.hit_map(), rows_map(1, 22, 78..100));
}

#[test]
fn hit_map_lines_end_with_the_last_row() {
    let mut app = app(numbered_log(3).as_bytes());
    drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(*app.hit_map(), rows_map(1, 22, 0..3));
}

#[test]
fn hit_map_has_a_wrapped_row_on_each_of_its_lines() {
    let mut app = two_line_entries();
    terminal_after(&mut app, &[char_key('w')]);
    let lines: Vec<usize> = (0..11).flat_map(|i| [i, i]).collect();
    assert_eq!(app.hit_map().lines, lines);
}

#[test]
fn hit_map_has_the_histogram_buckets_and_the_rows_below_them() {
    let mut app = spike();
    terminal_after(&mut app, &[char_key('H')]);
    assert_eq!(
        *app.hit_map(),
        HitMap {
            buckets: Some(Rect::new(0, 1, 24, 1)),
            ..rows_map(2, 21, 0..21)
        }
    );
}

#[test]
fn hit_map_has_the_details_pane_below_the_rows() {
    let mut app = hundred_lines();
    terminal_after(&mut app, &[key(KeyCode::Enter)]);
    assert_eq!(
        *app.hit_map(),
        HitMap {
            details: Some(Rect::new(0, 15, 80, 8)),
            ..rows_map(1, 14, 0..14)
        }
    );
}

#[test]
fn hit_map_of_a_too_small_terminal_has_no_rows() {
    let mut app = hundred_lines();
    drawn(&mut app, 79, 24, Theme::Styled);
    assert_eq!(app.hit_map().lines, Vec::<usize>::new());
}

// --- help overlay ---

#[test]
fn help_overlay_lists_the_keymap_over_the_rows() {
    let keys = [key(KeyCode::F(1))];
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [ma┌ Help ──────────────────────────────────────┐                 "
    " 12:00:00.002 [ma│ k ↑           Move up                      │                 "
    " 12:00:00.003 [ma│ j ↓           Move down                    │                 "
    " 12:00:00.004 [ma│ PgUp          Page up                      │                 "
    " 12:00:00.005 [ma│ PgDn          Page down                    │                 "
    " 12:00:00.006 [ma│ Ctrl-u        Half page up                 │                 "
    " 12:00:00.007 [ma│ Ctrl-d        Half page down               │                 "
    " 12:00:00.008 [ma│ g Home        Go to top                    │                 "
    " 12:00:00.009 [ma│ G End         Go to bottom and follow      │                 "
    " 12:00:00.010 [ma│ ←             Scroll left                  │                 "
    " 12:00:00.011 [ma│ →             Scroll right                 │                 "
    " 12:00:00.012 [ma│ Tab           Focus next pane              │                 "
    " 12:00:00.013 [ma│ Enter         Details panel                │                 "
    " 12:00:00.014 [ma│ Esc           Cancel waiting export        │                 "
    " 12:00:00.015 [ma│ F             Toggle follow                │                 "
    " 12:00:00.016 [ma│ P             Cycle format preset          │                 "
    " 12:00:00.017 [ma│ /             Search forward               │                 "
    " 12:00:00.018 [ma│ ?             Search backward              │                 "
    " 12:00:00.019 [ma│ n             Next match                   │                 "
    " 12:00:00.020 [ma│ N             Previous match               │                 "
    " 12:00:00.021 [ma│ :             Jump to line                 │                 "
    " 12:00:00.022 [ma└────────────────────────────────────────────┘                 "
    "j/k Scroll  PgUp/PgDn Page  F1/Esc/q Close                                      "
    "#);
}

#[test]
fn help_overlay_scrolls_a_page_with_page_down() {
    let keys = [key(KeyCode::F(1)), key(KeyCode::PageDown)];
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [ma┌ Help ──────────────────────────────────────┐                 "
    " 12:00:00.002 [ma│ t             Jump to time                 │                 "
    " 12:00:00.003 [ma│ 1 2 3 4 5 6   Toggle level (TRACE to FATAL)│                 "
    " 12:00:00.004 [ma│ F2            Level filter                 │                 "
    " 12:00:00.005 [ma│ F4 &          Text filter                  │                 "
    " 12:00:00.006 [ma│ F5            Time filter                  │                 "
    " 12:00:00.007 [ma│ F6            Logger filter                │                 "
    " 12:00:00.008 [ma│ c             Only this logger             │                 "
    " 12:00:00.009 [ma│ C             Hide this logger             │                 "
    " 12:00:00.010 [ma│ x             Clear all filters            │                 "
    " 12:00:00.011 [ma│ Space         Fold or unfold entry         │                 "
    " 12:00:00.012 [ma│ z             Fold all                     │                 "
    " 12:00:00.013 [ma│ Z             Unfold all                   │                 "
    " 12:00:00.014 [ma│ ]             Next exception               │                 "
    " 12:00:00.015 [ma│ [             Previous exception           │                 "
    " 12:00:00.016 [ma│ m             Toggle bookmark              │                 "
    " 12:00:00.017 [ma│ '             Next bookmark                │                 "
    " 12:00:00.018 [ma│ "             Previous bookmark            │                 "
    " 12:00:00.019 [ma│ y             Copy entry                   │                 "
    " 12:00:00.020 [ma│ e F8          Export filtered              │                 "
    " 12:00:00.021 [ma│ T             Toggle thread column         │                 "
    " 12:00:00.022 [ma└────────────────────────────────────────────┘                 "
    "j/k Scroll  PgUp/PgDn Page  F1/Esc/q Close                                      "
    "#);
}

#[test]
fn help_overlay_scrolled_to_its_end_shows_its_last_lines() {
    let keys = [
        key(KeyCode::F(1)),
        key(KeyCode::PageDown),
        key(KeyCode::PageDown),
        key(KeyCode::PageDown),
    ];
    assert_snapshot!(screen_after_pressing(&mut hundred_lines(), &keys), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [ma┌ Help ──────────────────────────────────────┐                 "
    " 12:00:00.002 [ma│ F6            Logger filter                │                 "
    " 12:00:00.003 [ma│ c             Only this logger             │                 "
    " 12:00:00.004 [ma│ C             Hide this logger             │                 "
    " 12:00:00.005 [ma│ x             Clear all filters            │                 "
    " 12:00:00.006 [ma│ Space         Fold or unfold entry         │                 "
    " 12:00:00.007 [ma│ z             Fold all                     │                 "
    " 12:00:00.008 [ma│ Z             Unfold all                   │                 "
    " 12:00:00.009 [ma│ ]             Next exception               │                 "
    " 12:00:00.010 [ma│ [             Previous exception           │                 "
    " 12:00:00.011 [ma│ m             Toggle bookmark              │                 "
    " 12:00:00.012 [ma│ '             Next bookmark                │                 "
    " 12:00:00.013 [ma│ "             Previous bookmark            │                 "
    " 12:00:00.014 [ma│ y             Copy entry                   │                 "
    " 12:00:00.015 [ma│ e F8          Export filtered              │                 "
    " 12:00:00.016 [ma│ T             Toggle thread column         │                 "
    " 12:00:00.017 [ma│ #             Toggle line numbers          │                 "
    " 12:00:00.018 [ma│ H             Toggle histogram             │                 "
    " 12:00:00.019 [ma│ w             Toggle wrap                  │                 "
    " 12:00:00.020 [ma│ F1            Help                         │                 "
    " 12:00:00.021 [ma│ q F10 Ctrl-c  Quit                         │                 "
    " 12:00:00.022 [ma└────────────────────────────────────────────┘                 "
    "j/k Scroll  PgUp/PgDn Page  F1/Esc/q Close                                      "
    "#);
}

#[test]
fn help_overlay_in_a_tall_terminal_is_as_high_as_its_lines_and_border() {
    let mut app = hundred_lines();
    let mut terminal = drawn(&mut app, 80, 60, Theme::Styled);
    app.key(key(KeyCode::F(1)));
    draw(&mut terminal, &mut app, Theme::Styled);
    let find = |mark: &str| (0..60).find(|&y| row(&terminal, y).contains(mark)).unwrap();
    let (top, bottom) = (find("┌ Help "), find("└"));
    assert_eq!(
        usize::from(bottom - top) + 1,
        crate::keymap::Keymap::default().help().len() + 2
    );
    assert!(row(&terminal, top + 1).contains("│ k ↑ "));
}

/// The overlay opens with the key `help` is remapped to and lists the remapped keys
/// in place of the defaults; the footer names the first key of each action. `q`
/// closes the overlay whatever `quit` is bound to.
#[test]
fn help_overlay_lists_remapped_keys() {
    let mut app = hundred_lines();
    let config = "[keys]\nup = \"i\"\ndown = [\"o\", \"Down\"]\nhelp = \"h\"\n";
    app.configure(&Config::parse(config).unwrap());
    assert_snapshot!(screen_after_pressing(&mut app, &[char_key('h')]), @r#"
    "vilog ─ app.log ─ 5.1 KB ─ Raw ─ PAUSED ●                                       "
    "▶12:00:00.001 [ma┌ Help ──────────────────────────────────────┐                 "
    " 12:00:00.002 [ma│ i             Move up                      │                 "
    " 12:00:00.003 [ma│ o ↓           Move down                    │                 "
    " 12:00:00.004 [ma│ PgUp          Page up                      │                 "
    " 12:00:00.005 [ma│ PgDn          Page down                    │                 "
    " 12:00:00.006 [ma│ Ctrl-u        Half page up                 │                 "
    " 12:00:00.007 [ma│ Ctrl-d        Half page down               │                 "
    " 12:00:00.008 [ma│ g Home        Go to top                    │                 "
    " 12:00:00.009 [ma│ G End         Go to bottom and follow      │                 "
    " 12:00:00.010 [ma│ ←             Scroll left                  │                 "
    " 12:00:00.011 [ma│ →             Scroll right                 │                 "
    " 12:00:00.012 [ma│ Tab           Focus next pane              │                 "
    " 12:00:00.013 [ma│ Enter         Details panel                │                 "
    " 12:00:00.014 [ma│ Esc           Cancel waiting export        │                 "
    " 12:00:00.015 [ma│ F             Toggle follow                │                 "
    " 12:00:00.016 [ma│ P             Cycle format preset          │                 "
    " 12:00:00.017 [ma│ /             Search forward               │                 "
    " 12:00:00.018 [ma│ ?             Search backward              │                 "
    " 12:00:00.019 [ma│ n             Next match                   │                 "
    " 12:00:00.020 [ma│ N             Previous match               │                 "
    " 12:00:00.021 [ma│ :             Jump to line                 │                 "
    " 12:00:00.022 [ma└────────────────────────────────────────────┘                 "
    "o/i Scroll  PgUp/PgDn Page  h/Esc/q Close                                       "
    "#);
}

/// The footer names the first key of each action it lists; a pair is shortened to
/// `Ctrl-u/d` only when both keys are Ctrl keys.
#[test]
fn key_hints_show_remapped_keys() {
    let mut app = hundred_lines();
    let config = "[keys]\nhalf_page_up = \"u\"\nquit = [\"Q\", \"F10\"]\n";
    app.configure(&Config::parse(config).unwrap());
    let terminal = drawn(&mut app, 80, 24, Theme::Styled);
    assert_eq!(
        row(&terminal, 23),
        "j/k Move  PgUp/PgDn Page  u/Ctrl-d Half page  g/G Top/Bottom  Q Quit"
    );
}

// --- JSON lines ---

/// The rows of the JSON lines fixture `name` with its fifth entry selected, and that
/// entry in the details pane, on a screen tall enough to show all of it.
fn fifth_json_entry_in_details(name: &str) -> String {
    let mut app = app_in(built_in("json"), &test_support::json_fixture(name));
    let mut terminal = drawn(&mut app, 80, 42, Theme::Plain);
    let keys = [char_key('j'); 4].into_iter().chain([key(KeyCode::Enter)]);
    for key in keys {
        app.apply(action(key).unwrap());
        draw(&mut terminal, &mut app, Theme::Plain);
    }
    let lines: Vec<String> = (1..=8).chain(27..=40).map(|y| row(&terminal, y)).collect();
    lines.join("\n")
}

/// pino writes levels and times as numbers, and an error's stack one object deep in
/// `err.stack`: the rest of `err` follows the other fields as dotted keys.
#[test]
fn pino_entry_shows_its_columns_in_the_table_and_its_stack_and_fields_in_details() {
    assert_snapshot!(fifth_json_entry_in_details("pino.log"), @r#"
     10:30:00.000 INFO  api                      server listening at http://0.0.0.0:
     10:30:00.412 DEBUG api                      route table built
     10:30:01.087 INFO  api                      request completed
     10:30:02.530 WARN  api                      deprecated header "X-Legacy" used
    ▶10:30:03.114 ERROR api                      Cannot read properties of undefined
     10:30:03.900 TRACE api                      cache probe
     10:30:05.001 FATAL api                      out of memory, exiting
     10:30:06.250 INFO  api                      café menu loaded
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ api                                                                          │
    │ Cannot read properties of undefined (reading 'id')                           │
    │ TypeError: Cannot read properties of undefined (reading 'id')                │
    │     at getUser (/app/src/users.js:42:19)                                     │
    │     at Layer.handle (/app/node_modules/express/lib/router/layer.js:95:5)     │
    │     at next (/app/node_modules/express/lib/router/route.js:149:13)           │
    │ level: 50                                                                    │
    │ time: 1773484203114                                                          │
    │ pid: 4121                                                                    │
    │ hostname: web-1                                                              │
    │ err.type: TypeError                                                          │
    │ err.message: Cannot read properties of undefined (reading 'id')              │
    └──────────────────────────────────────────────────────────────────────────────┘
    "#);
}

/// The stack trace ends with a line break, which makes no empty line.
#[test]
fn logstash_entry_shows_its_columns_in_the_table_and_its_stack_and_fields_in_details() {
    assert_snapshot!(fifth_json_entry_in_details("logstash-logback.log"), @r#"
     08:12:01.104 INFO  c.a.o.OrderApplication   Started OrderApplication in 3.2 sec
     08:12:01.310 DEBUG c.z.h.HikariDataSource   HikariPool-1 - Start completed.
     08:12:02.020 INFO  c.a.orders.OrderService  Order 42 created
     08:12:03.481 WARN  c.acme.orders.HttpClient Slow upstream, retry 2
    ▶08:12:04.775 ERROR c.a.o.PaymentGateway     Payment failed for "Alice"
     08:12:05.002 INFO  c.a.orders.OrderService  Zamówienie 43 przyjęte
     08:12:06.450 TRACE com.acme.orders.Cache    Cache probe
     08:12:07.900 INFO  c.a.o.OrderApplication   Shutting down
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ com.acme.orders.PaymentGateway (http-nio-8080-exec-1)                        │
    │ Payment failed for "Alice"                                                   │
    │ java.lang.IllegalStateException: Card declined                               │
    │     at com.acme.orders.PaymentGateway.charge(PaymentGateway.java:31)         │
    │     at com.acme.orders.OrderService.create(OrderService.java:97)             │
    │     at java.base/java.lang.Thread.run(Thread.java:840)                       │
    │ @timestamp: 2026-03-14T09:12:04.775+01:00                                    │
    │ @version: 1                                                                  │
    │ level: ERROR                                                                 │
    │ level_value: 40000                                                           │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    "#);
}

/// A JSON log of one INFO entry of the logger `api` per message in `messages`, each
/// written into the line as it is, a second apart from 09:12:01 UTC.
fn json_log(messages: &[&str]) -> App {
    let log: String = messages
        .iter()
        .zip(1..)
        .map(|(message, second)| {
            format!(
                r#"{{"time":"2026-03-14T09:12:{second:02}Z","level":"INFO","name":"api","msg":{message}}}"#
            ) + "\n"
        })
        .collect();
    app_in(built_in("json"), log.as_bytes())
}

/// A message with escapes and a line break, as JSON writes it.
const TWO_LINE_MESSAGE: &str = r#""café \"quoted\"\nsecond line""#;

#[test]
fn json_row_shows_the_first_line_of_the_decoded_message() {
    let mut app = json_log(&[TWO_LINE_MESSAGE]);
    assert_eq!(
        lines_after(&mut app, &[], 1..=1),
        r#"▶09:12:01.000 INFO  api                      café "quoted""#
    );
}

/// The logger is already above the message, so it isn't among the fields.
#[test]
fn json_details_show_every_line_of_the_decoded_message_then_the_other_fields() {
    let mut app = json_log(&[TWO_LINE_MESSAGE]);
    assert_snapshot!(lines_after(&mut app, &[key(KeyCode::Enter)], DETAILS_LINES), @r#"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ api                                                                          │
    │ café "quoted"                                                                │
    │ second line                                                                  │
    │ time: 2026-03-14T09:12:01Z                                                   │
    │ level: INFO                                                                  │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    "#);
}

/// The message keeps its line when empty, as in a text format, so the fields don't
/// read as the message.
#[test]
fn json_details_show_an_empty_message_as_an_empty_line() {
    let mut app = json_log(&[r#""""#]);
    assert_snapshot!(lines_after(&mut app, &[key(KeyCode::Enter)], DETAILS_LINES), @r"
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ api                                                                          │
    │                                                                              │
    │ time: 2026-03-14T09:12:01Z                                                   │
    │ level: INFO                                                                  │
    │                                                                              │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    ");
}

#[test]
fn json_message_that_is_not_a_string_is_shown_as_written() {
    let mut app = json_log(&["42", r#"{"code":7,"text":"a\"b"}"#]);
    assert_snapshot!(lines_after(&mut app, &[], 1..=2), @r#"
    ▶09:12:01.000 INFO  api                      42
     09:12:02.000 INFO  api                      {"code":7,"text":"a\"b"}
    "#);
}

/// PRD §9.2: `"quoted"` finds `\"quoted\"` in the file.
#[test]
fn search_in_a_json_log_matches_and_highlights_the_decoded_text() {
    let mut app = json_log(&[r#""user quoted twice""#, r#""user said \"quoted\" here""#]);
    let terminal = searched_terminal(&mut app, r#""quoted""#, &[]);
    assert_snapshot!(with_highlights(&terminal, 1..=2), @r#"
     09:12:01.000 INFO  api                      user quoted twice
    ▶09:12:02.000 INFO  api                      user said "quoted" here
                                                           ^^^^^^^^
    "#);
}

// --- logfmt ---

/// Screen lines `lines` of the 80×24 screen of the logfmt fixture `name`, read as
/// logfmt, after pressing `keys`.
fn logfmt_screen(name: &str, keys: &[KeyEvent], lines: impl IntoIterator<Item = u16>) -> String {
    let mut app = app_in(built_in("logfmt"), &test_support::logfmt_fixture(name));
    let terminal = terminal_after(&mut app, keys);
    let rows: Vec<String> = lines.into_iter().map(|y| row(&terminal, y)).collect();
    rows.join("\n")
}

/// logrus quotes its time and its message, and Go's stack trace after the panic is
/// folded into the last entry. The file has no logger key, so that column is empty.
#[test]
fn logrus_log_opens_with_timestamp_level_and_message_columns() {
    assert_snapshot!(logfmt_screen("logrus.log", &[], 0..=8), @r#"
    vilog ─ app.log ─ 1.3 KB ─ logfmt ─ PAUSED ●             F:1 E:1 W:1 I:3 D:1 T:1
    ▶08:12:01.000 INFO                           Starting order service
     08:12:02.000 DEBUG                          Connecting to database
     08:12:03.000 INFO                           Order 42 created
     08:12:04.000 WARN                           Deprecated header "X-Legacy" used
     08:12:05.000 ERROR                          Payment failed
     08:12:06.000 INFO                           Zamówienie 43 przyjęte
     08:12:07.000 TRACE                          Cache probe
     08:12:08.000 FATAL                          Lost connection to database [+11]
    "#);
}

/// slog leaves the time and one-word values bare. An empty message stays empty, and
/// the last line, cut off inside its quoted message, shows what it has. Details show
/// the decoded message, then every other field in file order.
#[test]
fn slog_entry_shows_its_columns_in_the_table_and_its_fields_in_details() {
    let mut keys = vec![char_key('j'); 5];
    keys.push(key(KeyCode::Enter));
    let lines = (1..=9).chain(DETAILS_LINES);
    assert_snapshot!(logfmt_screen("slog.log", &keys, lines), @r#"
     08:12:01.104 INFO                           started
     08:12:01.310 DEBUG                          loading config
     08:12:02.020 INFO                           order created
     08:12:02.512 INFO                           listing orders
     08:12:03.481 WARN                           slow upstream
    ▶08:12:04.775 ERROR                          payment failed
     08:12:05.002 INFO                           Zamówienie 43 przyjęte
     08:12:06.250 INFO
     08:12:07.900 INFO                           shutting dow
    ┌ Details ─────────────────────────────────────────────────────────────────────┐
    │ payment failed                                                               │
    │ time: 2026-03-14T09:12:04.775+01:00                                          │
    │ level: ERROR                                                                 │
    │ err: card declined: "insufficient funds"                                     │
    │ order_id: 42                                                                 │
    │                                                                              │
    └──────────────────────────────────────────────────────────────────────────────┘
    "#);
}

/// A logfmt log of one INFO entry per message in `messages`, each written into the
/// line after `msg=` as it is, a second apart from 09:12:01 UTC.
fn logfmt_log(messages: &[&str]) -> App {
    let log: String = messages
        .iter()
        .zip(1..)
        .map(|(message, second)| {
            format!("time=2026-03-14T09:12:{second:02}Z level=INFO msg={message}\n")
        })
        .collect();
    app_in(built_in("logfmt"), log.as_bytes())
}

/// PRD §9.2: `"quoted"` finds `\"quoted\"` in the file.
#[test]
fn search_in_a_logfmt_log_matches_and_highlights_the_decoded_text() {
    let mut app = logfmt_log(&[r#""user quoted twice""#, r#""user said \"quoted\" here""#]);
    let terminal = searched_terminal(&mut app, r#""quoted""#, &[]);
    assert_snapshot!(with_highlights(&terminal, 1..=2), @r#"
     09:12:01.000 INFO                           user quoted twice
    ▶09:12:02.000 INFO                           user said "quoted" here
                                                           ^^^^^^^^
    "#);
}

/// Only quoted values have escapes: slog leaves a Windows path bare, and its
/// backslashes are shown and searched as written.
#[test]
fn search_in_a_logfmt_log_reads_bare_values_as_written() {
    let mut app = logfmt_log(&["started", r"C:\temp\new"]);
    let terminal = searched_terminal(&mut app, r"C:\\temp\\new", &[]);
    assert_snapshot!(with_highlights(&terminal, 1..=2), @r"
     09:12:01.000 INFO                           started
    ▶09:12:02.000 INFO                           C:\temp\new
                                                 ^^^^^^^^^^^
    ");
}
