use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use proptest::collection::vec;
use proptest::prelude::*;
use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use crate::clipboard::{CopyError, Via};
use crate::config::Config;
use crate::filter::{Base, Classes, Filters, Levels, Rule, TimeFilter, TimeRange, TimeSpec};
use crate::format::{BUILT_IN, Format, Level};
use crate::index::Progress;
use crate::keymap::Keymap;
use crate::matcher::{self, Matcher};
use crate::test_support;
use crate::test_support::{
    INDEX_DEADLINE, LARGE_INDEX_DEADLINE, action, app, app_in, built_in, char_key, ctrl,
    detected_app, following_app, json_fixture, key, named_app, no_thread_or_logger, numbered_log,
    unindexed_app, wait_for_filter, wait_for_index, wait_for_index_within, wait_for_search,
};

/// `(top, selected)`: the first line on screen and the selected line.
fn pos(app: &App) -> (usize, Option<usize>) {
    (app.viewport().top(), app.viewport().selected())
}

/// 100 lines on a 10-line screen.
fn hundred_lines() -> App {
    let mut app = app(numbered_log(100).as_bytes());
    app.resize(10);
    app
}

fn applied(mut app: App, actions: &[Action]) -> App {
    for &action in actions {
        app.apply(action);
    }
    app
}

// --- key mapping: moves ---

#[test]
fn j_moves_down() {
    assert_eq!(action(char_key('j')), Some(Action::Down));
}

#[test]
fn down_arrow_moves_down() {
    assert_eq!(action(key(KeyCode::Down)), Some(Action::Down));
}

#[test]
fn k_moves_up() {
    assert_eq!(action(char_key('k')), Some(Action::Up));
}

#[test]
fn up_arrow_moves_up() {
    assert_eq!(action(key(KeyCode::Up)), Some(Action::Up));
}

#[test]
fn page_down_key_pages_down() {
    assert_eq!(action(key(KeyCode::PageDown)), Some(Action::PageDown));
}

#[test]
fn page_up_key_pages_up() {
    assert_eq!(action(key(KeyCode::PageUp)), Some(Action::PageUp));
}

#[test]
fn ctrl_d_moves_half_a_page_down() {
    assert_eq!(action(ctrl('d')), Some(Action::HalfPageDown));
}

#[test]
fn ctrl_u_moves_half_a_page_up() {
    assert_eq!(action(ctrl('u')), Some(Action::HalfPageUp));
}

#[test]
fn g_goes_to_the_top() {
    assert_eq!(action(char_key('g')), Some(Action::Top));
}

#[test]
fn home_goes_to_the_top() {
    assert_eq!(action(key(KeyCode::Home)), Some(Action::Top));
}

#[test]
fn capital_g_goes_to_the_bottom() {
    assert_eq!(action(char_key('G')), Some(Action::Bottom));
}

#[test]
fn capital_g_reported_with_shift_goes_to_the_bottom() {
    let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
    assert_eq!(action(shift_g), Some(Action::Bottom));
}

#[test]
fn end_goes_to_the_bottom() {
    assert_eq!(action(key(KeyCode::End)), Some(Action::Bottom));
}

// --- key mapping: the table ---

#[test]
fn left_arrow_scrolls_left() {
    assert_eq!(action(key(KeyCode::Left)), Some(Action::ScrollLeft));
}

#[test]
fn right_arrow_scrolls_right() {
    assert_eq!(action(key(KeyCode::Right)), Some(Action::ScrollRight));
}

#[test]
fn capital_f_toggles_follow() {
    assert_eq!(action(char_key('F')), Some(Action::ToggleFollow));
}

#[test]
fn capital_t_toggles_the_thread_column() {
    assert_eq!(action(char_key('T')), Some(Action::ToggleThread));
}

#[test]
fn hash_toggles_the_line_numbers() {
    assert_eq!(action(char_key('#')), Some(Action::ToggleLineNumbers));
}

#[test]
fn w_toggles_wrapping() {
    assert_eq!(action(char_key('w')), Some(Action::ToggleWrap));
}

#[test]
fn capital_p_cycles_the_preset() {
    assert_eq!(action(char_key('P')), Some(Action::CyclePreset));
}

#[test]
fn space_toggles_the_fold_of_the_selected_entry() {
    assert_eq!(action(char_key(' ')), Some(Action::ToggleFold));
}

#[test]
fn z_folds_every_entry() {
    assert_eq!(action(char_key('z')), Some(Action::FoldAll));
}

#[test]
fn capital_z_unfolds_every_entry() {
    assert_eq!(action(char_key('Z')), Some(Action::UnfoldAll));
}

// --- key mapping: filters and panels ---

#[test]
fn digits_1_to_6_toggle_trace_to_fatal() {
    for (digit, level) in ('1'..='6').zip(Level::ALL) {
        assert_eq!(
            action(char_key(digit)),
            Some(Action::ToggleLevel(level)),
            "{digit}"
        );
    }
}

#[test]
fn digits_0_and_7_do_nothing() {
    assert_eq!(action(char_key('0')), None);
    assert_eq!(action(char_key('7')), None);
}

#[test]
fn f2_opens_the_level_panel() {
    assert_eq!(action(key(KeyCode::F(2))), Some(Action::LevelPanel));
}

#[test]
fn c_shows_only_the_selected_class() {
    assert_eq!(action(char_key('c')), Some(Action::OnlyClass));
}

#[test]
fn capital_c_hides_the_selected_class() {
    assert_eq!(action(char_key('C')), Some(Action::HideClass));
}

#[test]
fn f6_opens_the_class_panel() {
    assert_eq!(action(key(KeyCode::F(6))), Some(Action::ClassPanel));
}

#[test]
fn enter_confirms() {
    assert_eq!(action(key(KeyCode::Enter)), Some(Action::Confirm));
}

#[test]
fn tab_cycles_the_focus() {
    assert_eq!(action(key(KeyCode::Tab)), Some(Action::CycleFocus));
}

#[test]
fn esc_cancels() {
    assert_eq!(action(key(KeyCode::Esc)), Some(Action::Cancel));
}

#[test]
fn f4_and_ampersand_open_the_text_filter_prompt() {
    assert_eq!(action(key(KeyCode::F(4))), Some(Action::TextPanel));
    assert_eq!(action(char_key('&')), Some(Action::TextPanel));
}

#[test]
fn x_clears_the_filters() {
    assert_eq!(action(char_key('x')), Some(Action::ClearFilters));
}

// --- key mapping: quitting ---

#[test]
fn q_quits() {
    assert_eq!(action(char_key('q')), Some(Action::Quit));
}

#[test]
fn f10_quits() {
    assert_eq!(action(key(KeyCode::F(10))), Some(Action::Quit));
}

#[test]
fn ctrl_c_quits() {
    assert_eq!(action(ctrl('c')), Some(Action::Quit));
}

// --- key mapping: what does nothing ---

#[test]
fn d_and_u_without_ctrl_do_nothing() {
    assert_eq!(action(char_key('d')), None);
    assert_eq!(action(char_key('u')), None);
}

#[test]
fn letters_with_ctrl_are_not_plain_letters() {
    assert_eq!(action(ctrl('j')), None);
    assert_eq!(action(ctrl('g')), None);
    assert_eq!(action(ctrl('q')), None);
}

fn shifted(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

// Shift is ignored only for characters, which carry their case themselves.

#[test]
fn shift_end_does_nothing() {
    assert_eq!(action(shifted(KeyCode::End)), None);
}

#[test]
fn shift_home_does_nothing() {
    assert_eq!(action(shifted(KeyCode::Home)), None);
}

#[test]
fn shift_down_does_nothing() {
    assert_eq!(action(shifted(KeyCode::Down)), None);
}

#[test]
fn shift_page_down_does_nothing() {
    assert_eq!(action(shifted(KeyCode::PageDown)), None);
}

#[test]
fn shift_f10_does_nothing() {
    assert_eq!(action(shifted(KeyCode::F(10))), None);
}

#[test]
fn letters_with_alt_are_not_plain_letters() {
    let alt_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT);
    assert_eq!(action(alt_j), None);
}

#[test]
fn unbound_keys_do_nothing() {
    assert_eq!(action(char_key('v')), None);
    assert_eq!(action(key(KeyCode::F(12))), None);
}

#[test]
fn releasing_a_key_does_nothing() {
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    assert_eq!(action(release), None);
}

#[test]
fn holding_a_key_repeats_its_action() {
    let repeat =
        KeyEvent::new_with_kind(KeyCode::Char('j'), KeyModifiers::NONE, KeyEventKind::Repeat);
    assert_eq!(action(repeat), Some(Action::Down));
}

// --- the file ---

#[test]
fn file_name_is_the_last_component_of_the_path() {
    let app = named_app("/var/log/wildfly/server.log", b"x\n");
    assert_eq!(app.file_name(), "server.log");
}

#[test]
fn relative_file_name_is_the_last_component_too() {
    let app = named_app("logs/server.log", b"x\n");
    assert_eq!(app.file_name(), "server.log");
}

#[test]
fn non_utf8_file_name_is_decoded_lossily() {
    let path = PathBuf::from(OsString::from_vec(b"/var/log/caf\xe9.log".to_vec()));
    let app = named_app(path, b"x\n");
    assert_eq!(app.file_name(), "caf\u{FFFD}.log");
}

#[test]
fn size_is_the_file_length_in_bytes() {
    assert_eq!(app(b"one\ntwo\r\n").size(), 9);
}

#[test]
fn empty_file_has_size_zero() {
    assert_eq!(app(b"").size(), 0);
}

// --- rows from the index ---

const STARTED: &str = "2026-10-07 12:00:00.123 [main] INFO  com.example.App - Started\n";
const FAILED: &str = "2026-10-07 12:00:01.500 [main] ERROR com.example.App - Failed\n";
const EXCEPTION: &str = "java.lang.IllegalStateException: boom\n";
const FRAME: &str = "\tat com.example.App.main(App.java:10)\n";
const DONE: &str = "2026-10-07 12:00:02.000 [main] WARN  com.example.App - Done\n";

fn logback(lines: &[&str]) -> App {
    app_in(built_in("logback"), lines.concat().as_bytes())
}

#[test]
fn every_entry_is_a_row() {
    let app = logback(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]);
    assert_eq!(app.viewport().total(), 3);
}

#[test]
fn row_shows_the_first_line_of_its_entry() {
    let app = logback(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]);
    assert_eq!(app.line(1), FAILED.trim_end());
    assert_eq!(app.line(2), DONE.trim_end());
}

#[test]
fn row_of_the_headless_entry_shows_the_first_line_of_the_file() {
    let app = logback(&[EXCEPTION, FRAME, STARTED]);
    assert_eq!(app.viewport().total(), 2);
    assert_eq!(app.line(0), EXCEPTION.trim_end());
}

#[test]
fn row_of_an_entry_with_crlf_endings_has_no_line_ending() {
    let text = [FAILED, FRAME, DONE].concat().replace('\n', "\r\n");
    let app = app_in(built_in("logback"), text.as_bytes());
    assert_eq!(app.line(0), FAILED.trim_end());
    assert_eq!(app.line(1), DONE.trim_end());
}

#[test]
fn last_row_without_a_newline_shows_the_whole_line() {
    let app = app(b"first\nlast");
    assert_eq!(app.line(1), "last");
}

#[test]
fn new_app_has_no_rows_before_the_index_is_polled() {
    let app = unindexed_app(built_in("raw"), numbered_log(100).as_bytes());
    assert_eq!(app.viewport().total(), 0);
    assert_eq!(app.viewport().selected(), None);
}

#[test]
fn app_indexes_in_the_preset_zone_and_anchor_it_was_given() {
    let mtime: Timestamp = "2026-10-08T12:00:00Z".parse().unwrap();
    let zone = TimeZone::get("Europe/Warsaw").unwrap();
    let app = App::new(
        Path::new("/var/log/console.log"),
        test_support::source(b"12:00:00,123 INFO  [org.jboss.as] (main) started\n"),
        Some(built_in("wildfly-console")),
        zone.clone(),
        mtime,
        false,
    );
    let settings = app.settings();
    assert_eq!(settings.format, built_in("wildfly-console"));
    assert_eq!(settings.zone, zone);
    assert_eq!(
        settings.anchor,
        Anchor::Modified("2026-10-08T14:00:00".parse().unwrap())
    );
}

#[test]
fn anchor_comes_from_a_date_in_the_file_name() {
    let app = App::new(
        Path::new("/var/log/server.log.2026-10-06"),
        test_support::source(b"x\n"),
        Some(built_in("logback-basic")),
        test_support::zone(),
        test_support::mtime(),
        false,
    );
    assert_eq!(
        app.settings().anchor,
        Anchor::Named("2026-10-06".parse().unwrap())
    );
}

#[test]
fn app_without_a_format_reads_the_file_in_the_detected_preset() {
    let app = detected_app(
        [STARTED, FAILED, EXCEPTION, FRAME, DONE]
            .concat()
            .as_bytes(),
    );
    assert_eq!(app.settings().format, built_in("logback"));
    assert_eq!(app.viewport().total(), 3);
}

#[test]
fn new_app_has_indexed_none_of_the_file() {
    let content = numbered_log(100);
    let app = unindexed_app(built_in("raw"), content.as_bytes());
    let total = content.len() as u64;
    assert_eq!(app.progress(), Progress { indexed: 0, total });
}

#[test]
fn polling_the_index_to_the_end_brings_in_every_row() {
    let content = numbered_log(100);
    let mut app = unindexed_app(built_in("raw"), content.as_bytes());
    wait_for_index(&mut app);
    let total = content.len() as u64;
    assert_eq!(
        app.progress(),
        Progress {
            indexed: total,
            total
        }
    );
    assert_eq!(app.viewport().total(), 100);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn empty_file_is_indexed_without_rows() {
    let app = app(b"");
    assert!(app.progress().is_complete());
    assert_eq!(app.viewport().total(), 0);
}

#[test]
fn rows_taken_in_later_keep_the_selection() {
    let mut app = unindexed_app(built_in("logback-basic"), numbered_log(40_000).as_bytes());
    app.resize(10);
    let deadline = Instant::now() + LARGE_INDEX_DEADLINE;
    while app.viewport().total() < 2 {
        assert!(Instant::now() < deadline, "no rows came in");
        thread::sleep(Duration::from_millis(1));
        app.poll_index();
    }
    app.apply(Action::Down);
    wait_for_index_within(&mut app, LARGE_INDEX_DEADLINE);
    assert_eq!(app.viewport().total(), 40_000);
    assert_eq!(pos(&app), (0, Some(1)));
}

#[test]
fn every_line_of_the_file_is_a_row() {
    let app = app(numbered_log(100).as_bytes());
    assert_eq!(app.viewport().total(), 100);
}

#[test]
fn line_is_the_text_without_its_line_ending() {
    let app = app(b"first\r\nsecond\n");
    assert_eq!(app.line(0), "first");
    assert_eq!(app.line(1), "second");
}

#[test]
fn line_with_invalid_utf8_is_decoded_lossily() {
    let app = app(b"caf\xe9 au lait\n");
    assert_eq!(app.line(0), "caf\u{FFFD} au lait");
}

#[test]
fn line_keeps_control_characters_for_the_ui_to_replace() {
    let app = app(b"\tat \x1b[31mred\n");
    assert_eq!(app.line(0), "\tat \x1b[31mred");
}

// --- starting state ---

#[test]
fn new_app_is_at_the_top_with_the_first_line_selected() {
    assert_eq!(pos(&hundred_lines()), (0, Some(0)));
}

#[test]
fn new_app_keeps_running() {
    assert!(!hundred_lines().should_quit());
}

#[test]
fn empty_file_has_no_selection() {
    let mut app = app(b"");
    app.resize(10);
    assert_eq!(pos(&app), (0, None));
}

#[test]
fn line_numbers_are_shown_from_the_start() {
    assert!(hundred_lines().shows_line_numbers());
}

#[test]
fn resize_sets_the_screen_height() {
    let app = hundred_lines();
    assert_eq!(app.viewport().height(), 10);
}

// --- actions ---

#[test]
fn down_selects_the_next_line() {
    let app = applied(hundred_lines(), &[Action::Down]);
    assert_eq!(pos(&app), (0, Some(1)));
}

#[test]
fn up_selects_the_previous_line() {
    let app = applied(hundred_lines(), &[Action::Down, Action::Down, Action::Up]);
    assert_eq!(pos(&app), (0, Some(1)));
}

#[test]
fn page_down_moves_a_screen_down() {
    let app = applied(hundred_lines(), &[Action::Down, Action::PageDown]);
    assert_eq!(pos(&app), (10, Some(11)));
}

#[test]
fn page_up_moves_a_screen_up() {
    let app = applied(
        hundred_lines(),
        &[Action::PageDown, Action::PageDown, Action::PageUp],
    );
    assert_eq!(pos(&app), (10, Some(10)));
}

#[test]
fn half_page_down_moves_half_a_screen_down() {
    let app = applied(hundred_lines(), &[Action::HalfPageDown]);
    assert_eq!(pos(&app), (5, Some(5)));
}

#[test]
fn half_page_up_moves_half_a_screen_up() {
    let app = applied(hundred_lines(), &[Action::PageDown, Action::HalfPageUp]);
    assert_eq!(pos(&app), (5, Some(5)));
}

#[test]
fn bottom_selects_the_last_line_on_the_last_screen() {
    let app = applied(hundred_lines(), &[Action::Bottom]);
    assert_eq!(pos(&app), (90, Some(99)));
}

#[test]
fn top_selects_the_first_line_on_the_first_screen() {
    let app = applied(hundred_lines(), &[Action::Bottom, Action::Top]);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn quit_asks_the_viewer_to_stop() {
    let app = applied(hundred_lines(), &[Action::Quit]);
    assert!(app.should_quit());
}

#[test]
fn quit_does_not_move() {
    let app = applied(hundred_lines(), &[Action::Down, Action::Quit]);
    assert_eq!(pos(&app), (0, Some(1)));
}

#[test]
fn moves_do_not_quit() {
    let app = applied(
        hundred_lines(),
        &[
            Action::Down,
            Action::Up,
            Action::PageDown,
            Action::PageUp,
            Action::HalfPageDown,
            Action::HalfPageUp,
            Action::Bottom,
            Action::Top,
        ],
    );
    assert!(!app.should_quit());
}

#[test]
fn moves_in_an_empty_file_keep_it_without_a_selection() {
    let mut app = app(b"");
    app.resize(10);
    let app = applied(
        app,
        &[
            Action::Down,
            Action::PageDown,
            Action::HalfPageDown,
            Action::Bottom,
            Action::Up,
            Action::PageUp,
            Action::HalfPageUp,
            Action::Top,
        ],
    );
    assert_eq!(pos(&app), (0, None));
}

#[test]
fn toggling_the_line_numbers_hides_them_and_shows_them_again() {
    let mut app = applied(hundred_lines(), &[Action::ToggleLineNumbers]);
    assert!(!app.shows_line_numbers());
    app.apply(Action::ToggleLineNumbers);
    assert!(app.shows_line_numbers());
}

// --- resizing ---

#[test]
fn growing_at_the_bottom_fills_the_screen_with_earlier_lines() {
    let mut app = applied(hundred_lines(), &[Action::Bottom]);
    app.resize(20);
    assert_eq!(pos(&app), (80, Some(99)));
}

#[test]
fn shrinking_at_the_bottom_keeps_the_last_line_on_screen() {
    let mut app = applied(hundred_lines(), &[Action::Bottom]);
    app.resize(4);
    assert_eq!(pos(&app), (96, Some(99)));
}

#[test]
fn page_size_follows_the_new_height() {
    let mut app = hundred_lines();
    app.resize(20);
    app.apply(Action::PageDown);
    assert_eq!(pos(&app), (20, Some(20)));
}

// --- following ---

/// The lines of the rows on screen, top to bottom.
fn screen(app: &App) -> Vec<String> {
    let viewport = app.viewport();
    let end = viewport.total().min(viewport.top() + viewport.height());
    (viewport.top()..end)
        .map(|i| app.line(i).into_owned())
        .collect()
}

#[test]
fn following_shows_the_last_screen_until_the_whole_index_takes_over() {
    // Several chunks of the index, so that some come in before it is complete.
    let log = numbered_log(10_000);
    let last_screen: Vec<&str> = log.lines().skip(9_990).collect();
    let mut app = following_app(built_in("raw"), log.as_bytes());
    app.resize(10);
    let deadline = Instant::now() + LARGE_INDEX_DEADLINE;
    loop {
        app.poll_index();
        assert_eq!(screen(&app), last_screen, "{:?}", app.progress());
        assert_eq!(
            app.viewport().selected(),
            Some(app.viewport().top() + 9),
            "{:?}",
            app.progress()
        );
        if app.progress().is_complete() {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", app.progress());
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(pos(&app), (9_990, Some(9_999)));
}

#[test]
fn following_reads_the_last_entries_once_the_screen_has_lines() {
    let log = numbered_log(100);
    let mut app = following_app(built_in("raw"), log.as_bytes());
    app.resize(0);
    app.resize(10);
    assert_eq!(screen(&app), log.lines().skip(90).collect::<Vec<_>>());
    assert_eq!(app.viewport().selected(), Some(app.viewport().top() + 9));
}

#[test]
fn following_a_file_whose_tail_is_too_far_back_waits_for_the_whole_index() {
    // The last line starts further from the end than the tail may read.
    let mut log = b"first\nsecond\n".to_vec();
    log.resize(log.len() + TAIL_LIMIT + 1, b'x');
    let mut app = following_app(built_in("raw"), &log);
    app.resize(10);
    let deadline = Instant::now() + INDEX_DEADLINE;
    loop {
        app.poll_index();
        if app.progress().is_complete() {
            break;
        }
        assert_eq!(app.viewport().total(), 0, "{:?}", app.progress());
        assert!(Instant::now() < deadline, "{:?}", app.progress());
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(pos(&app), (0, Some(2)));
}

#[test]
fn following_a_file_shorter_than_the_screen_selects_its_last_entry() {
    let mut app = following_app(
        built_in("logback"),
        [STARTED, FAILED, EXCEPTION, FRAME, DONE]
            .concat()
            .as_bytes(),
    );
    app.resize(10);
    app.poll_index();
    assert_eq!(pos(&app), (0, Some(2)));
    assert_eq!(app.line(2), DONE.trim_end());
    wait_for_index(&mut app);
    assert_eq!(pos(&app), (0, Some(2)));
}

// --- cycling presets ---

fn cycled(mut app: App) -> App {
    app.apply(Action::CyclePreset);
    app
}

#[test]
fn cycling_reads_the_file_in_the_next_format_of_the_table_and_wraps_around() {
    let ids: Vec<&str> = BUILT_IN.iter().map(|format| format.id).collect();
    for (i, format) in BUILT_IN.iter().enumerate() {
        let app = cycled(app_in(format, STARTED.as_bytes()));
        let next = ids[(i + 1) % ids.len()];
        assert_eq!(app.settings().format.id, next, "after {}", format.id);
    }
}

#[test]
fn cycling_indexes_the_file_again_in_the_new_preset() {
    let log = [STARTED, FAILED, EXCEPTION, FRAME, DONE].concat();
    let mut app = cycled(app(log.as_bytes()));
    wait_for_index(&mut app);
    assert_eq!(app.viewport().total(), 3);
    assert_eq!(app.line(2), DONE.trim_end());
}

#[test]
fn cycling_a_file_opened_at_the_top_selects_the_first_row_again() {
    let log = [STARTED, FAILED, EXCEPTION, FRAME, DONE].concat();
    let mut app = app(log.as_bytes());
    app.resize(10);
    let mut app = cycled(applied(app, &[Action::Down, Action::Down, Action::Down]));
    wait_for_index(&mut app);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn cycling_a_followed_file_shows_its_last_entries_in_the_new_preset_at_once() {
    let log = [STARTED, FAILED, EXCEPTION, FRAME, DONE].concat();
    let mut app = following_app(built_in("raw"), log.as_bytes());
    app.resize(10);
    wait_for_index(&mut app);
    assert_eq!(pos(&app), (0, Some(4)));
    let mut app = cycled(app);
    assert_eq!(app.viewport().total(), 3);
    assert_eq!(pos(&app), (0, Some(2)));
    assert_eq!(app.line(2), DONE.trim_end());
    wait_for_index(&mut app);
    assert_eq!(pos(&app), (0, Some(2)));
}

// --- follow mode ---

/// A viewer over `content` in `preset` on a 10-line screen, opened at the end to
/// follow it, with the whole file indexed.
fn followed(preset: &'static Format, content: &str) -> App {
    let mut app = following_app(preset, content.as_bytes());
    app.resize(10);
    wait_for_index(&mut app);
    app
}

/// `app` after the file changed as `change` says, with what changed indexed.
fn changed(mut app: App, change: FileChange) -> App {
    app.file_changed(change);
    wait_for_index(&mut app);
    app
}

/// `app` after its file grew to `content`.
fn grown(app: App, content: &str) -> App {
    changed(
        app,
        FileChange::Grew(test_support::source(content.as_bytes())),
    )
}

/// `app` after its file was cut short and holds `content` now.
fn truncated(app: App, content: &str) -> App {
    changed(
        app,
        FileChange::Truncated(test_support::source(content.as_bytes())),
    )
}

/// 20 numbered lines, followed.
fn twenty_lines_followed() -> App {
    followed(built_in("raw"), &numbered_log(20))
}

#[test]
fn opening_at_the_end_follows_the_file() {
    assert!(twenty_lines_followed().following());
}

#[test]
fn opening_at_the_top_does_not_follow_the_file() {
    assert!(!hundred_lines().following());
}

#[test]
fn appended_entries_are_shown_with_the_last_one_selected_while_following() {
    let app = grown(twenty_lines_followed(), &numbered_log(25));
    assert_eq!(pos(&app), (15, Some(24)));
    assert_eq!(app.line(24), numbered_log(25).lines().last().unwrap());
    assert!(app.following());
    assert_eq!(app.new_entries(), 0);
}

#[test]
fn every_move_away_from_the_bottom_stops_following() {
    for action in [Action::Up, Action::PageUp, Action::HalfPageUp, Action::Top] {
        let app = applied(twenty_lines_followed(), &[action]);
        assert!(!app.following(), "{action:?}");
    }
}

#[test]
fn actions_that_stay_at_the_bottom_keep_following() {
    let app = applied(
        twenty_lines_followed(),
        &[
            Action::Down,
            Action::PageDown,
            Action::HalfPageDown,
            Action::Bottom,
            Action::ScrollRight,
            Action::ToggleThread,
        ],
    );
    assert!(app.following());
}

#[test]
fn appends_while_paused_do_not_move_the_view() {
    let app = applied(twenty_lines_followed(), &[Action::PageUp]);
    let before = pos(&app);
    let app = grown(app, &numbered_log(25));
    assert_eq!(pos(&app), before);
    assert_eq!(app.viewport().total(), 25);
}

#[test]
fn appends_while_paused_count_as_new_entries() {
    let app = applied(twenty_lines_followed(), &[Action::Up]);
    let app = grown(app, &numbered_log(23));
    assert_eq!(app.new_entries(), 3);
    let app = grown(app, &numbered_log(25));
    assert_eq!(app.new_entries(), 5);
}

#[test]
fn lines_added_to_the_last_entry_are_not_new_entries() {
    let app = followed(built_in("logback"), &[STARTED, FAILED].concat());
    let app = applied(app, &[Action::Up]);
    let app = grown(app, &[STARTED, FAILED, EXCEPTION, FRAME, DONE].concat());
    assert_eq!(app.new_entries(), 1);
}

#[test]
fn toggling_follow_while_following_stops_following_in_place() {
    let app = applied(twenty_lines_followed(), &[Action::ToggleFollow]);
    assert!(!app.following());
    assert_eq!(pos(&app), (10, Some(19)));
}

#[test]
fn toggling_follow_while_paused_follows_again_from_the_bottom() {
    let app = applied(twenty_lines_followed(), &[Action::PageUp]);
    let app = applied(grown(app, &numbered_log(25)), &[Action::ToggleFollow]);
    assert!(app.following());
    assert_eq!(pos(&app), (15, Some(24)));
    assert_eq!(app.new_entries(), 0);
}

#[test]
fn going_to_the_bottom_follows_again() {
    let app = applied(twenty_lines_followed(), &[Action::Top]);
    let app = applied(grown(app, &numbered_log(25)), &[Action::Bottom]);
    assert!(app.following());
    assert_eq!(app.new_entries(), 0);
}

#[test]
fn moving_down_onto_the_last_row_does_not_follow_again() {
    let app = applied(twenty_lines_followed(), &[Action::Up, Action::Down]);
    assert_eq!(pos(&app), (10, Some(19)));
    assert!(!app.following());
}

#[test]
fn growth_with_the_index_complete_is_shown_at_once() {
    let mut app = twenty_lines_followed();
    app.file_changed(FileChange::Grew(test_support::source(
        numbered_log(21).as_bytes(),
    )));
    assert_eq!(app.viewport().total(), 21);
}

#[test]
fn growth_during_indexing_is_appended_to_the_complete_index() {
    // Appended entries are dated forward from the file as it was, which is dated back
    // from its modification time at noon: 11:00 stays on its day and 15:00 follows
    // it. Indexed at once, 15:00 would be too far past noon for that day.
    let before = "11:00:00.000 [main] INFO  com.example.App - before\n";
    let after = "15:00:00.000 [main] INFO  com.example.App - after\n";
    let mut app = unindexed_app(built_in("logback-basic"), before.as_bytes());
    let grown = test_support::source([before, after].concat().as_bytes());
    app.file_changed(FileChange::Grew(grown));
    wait_for_index(&mut app);
    let timestamp = |i| app.row(i).unwrap().record.timestamp().unwrap().to_string();
    assert_eq!(app.viewport().total(), 2);
    assert_eq!(timestamp(0), "2026-10-08T11:00:00Z");
    assert_eq!(timestamp(1), "2026-10-08T15:00:00Z");
}

#[test]
fn growth_shorter_than_the_index_changes_nothing() {
    // The file was cut short between the watcher's look and the mapping.
    let app = grown(twenty_lines_followed(), &numbered_log(5));
    assert_eq!(app.viewport().total(), 20);
    assert_eq!(pos(&app), (10, Some(19)));
}

#[test]
fn truncation_while_paused_selects_the_truncation_row() {
    let app = applied(twenty_lines_followed(), &[Action::Up]);
    let app = truncated(app, "after 1\nafter 2\n");
    assert_eq!(pos(&app), (0, Some(0)));
    assert!(!app.following());
}

#[test]
fn truncation_while_paused_counts_the_new_content_as_new_entries() {
    let app = applied(twenty_lines_followed(), &[Action::Up]);
    let app = grown(app, &numbered_log(22));
    let app = truncated(app, "after 1\nafter 2\nafter 3\n");
    assert_eq!(app.new_entries(), 3);
}

#[test]
fn toggling_the_fold_on_the_truncation_row_does_nothing() {
    let log = [FAILED, EXCEPTION].concat();
    let app = truncated(followed(built_in("logback"), &log), &log);
    let app = applied(app, &[Action::Top, Action::ToggleFold]);
    assert_eq!(app.viewport().total(), 2);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn lines_are_read_from_the_new_content_after_truncation() {
    let app = applied(logback(&[FAILED, EXCEPTION, FRAME]), &[Action::UnfoldAll]);
    assert_eq!(app.line(2), FRAME.trim_end());
    let caused = "Caused by: java.io.IOException: x\n";
    let app = truncated(app, &[FAILED, caused, FRAME].concat());
    let app = applied(app, &[Action::UnfoldAll]);
    assert_eq!(app.line(2), caused.trim_end());
}

#[test]
fn folded_entries_stay_folded_as_the_file_grows() {
    let app = followed(built_in("logback"), &[STARTED, FAILED, EXCEPTION].concat());
    let app = grown(app, &[STARTED, FAILED, EXCEPTION, DONE].concat());
    assert_eq!(app.viewport().total(), 3);
}

#[test]
fn entry_gaining_lines_after_unfold_all_is_unfolded() {
    let app = applied(
        followed(built_in("logback"), &[STARTED, FAILED].concat()),
        &[Action::UnfoldAll],
    );
    let app = grown(app, &[STARTED, FAILED, EXCEPTION, FRAME].concat());
    assert_eq!(app.viewport().total(), 4);
}

#[test]
fn unfolded_last_entry_shows_the_lines_appended_to_it() {
    let app = followed(built_in("logback"), &[STARTED, FAILED, EXCEPTION].concat());
    let app = applied(app, &[Action::ToggleFold]);
    assert_eq!(app.viewport().total(), 3);
    let app = grown(app, &[STARTED, FAILED, EXCEPTION, FRAME].concat());
    assert_eq!(app.viewport().total(), 4);
    assert_eq!(app.line(3), FRAME.trim_end());
}

/// The viewer lets go of a deleted file, so nothing of it is left to show, not even
/// in the header's level counters or its size.
#[test]
fn deleted_file_empties_the_view_and_is_waited_for() {
    let app = followed(built_in("logback-basic"), &numbered_log(20));
    let app = changed(app, FileChange::Gone);
    assert!(app.waiting_for_file());
    assert_eq!(app.viewport().total(), 0);
    assert_eq!(app.viewport().selected(), None);
    assert_eq!(app.size(), 0);
    assert_eq!(app.level_counts(), [0; 6]);
    assert!(app.following());
}

#[test]
fn deleted_file_drops_the_new_entry_count_and_stays_paused() {
    let app = applied(twenty_lines_followed(), &[Action::Up]);
    let app = changed(grown(app, &numbered_log(25)), FileChange::Gone);
    assert_eq!(app.viewport().total(), 0);
    assert_eq!(app.new_entries(), 0);
    assert!(!app.following());
}

#[test]
fn deleted_file_drops_the_filters_results_but_keeps_the_filters() {
    let app = filtered(
        followed(built_in("logback"), &[STARTED, FAILED, DONE].concat()),
        text_filter("Failed"),
    );
    let mut app = changed(app, FileChange::Gone);
    wait_for_filter(&mut app);
    assert!(rows(&app).is_empty());
    assert!(!app.nothing_matches());
    let mut app = changed(
        app,
        FileChange::Replaced(test_support::source([DONE, FAILED].concat().as_bytes())),
    );
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), [FAILED.trim_end()]);
}

#[test]
fn deleted_file_stops_a_filter_still_evaluating_it() {
    let mut app = unindexed_app(built_in("raw"), numbered_log(5_000).as_bytes());
    app.set_filters(text_filter("step 4"));
    let deadline = Instant::now() + INDEX_DEADLINE;
    while !app.filtering() {
        assert!(Instant::now() < deadline, "the filter never fell behind");
        app.poll_index();
    }
    let mut app = changed(app, FileChange::Gone);
    wait_for_filter(&mut app);
    assert_eq!(app.viewport().total(), 0);
}

#[test]
fn deleted_file_drops_the_search_matches() {
    let app = searched(twenty_lines_followed(), '/', "step 1");
    assert_eq!(app.match_count(), Some((1, 11)));
    let mut app = changed(app, FileChange::Gone);
    wait_for_search(&mut app);
    assert_eq!(app.match_count(), Some((0, 0)));
}

#[test]
fn deleted_file_empties_the_details_pane() {
    let app = applied(details_focused(), &[Action::Down]);
    assert_eq!(app.details_scroll(), 1);
    let app = changed(app, FileChange::Gone);
    assert!(app.details_entry().is_none());
    assert_eq!(app.details_scroll(), 0);
}

#[test]
fn deleted_file_cancels_a_waiting_export() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = export_before_polling(&dir);
    app.file_changed(FileChange::Gone);
    assert_export_cancelled(app, &dir);
}

#[test]
fn truncation_or_rotation_cancels_a_waiting_export() {
    let changes: [fn(Source) -> FileChange; 2] = [FileChange::Truncated, FileChange::Replaced];
    for change in changes {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = export_before_polling(&dir);
        app.file_changed(change(test_support::source(TWO_TRACES.concat().as_bytes())));
        assert_export_cancelled(app, &dir);
    }
}

#[test]
fn file_back_at_the_path_is_shown_from_its_start() {
    let app = changed(twenty_lines_followed(), FileChange::Gone);
    let app = changed(
        app,
        FileChange::Replaced(test_support::source(b"new 1\nnew 2\n")),
    );
    assert!(!app.waiting_for_file());
    assert_eq!(screen(&app), ["new 1", "new 2"]);
    assert_eq!(app.viewport().selected(), Some(1));
    assert!(app.following());
}

// --- liveness ---

const TEN_SECONDS: Duration = Duration::from_secs(10);
const FIVE_MINUTES: Duration = Duration::from_secs(300);

/// `app` told the time `at`.
fn ticked(mut app: App, at: Instant) -> App {
    app.tick(at);
    app
}

/// `app` opened at `start` and told the time five minutes later, by when an unchanged
/// file is idle.
fn idle(app: App, start: Instant) -> App {
    ticked(ticked(app, start), start + FIVE_MINUTES)
}

#[test]
fn liveness_counts_from_opening_while_the_file_does_not_change() {
    let start = Instant::now();
    let app = ticked(twenty_lines_followed(), start);
    assert_eq!(app.liveness(), Liveness::Live);
    let app = ticked(app, start + TEN_SECONDS);
    assert_eq!(app.liveness(), Liveness::Quiet);
}

/// The change happened at the latest tick, and the liveness counts from there.
#[test]
fn growth_is_a_change_also_without_following() {
    let start = Instant::now();
    let app = idle(app(numbered_log(20).as_bytes()), start);
    assert_eq!(app.liveness(), Liveness::Idle);
    let app = grown(app, &numbered_log(21));
    let app = ticked(
        app,
        start + FIVE_MINUTES + TEN_SECONDS - Duration::from_millis(1),
    );
    assert_eq!(app.liveness(), Liveness::Live);
}

#[test]
fn truncation_is_a_change() {
    let start = Instant::now();
    let app = truncated(idle(twenty_lines_followed(), start), &numbered_log(2));
    assert_eq!(app.liveness(), Liveness::Live);
}

#[test]
fn missing_file_is_idle() {
    let app = ticked(twenty_lines_followed(), Instant::now());
    let app = changed(app, FileChange::Gone);
    assert_eq!(app.liveness(), Liveness::Idle);
}

/// After a rotation, or once the file is back after it was gone.
#[test]
fn another_file_at_the_path_is_a_change() {
    let start = Instant::now();
    let app = changed(idle(twenty_lines_followed(), start), FileChange::Gone);
    let app = changed(
        app,
        FileChange::Replaced(test_support::source(b"new 1\nnew 2\n")),
    );
    assert_eq!(app.liveness(), Liveness::Live);
}

#[test]
fn gzip_file_is_idle() {
    let app = ticked(hundred_lines_gzipped(), Instant::now());
    assert_eq!(app.liveness(), Liveness::Idle);
}

/// Something that happens to a followed Logback file or its viewer.
#[derive(Debug, Clone)]
enum Step {
    Apply(Action),
    /// The file gets these bytes appended.
    Append(Vec<u8>),
    /// The file is cut short and then holds these bytes.
    Truncate(Vec<u8>),
    /// Another file holding these bytes takes the file's place.
    Replace(Vec<u8>),
    Gone,
}

/// Logback headers and continuation lines mixed with arbitrary bytes, not always
/// ending a line. The bytes leave out 0x1f, so that no file starts like a gzip file,
/// which is opened differently.
fn some_log() -> impl Strategy<Value = Vec<u8>> {
    let byte = any::<u8>().prop_map(|b| if b == 0x1f { b'?' } else { b });
    let part = prop_oneof![
        prop::sample::select(vec![STARTED, FAILED, EXCEPTION, FRAME, DONE])
            .prop_map(|line| line.as_bytes().to_vec()),
        vec(byte, 0..20),
    ];
    vec(part, 0..4).prop_map(|parts| parts.concat())
}

fn step() -> impl Strategy<Value = Step> {
    let action = prop::sample::select(vec![
        Action::Up,
        Action::Down,
        Action::PageUp,
        Action::Top,
        Action::Bottom,
        Action::ToggleFollow,
        Action::ToggleFold,
        Action::FoldAll,
        Action::UnfoldAll,
    ]);
    prop_oneof![
        4 => action.prop_map(Step::Apply),
        4 => some_log().prop_map(Step::Append),
        1 => some_log().prop_map(Step::Truncate),
        1 => some_log().prop_map(Step::Replace),
        1 => Just(Step::Gone),
    ]
}

/// Every row's line, top to bottom.
fn all_lines(app: &App) -> Vec<String> {
    (0..app.viewport().total())
        .map(|i| app.line(i).into_owned())
        .collect()
}

// --- gzip files ---

/// 100 numbered lines in a gzip file on a 10-line screen, opened at the end, with
/// the whole file indexed.
fn hundred_lines_gzipped() -> App {
    let mut app = test_support::gzip_app(&test_support::gzip(numbered_log(100).as_bytes()));
    app.resize(10);
    wait_for_index(&mut app);
    app
}

#[test]
fn gzip_file_opens_at_the_end_without_following() {
    let app = hundred_lines_gzipped();
    assert!(!app.following());
    assert_eq!(pos(&app), (90, Some(99)));
}

#[test]
fn going_to_the_bottom_of_a_gzip_file_does_not_follow() {
    let app = applied(hundred_lines_gzipped(), &[Action::Top, Action::Bottom]);
    assert!(!app.following());
    assert_eq!(pos(&app), (90, Some(99)));
}

#[test]
fn toggling_follow_on_a_gzip_file_does_nothing() {
    let app = applied(
        hundred_lines_gzipped(),
        &[Action::Top, Action::ToggleFollow],
    );
    assert!(!app.following());
    assert_eq!(pos(&app), (0, Some(0)));
}

/// Cycling the preset opens the file again the way it was opened, at the end, even
/// though a gzip file is never followed.
#[test]
fn cycling_the_preset_of_a_gzip_file_opened_at_the_end_stays_at_the_end() {
    let log: String = (1..=100)
        .map(|i| format!("2026-10-07 12:00:00.{i:03} [main] INFO  com.example.App - step {i}\n"))
        .collect();
    let mut app = test_support::gzip_app(&test_support::gzip(log.as_bytes()));
    app.resize(10);
    wait_for_index(&mut app);
    let mut app = cycled(app);
    wait_for_index(&mut app);
    assert_eq!(app.settings().format, built_in("logback"));
    assert_eq!(pos(&app), (90, Some(99)));
    assert!(!app.following());
}

#[test]
fn gzip_file_is_not_watched() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("app.log.gz");
    fs::write(&path, test_support::gzip(numbered_log(100).as_bytes())).unwrap();
    let mut app = hundred_lines_gzipped();
    app.watch(&path, File::open(&path).unwrap(), Options::default())
        .unwrap();
    assert!(!app.expects_updates());
}

proptest! {
    #[test]
    fn followed_file_shows_the_rows_of_its_content_indexed_at_once(
        start in some_log(),
        steps in vec(step(), 0..8),
    ) {
        let mut app = following_app(built_in("logback"), &start);
        app.resize(5);
        wait_for_index(&mut app);
        let mut content = start;
        let mut truncation_row = false;
        // Once the file is gone, only another file at the path comes from the watcher.
        let mut gone = false;
        for step in steps {
            match step {
                Step::Apply(action) => app.apply(action),
                Step::Append(_) | Step::Truncate(_) if gone => {}
                Step::Append(bytes) => {
                    content.extend(bytes);
                    app.file_changed(FileChange::Grew(test_support::source(&content)));
                }
                Step::Truncate(bytes) => {
                    content = bytes;
                    truncation_row = true;
                    app.file_changed(FileChange::Truncated(test_support::source(&content)));
                }
                Step::Replace(bytes) => {
                    content = bytes;
                    truncation_row = false;
                    gone = false;
                    app.file_changed(FileChange::Replaced(test_support::source(&content)));
                }
                Step::Gone => {
                    content.clear();
                    truncation_row = false;
                    gone = true;
                    app.file_changed(FileChange::Gone);
                }
            }
            wait_for_index(&mut app);
            let total = app.viewport().total();
            prop_assert!(app.viewport().selected().is_none_or(|row| row < total));
        }
        app.apply(Action::UnfoldAll);
        let mut fresh = app_in(built_in("logback"), &content);
        fresh.apply(Action::UnfoldAll);
        let mut expected = all_lines(&fresh);
        if truncation_row {
            expected.insert(0, "── file truncated ──".to_string());
        }
        prop_assert_eq!(all_lines(&app), expected);
    }
}

// --- filters ---

/// Filters showing only entries whose message contains `word`.
fn text_filter(word: &str) -> Filters {
    Filters {
        text: Some(Matcher::new(word, matcher::Options::default()).unwrap()),
        ..Filters::default()
    }
}

/// `app` with `filters` set and their results in.
fn filtered(mut app: App, filters: Filters) -> App {
    app.set_filters(filters);
    wait_for_filter(&mut app);
    app
}

/// Every row of `app` as [`App::line`] shows it.
fn rows(app: &App) -> Vec<String> {
    (0..app.viewport().total())
        .map(|i| app.line(i).into_owned())
        .collect()
}

#[test]
fn filters_show_only_matching_entries() {
    let app = filtered(
        logback(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]),
        text_filter("Failed"),
    );
    assert_eq!(rows(&app), [FAILED.trim_end()]);
}

#[test]
fn filters_are_kept_as_set() {
    let mut app = logback(&[STARTED]);
    let mut filters = Filters::default();
    filters.classes.include = vec![Rule::Substring("com.example".to_string())];
    app.set_filters(filters.clone());
    assert_eq!(app.filters().classes, filters.classes);
}

#[test]
fn a_matching_entry_unfolds_in_the_filtered_rows() {
    let mut app = filtered(
        logback(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]),
        text_filter("Failed"),
    );
    app.resize(10);
    app.apply(Action::ToggleFold);
    assert_eq!(
        rows(&app),
        [FAILED.trim_end(), EXCEPTION.trim_end(), FRAME.trim_end()]
    );
}

#[test]
fn new_filters_select_the_first_row() {
    let mut app = logback(&[STARTED, FAILED, DONE]);
    app.resize(10);
    app.apply(Action::Bottom);
    app.apply(Action::Up);
    let mut filters = Filters::default();
    filters.levels.set(Level::Info, false);
    let app = filtered(app, filters);
    assert_eq!(rows(&app), [FAILED.trim_end(), DONE.trim_end()]);
    assert_eq!(app.viewport().selected(), Some(0));
}

#[test]
fn the_last_line_number_counts_entries_the_filters_hide() {
    let app = filtered(
        logback(&[STARTED, FAILED, EXCEPTION, FRAME, DONE]),
        text_filter("Started"),
    );
    assert_eq!(app.last_line_number(), Some(5));
}

#[test]
fn appended_entries_are_filtered_while_following() {
    let app = filtered(
        followed(built_in("logback"), &[STARTED, FAILED].concat()),
        text_filter("boom"),
    );
    assert_eq!(app.viewport().total(), 0);
    // The exception continues the last entry, which matches now; the next doesn't.
    let mut app = grown(app, &[STARTED, FAILED, EXCEPTION, DONE].concat());
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), [FAILED.trim_end()]);
    assert_eq!(app.viewport().selected(), Some(0));
    assert!(app.following());
}

#[test]
fn a_partial_line_that_joins_the_entry_before_is_filtered_with_it() {
    // Cut short, the last line looks like the start of an entry; written out, it
    // continues the one before.
    let partial = "2026-10-07 12:00:02.000 [main] INFO  com.example.App -";
    let app = filtered(
        followed(built_in("logback"), &[FAILED, partial].concat()),
        text_filter("boom"),
    );
    let mut app = grown(app, &[FAILED, partial, "boom\n"].concat());
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), [FAILED.trim_end()]);
}

#[test]
fn a_truncated_file_is_filtered_from_its_start() {
    let app = filtered(
        followed(built_in("logback"), &[STARTED, FAILED, DONE].concat()),
        text_filter("Failed"),
    );
    let mut app = truncated(app, &[DONE, FAILED].concat());
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), ["── file truncated ──", FAILED.trim_end()]);
}

/// Filters `log`, in a time-only preset, by a time range that holds all of it, and
/// returns how many rows pass and whether the filters were seen to catch up with the
/// undated entries while indexing went on.
fn filter_while_indexing(log: &str) -> (usize, bool) {
    let mut app = unindexed_app(built_in("logback-basic"), log.as_bytes());
    app.set_filters(Filters {
        time: Some(TimeRange {
            start: "2026-10-01T00:00:00Z".parse().unwrap(),
            end: None,
        }),
        ..Filters::default()
    });
    let deadline = Instant::now() + LARGE_INDEX_DEADLINE;
    let mut caught_up_while_indexing = false;
    loop {
        app.poll_index();
        let progress = app.progress();
        if progress.is_complete() && !app.filtering() {
            return (app.viewport().total(), caught_up_while_indexing);
        }
        caught_up_while_indexing |=
            !progress.is_complete() && progress.indexed > 0 && !app.filtering();
        assert!(Instant::now() < deadline, "{progress:?}");
        // Polling without a pause gives the most chances to see the filters catch up.
        thread::yield_now();
    }
}

#[test]
fn time_only_entries_are_filtered_by_their_dates_once_indexed() {
    // Some 2 MiB, indexed in many chunks: the entries come in undated, and only the
    // complete index dates them. Whether the filters catch up with them before that
    // depends on the threads, so it is tried a few times.
    let n = 38_000;
    let log: String = (0..n)
        .map(|i| format!("12:00:00.000 [main] INFO  com.example.App - step {i}\n"))
        .collect();
    let mut caught_up = false;
    for _ in 0..10 {
        let (total, caught_up_this_time) = filter_while_indexing(&log);
        assert_eq!(total, n);
        caught_up = caught_up_this_time;
        if caught_up {
            break;
        }
    }
    assert!(caught_up, "the filters never caught up while indexing");
}

#[test]
fn growth_keeps_the_selected_match_while_paused() {
    // More entries than the engine evaluates in one chunk.
    let mut app = filtered(app(numbered_log(2_000).as_bytes()), text_filter("step"));
    app.resize(10);
    app.apply(Action::Bottom);
    app.apply(Action::Up);
    let mut app = grown(app, &numbered_log(2_001));
    assert_eq!(app.viewport().selected(), Some(1_998));
    wait_for_filter(&mut app);
    assert_eq!(app.viewport().total(), 2_001);
    assert_eq!(app.viewport().selected(), Some(1_998));
    assert_eq!(app.line(1_998), numbered_log(1_999).lines().last().unwrap());
}

/// Two entries with continuation lines between INFO entries, so that hiding INFO
/// leaves them.
const TWO_TRACES: [&str; 6] = [STARTED, FAILED, EXCEPTION, FRAME, DONE, FRAME];

fn without_info() -> Filters {
    let mut filters = Filters::default();
    filters.levels.set(Level::Info, false);
    filters
}

/// The rows of [`TWO_TRACES`] without INFO, every entry unfolded.
fn unfolded_traces() -> [&'static str; 5] {
    [FAILED, EXCEPTION, FRAME, DONE, FRAME].map(str::trim_end)
}

#[test]
fn an_unfolded_match_stays_unfolded_under_new_filters() {
    let mut app = filtered(logback(&TWO_TRACES), text_filter("Failed"));
    app.resize(10);
    app.apply(Action::ToggleFold);
    let app = filtered(app, without_info());
    assert_eq!(
        rows(&app),
        [FAILED, EXCEPTION, FRAME, DONE].map(str::trim_end)
    );
}

#[test]
fn unfolding_all_while_filtered_unfolds_the_matches_of_new_filters() {
    let mut app = filtered(logback(&TWO_TRACES), text_filter("Started"));
    app.resize(10);
    app.apply(Action::UnfoldAll);
    let app = filtered(app, without_info());
    assert_eq!(rows(&app), unfolded_traces());
}

#[test]
fn unfolding_all_while_the_tail_is_shown_unfolds_the_matches() {
    let mut app = following_app(built_in("logback"), TWO_TRACES.concat().as_bytes());
    app.set_filters(without_info());
    app.resize(10);
    app.apply(Action::UnfoldAll);
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), unfolded_traces());
}

#[test]
fn another_preset_is_filtered_anew() {
    // Under Raw the whole line is the message; under Logback `INFO` is the level.
    let mut app = filtered(
        app_in(built_in("raw"), STARTED.as_bytes()),
        text_filter("INFO"),
    );
    assert_eq!(rows(&app), [STARTED.trim_end()]);
    app.apply(Action::CyclePreset);
    wait_for_filter(&mut app);
    assert_eq!(app.settings().format, built_in("logback"));
    assert!(app.nothing_matches());
}

#[test]
fn an_empty_file_is_no_case_of_nothing_matching() {
    let app = filtered(named_app("/var/log/empty.log", b""), text_filter("boom"));
    assert!(!app.nothing_matches());
}

#[test]
fn nothing_matches_only_once_the_index_and_the_results_are_complete() {
    let mut app = unindexed_app(built_in("raw"), numbered_log(5_000).as_bytes());
    app.set_filters(text_filter("boom"));
    let deadline = Instant::now() + LARGE_INDEX_DEADLINE;
    loop {
        let done = app.progress().is_complete() && !app.filtering();
        assert_eq!(app.nothing_matches(), done, "{:?}", app.progress());
        if done {
            break;
        }
        assert!(Instant::now() < deadline);
        app.poll_index();
        thread::sleep(Duration::from_millis(1));
    }
}

// --- level filter ---

/// Every level shown but `hidden`.
fn all_but(hidden: Level) -> Levels {
    let mut levels = Levels::default();
    levels.toggle(hidden);
    levels
}

/// `STARTED`, `FAILED` and `DONE` filtered to the messages with an `a`: `Started`
/// and `Failed`.
fn with_an_a() -> App {
    filtered(logback(&[STARTED, FAILED, DONE]), text_filter("a"))
}

/// The open level panel of `app`; fails the test if it is closed.
fn panel(app: &App) -> LevelPanel {
    match app.panel() {
        Some(&Panel::Levels(panel)) => panel,
        other => panic!("the level panel is not open: {other:?}"),
    }
}

#[test]
fn level_key_hides_that_level_on_top_of_the_other_filters() {
    let mut app = applied(with_an_a(), &[Action::ToggleLevel(Level::Error)]);
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), [STARTED.trim_end()]);
}

#[test]
fn level_panel_filters_an_entry_written_with_an_alias_by_its_level() {
    let severe = "2026-10-07 12:00:01.500 [main] SEVERE com.example.App - Failed\n";
    let mut app = applied(
        logback(&[STARTED, severe]),
        &[
            Action::LevelPanel,
            Action::ToggleLevel(Level::Info),
            Action::Confirm,
        ],
    );
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), [severe.trim_end()]);
}

#[test]
fn level_panel_opens_with_the_current_levels_and_the_cursor_on_trace() {
    let app = applied(
        logback(&[STARTED]),
        &[Action::ToggleLevel(Level::Error), Action::LevelPanel],
    );
    assert_eq!(
        panel(&app),
        LevelPanel {
            levels: all_but(Level::Error),
            cursor: Level::Trace,
        }
    );
}

#[test]
fn panel_cursor_moves_between_trace_and_fatal() {
    let mut app = applied(logback(&[STARTED]), &[Action::LevelPanel, Action::Up]);
    assert_eq!(panel(&app).cursor, Level::Trace);
    for _ in 0..7 {
        app.apply(Action::Down);
    }
    assert_eq!(panel(&app).cursor, Level::Fatal);
    app.apply(Action::Up);
    assert_eq!(panel(&app).cursor, Level::Error);
}

#[test]
fn moves_while_the_panel_is_open_keep_the_selected_row() {
    let mut app = logback(&[STARTED, FAILED, DONE]);
    app.resize(10);
    let app = applied(app, &[Action::LevelPanel, Action::Down, Action::Bottom]);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn space_in_the_panel_toggles_the_level_under_the_cursor_in_the_panel_only() {
    let app = applied(
        logback(&[STARTED]),
        &[Action::LevelPanel, Action::Down, Action::ToggleFold],
    );
    assert_eq!(panel(&app).levels, all_but(Level::Debug));
    assert_eq!(app.filters().levels, Levels::default());
}

#[test]
fn level_key_in_the_panel_toggles_that_level_in_the_panel_only() {
    let app = applied(
        logback(&[STARTED]),
        &[Action::LevelPanel, Action::ToggleLevel(Level::Warn)],
    );
    assert_eq!(panel(&app).levels, all_but(Level::Warn));
    assert_eq!(app.filters().levels, Levels::default());
}

#[test]
fn confirm_applies_the_panel_levels_on_top_of_the_other_filters_and_closes_it() {
    let mut app = applied(
        with_an_a(),
        &[
            Action::LevelPanel,
            Action::ToggleLevel(Level::Info),
            Action::Confirm,
        ],
    );
    wait_for_filter(&mut app);
    assert_eq!(app.panel(), None);
    assert_eq!(rows(&app), [FAILED.trim_end()]);
}

#[test]
fn cancel_closes_the_panel_and_keeps_the_filters() {
    let app = applied(
        logback(&[STARTED]),
        &[
            Action::LevelPanel,
            Action::ToggleLevel(Level::Info),
            Action::Cancel,
        ],
    );
    assert_eq!(app.panel(), None);
    assert_eq!(app.filters().levels, Levels::default());
}

#[test]
fn quit_quits_while_the_panel_is_open() {
    let app = applied(logback(&[STARTED]), &[Action::LevelPanel, Action::Quit]);
    assert!(app.should_quit());
}

// --- class filter ---

const ORDER: &str = "2026-10-07 12:00:00.000 [main] INFO  com.myco.order.OrderService - created\n";
const PAY: &str = "2026-10-07 12:00:01.000 [main] ERROR com.myco.pay.PayClient - failed\n";
const SQL: &str = "2026-10-07 12:00:02.000 [main] DEBUG org.hibernate.SQL - select\n";

/// `ORDER`, `PAY` and `SQL`, with `rules` as the class filter and its results in.
fn with_classes(rules: &[&str]) -> App {
    let filters = Filters {
        classes: Classes::parse(rules.iter().copied()).unwrap(),
        ..Filters::default()
    };
    filtered(logback(&[ORDER, PAY, SQL]), filters)
}

/// `app` after pressing `keys`.
fn pressed(mut app: App, keys: &[KeyEvent]) -> App {
    for &key in keys {
        app.key(key);
    }
    app
}

/// The keys that type `text`.
fn typing(text: &str) -> Vec<KeyEvent> {
    text.chars().map(char_key).collect()
}

/// `app` with the class panel opened and `keys` pressed in it.
fn in_class_panel(app: App, keys: &[KeyEvent]) -> App {
    let app = pressed(app, &[key(KeyCode::F(6))]);
    pressed(app, keys)
}

/// The open class panel of `app`; fails the test if it is closed.
fn class_panel(app: &App) -> &ClassPanel {
    match app.panel() {
        Some(Panel::Classes(panel)) => panel,
        other => panic!("the class panel is not open: {other:?}"),
    }
}

/// The rows of `app` as the lines of the file they are, line endings included.
fn rows_of(app: &App) -> Vec<String> {
    rows(app).into_iter().map(|row| row + "\n").collect()
}

#[test]
fn only_class_replaces_the_includes_with_the_selected_class_and_keeps_the_excludes() {
    let app = applied(
        with_classes(&["com.myco.*", "!PayClient"]),
        &[Action::OnlyClass],
    );
    assert_eq!(
        app.filters().classes,
        Classes {
            include: vec![Rule::Prefix("com.myco.order.OrderService".to_string())],
            exclude: vec![Rule::Substring("PayClient".to_string())],
        }
    );
}

#[test]
fn hide_class_adds_an_exclude_for_the_selected_class() {
    let mut app = applied(with_classes(&[]), &[Action::Down, Action::HideClass]);
    wait_for_filter(&mut app);
    assert_eq!(
        app.filters().classes.exclude,
        [Rule::Prefix("com.myco.pay.PayClient".to_string())]
    );
    assert_eq!(rows_of(&app), [ORDER, SQL]);
}

#[test]
fn on_an_entry_without_a_class_only_and_hide_class_change_nothing_but_say_so() {
    for action in [Action::OnlyClass, Action::HideClass] {
        let app = applied(hundred_lines(), &[action]);
        assert_eq!(app.filters().classes, Classes::default(), "{action:?}");
        assert_eq!(
            app.notice(),
            Some("the selected entry has no logger"),
            "{action:?}"
        );
    }
}

#[test]
fn a_notice_goes_away_with_the_next_key() {
    let app = pressed(hundred_lines(), &[char_key('c'), char_key('j')]);
    assert_eq!(app.notice(), None);
}

#[test]
fn keys_outside_a_text_panel_do_their_action() {
    let app = pressed(hundred_lines(), &[char_key('j'), char_key('q')]);
    assert_eq!(pos(&app), (0, Some(1)));
    assert!(app.should_quit());
}

#[test]
fn class_panel_opens_with_the_rules_one_per_line_and_an_empty_line_to_type_in() {
    let app = in_class_panel(
        with_classes(&["!org.hibernate", "com.myco.*", "Order"]),
        &[],
    );
    assert_eq!(
        *class_panel(&app),
        ClassPanel {
            lines: vec![
                "com.myco.*".to_string(),
                "Order".to_string(),
                "!org.hibernate".to_string(),
                String::new(),
            ],
            cursor: 3,
            error: None,
        }
    );
}

#[test]
fn characters_typed_in_the_class_panel_go_into_its_line_not_to_their_actions() {
    let mut keys = typing("qjG1c");
    keys.push(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT));
    let app = in_class_panel(hundred_lines(), &keys);
    assert_eq!(class_panel(&app).lines, ["qjG1cC"]);
    assert!(!app.should_quit());
    assert_eq!(pos(&app), (0, Some(0)));
    assert_eq!(app.filters().levels, Levels::default());
    assert_eq!(app.filters().classes, Classes::default());
}

#[test]
fn ctrl_c_quits_while_the_class_panel_is_open() {
    let app = in_class_panel(hundred_lines(), &[ctrl('c')]);
    assert!(app.should_quit());
}

#[test]
fn backspace_deletes_the_last_character_then_the_empty_line() {
    let backspace = key(KeyCode::Backspace);
    let mut keys = typing("ab");
    keys.push(key(KeyCode::Down));
    keys.extend(typing("c"));
    keys.extend([backspace, backspace, backspace]);
    let app = in_class_panel(hundred_lines(), &keys);
    let panel = class_panel(&app);
    assert_eq!(
        (panel.lines.as_slice(), panel.cursor),
        (&["a".to_string()][..], 0)
    );
}

#[test]
fn backspace_on_the_only_line_when_empty_does_nothing() {
    let app = in_class_panel(hundred_lines(), &[key(KeyCode::Backspace)]);
    let panel = class_panel(&app);
    assert_eq!(
        (panel.lines.as_slice(), panel.cursor),
        (&[String::new()][..], 0)
    );
}

#[test]
fn down_on_the_last_line_adds_an_empty_line_unless_it_is_empty() {
    let down = key(KeyCode::Down);
    let mut keys = vec![down];
    keys.extend(typing("a"));
    keys.extend([down, down]);
    let app = in_class_panel(hundred_lines(), &keys);
    let panel = class_panel(&app);
    assert_eq!(panel.lines, ["a", ""]);
    assert_eq!(panel.cursor, 1);
}

#[test]
fn up_and_down_move_the_cursor_between_the_lines_stopping_at_the_top() {
    let up = key(KeyCode::Up);
    let mut keys = vec![up, up, up];
    keys.extend(typing("x"));
    keys.push(key(KeyCode::Down));
    keys.extend(typing("y"));
    let app = in_class_panel(with_classes(&["a", "b"]), &keys);
    assert_eq!(class_panel(&app).lines, ["ax", "by", ""]);
}

#[test]
fn enter_applies_the_class_panel_rules_on_top_of_the_other_filters_and_closes_it() {
    let mut levels = Levels::default();
    levels.toggle(Level::Error);
    let app = filtered(
        logback(&[ORDER, PAY, SQL]),
        Filters {
            levels,
            ..Filters::default()
        },
    );
    let mut keys = typing("com.myco.*");
    keys.push(key(KeyCode::Enter));
    let mut app = in_class_panel(app, &keys);
    wait_for_filter(&mut app);
    assert_eq!(app.panel(), None);
    assert_eq!(rows_of(&app), [ORDER]);
}

#[test]
fn esc_closes_the_class_panel_without_applying_it() {
    let mut keys = typing("com.myco.*");
    keys.push(key(KeyCode::Esc));
    let app = in_class_panel(hundred_lines(), &keys);
    assert_eq!(app.panel(), None);
    assert_eq!(app.filters().classes, Classes::default());
}

#[test]
fn rules_that_do_not_parse_keep_the_class_panel_open_with_the_reason() {
    let mut keys = typing("Order");
    keys.push(key(KeyCode::Down));
    keys.extend(typing("!"));
    keys.push(key(KeyCode::Enter));
    let app = in_class_panel(hundred_lines(), &keys);
    assert_eq!(
        class_panel(&app).error,
        Some(Classes::parse(["Order", "!"]).unwrap_err())
    );
    assert_eq!(app.filters().classes, Classes::default());
}

// --- time filter ---

/// A Logback entry at `time` on 2026-10-07 that says when it is.
fn at(time: &str) -> String {
    on("2026-10-07", time)
}

/// A Logback entry at `time` on `date` that says when it is.
fn on(date: &str, time: &str) -> String {
    format!("{date} {time} [main] INFO  com.example.App - at {time}\n")
}

/// A Logback log of an entry at each of `times`, in that order.
fn log_at<S: AsRef<str>>(times: &[S]) -> String {
    times.iter().map(|time| at(time.as_ref())).collect()
}

/// The rows of the entries at `times`.
fn rows_at<S: AsRef<str>>(times: &[S]) -> Vec<String> {
    times
        .iter()
        .map(|time| at(time.as_ref()).trim_end().to_string())
        .collect()
}

/// The time filter `spec` counted from `base`.
fn since(spec: &str, base: Base) -> Option<TimeFilter> {
    Some(TimeFilter {
        spec: TimeSpec::parse(spec).unwrap(),
        base,
    })
}

/// `app` with the time filter `spec`, counted from the file, and its results in.
fn timed(mut app: App, spec: &str) -> App {
    app.set_time_filter(since(spec, Base::File));
    wait_for_filter(&mut app);
    app
}

/// `app` with the time panel opened and `keys` pressed in it.
fn in_time_panel(app: App, keys: &[KeyEvent]) -> App {
    let app = pressed(app, &[key(KeyCode::F(5))]);
    pressed(app, keys)
}

/// The open time panel of `app`; fails the test if it is closed.
fn time_panel(app: &App) -> &TimePanel {
    match app.panel() {
        Some(Panel::Time(panel)) => panel,
        other => panic!("the time panel is not open: {other:?}"),
    }
}

#[test]
fn a_time_filter_shows_only_the_entries_from_its_start_to_the_end_of_its_last_minute() {
    let log = log_at(&[
        "11:59:59.999",
        "12:00:00.000",
        "12:30:59.999",
        "12:31:00.000",
        "13:00:00.000",
    ]);
    let app = timed(app_in(built_in("logback"), log.as_bytes()), "12:00-12:30");
    assert_eq!(rows(&app), rows_at(&["12:00:00.000", "12:30:59.999"]));
}

#[test]
fn a_range_entirely_outside_the_file_matches_nothing() {
    let log = log_at(&["12:00:00.000", "13:00:00.000"]);
    for spec in ["2020-01-01 12:00..2020-01-01 13:00", "03:00-04:00"] {
        let app = timed(app_in(built_in("logback"), log.as_bytes()), spec);
        assert!(app.nothing_matches(), "{spec}");
    }
}

#[test]
fn the_last_n_slides_as_entries_are_appended() {
    let app = timed(
        followed(
            built_in("logback"),
            &log_at(&["12:00:00.000", "12:00:30.000"]),
        ),
        "1m",
    );
    assert_eq!(rows(&app), rows_at(&["12:00:00.000", "12:00:30.000"]));
    let mut app = grown(
        app,
        &log_at(&["12:00:00.000", "12:00:30.000", "12:01:10.000"]),
    );
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), rows_at(&["12:00:30.000", "12:01:10.000"]));
    assert!(app.following());
}

/// `n` times a millisecond apart from 12:00:`second` on.
fn millis_from(second: usize, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("12:00:{:02}.{:03}", second + i / 1000, i % 1000))
        .collect()
}

/// 3,000 entries within the minute before the last one, 12:00:59: the first at
/// 12:00:10, and one at 12:00:05 ten entries before the last, out of order.
fn long_window() -> Vec<String> {
    let mut times = vec!["12:00:10.000".to_string()];
    times.extend(millis_from(30, 2_989));
    times.push("12:00:05.000".to_string());
    times.extend(millis_from(40, 8));
    times.push("12:00:59.000".to_string());
    times
}

/// `times` and then `next`.
fn and_then(times: &[String], next: &str) -> Vec<String> {
    let mut times = times.to_vec();
    times.push(next.to_string());
    times
}

/// `app` right after its file grew to `content`, before it polls anything.
fn grown_unpolled(mut app: App, content: &str) -> App {
    app.file_changed(FileChange::Grew(test_support::source(content.as_bytes())));
    app
}

#[test]
fn the_last_n_drops_a_match_that_falls_out_of_it_well_before_the_appended_entries() {
    let times = long_window();
    let app = timed(followed(built_in("logback"), &log_at(&times)), "1m");
    // The window now starts at 12:00:10.
    let mut app = grown(app, &log_at(&and_then(&times, "12:01:10.000")));
    wait_for_filter(&mut app);
    let mut shown = times.clone();
    shown.retain(|time| time != "12:00:05.000");
    shown.push("12:01:10.000".to_string());
    assert_eq!(rows(&app), rows_at(&shown));
}

#[test]
fn a_slide_keeps_showing_the_matches_before_the_first_one_that_drops_out() {
    let times = long_window();
    let app = timed(followed(built_in("logback"), &log_at(&times)), "1m");
    let app = grown_unpolled(app, &log_at(&and_then(&times, "12:01:10.000")));
    let shown = app.viewport().total();
    assert!(shown >= 2_990, "{shown} rows");
}

#[test]
fn a_slide_that_drops_nothing_keeps_every_row_shown() {
    let times = millis_from(30, 3_000);
    let app = timed(followed(built_in("logback"), &log_at(&times)), "1m");
    // The window moves from 11:59:32.999 to 12:00:00 on.
    let app = grown_unpolled(app, &log_at(&and_then(&times, "12:01:00.000")));
    let shown = app.viewport().total();
    assert!(shown >= 3_000, "{shown} rows");
}

#[test]
fn a_window_whose_start_moves_earlier_takes_in_the_entries_it_reaches_again() {
    let times = [
        "11:59:30.000",
        "12:00:30.000",
        "12:00:40.000",
        "12:01:00.000",
    ];
    let app = timed(followed(built_in("logback"), &log_at(&times)), "1m");
    assert_eq!(rows(&app), rows_at(&times[1..]));
    // The last entry, out of order, moves the start back to 11:59:20.
    let times = and_then(&times.map(String::from), "12:00:20.000");
    let mut app = grown(app, &log_at(&times));
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), rows_at(&times));
}

#[test]
fn a_range_without_a_date_moves_to_the_date_of_a_new_last_entry() {
    let before = [
        on("2026-10-08", "12:10:00.000"),
        on("2026-10-08", "12:15:00.000"),
        on("2026-10-07", "12:10:00.000"),
        on("2026-10-07", "12:20:00.000"),
    ];
    let app = timed(
        followed(built_in("logback"), &before.concat()),
        "12:00-12:30",
    );
    assert_eq!(rows_of(&app), before[2..]);
    let next = on("2026-10-08", "12:25:00.000");
    let mut app = grown(app, &[before.concat(), next.clone()].concat());
    wait_for_filter(&mut app);
    assert_eq!(rows_of(&app), [before[0].clone(), before[1].clone(), next]);
}

/// The wall clock of [`the_last_n_on_the_wall_clock_follows_it_as_entries_are_appended`],
/// in seconds since the epoch.
static WALL_CLOCK: AtomicI64 = AtomicI64::new(0);

fn wall_clock() -> Timestamp {
    Timestamp::from_second(WALL_CLOCK.load(Ordering::SeqCst)).unwrap()
}

fn set_wall_clock(at: &str) {
    let at: Timestamp = at.parse().unwrap();
    WALL_CLOCK.store(at.as_second(), Ordering::SeqCst);
}

#[test]
fn the_last_n_on_the_wall_clock_follows_it_as_entries_are_appended() {
    set_wall_clock("2026-10-07T12:00:50Z");
    let mut app = followed(
        built_in("logback"),
        &log_at(&["12:00:00.000", "12:00:40.000"]),
    );
    app.set_clock(wall_clock);
    app.set_time_filter(since("1m", Base::Now));
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), rows_at(&["12:00:00.000", "12:00:40.000"]));
    set_wall_clock("2026-10-07T12:01:30Z");
    let mut app = grown(
        app,
        &log_at(&["12:00:00.000", "12:00:40.000", "12:01:20.000"]),
    );
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), rows_at(&["12:00:40.000", "12:01:20.000"]));
}

#[test]
fn a_time_filter_on_a_log_without_timestamps_matches_nothing() {
    let app = timed(hundred_lines(), "10m");
    assert!(app.nothing_matches());
}

#[test]
fn entries_out_of_order_at_the_window_edge_pass_by_their_own_timestamps() {
    // The window counts back from the last entry, 12:00:00, not from the latest
    // timestamp, 12:00:01.
    let log = log_at(&[
        "11:49:59.000",
        "11:50:00.500",
        "11:49:59.500",
        "12:00:01.000",
        "12:00:00.000",
    ]);
    let app = timed(app_in(built_in("logback"), log.as_bytes()), "10m");
    assert_eq!(
        rows(&app),
        rows_at(&["11:50:00.500", "12:00:01.000", "12:00:00.000"])
    );
}

#[test]
fn a_time_filter_set_while_opening_counts_back_from_the_last_entry_once_indexed() {
    // Time-only: the entries get their dates once the index is complete.
    let log = "11:00:00.000 [main] INFO  com.example.App - a\n\
               11:59:30.000 [main] INFO  com.example.App - b\n\
               12:00:00.000 [main] INFO  com.example.App - c\n";
    let mut app = unindexed_app(built_in("logback-basic"), log.as_bytes());
    app.set_time_filter(since("1m", Base::File));
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), log.lines().skip(1).collect::<Vec<_>>());
}

#[test]
fn the_time_filter_and_the_level_filter_apply_together() {
    let error = "2026-10-07 11:59:50.000 [main] ERROR com.example.App - boom\n";
    let log = [at("11:00:00.000"), error.to_string(), at("12:00:00.000")].concat();
    let app = applied(
        app_in(built_in("logback"), log.as_bytes()),
        &[Action::ToggleLevel(Level::Info)],
    );
    let app = timed(app, "1m");
    assert_eq!(rows(&app), [error.trim_end()]);
    let mut app = applied(app, &[Action::ToggleLevel(Level::Info)]);
    wait_for_filter(&mut app);
    assert_eq!(
        rows(&app),
        [error.trim_end(), at("12:00:00.000").trim_end()]
    );
}

#[test]
fn f5_opens_the_time_panel() {
    assert_eq!(action(key(KeyCode::F(5))), Some(Action::TimePanel));
}

#[test]
fn time_panel_opens_with_the_current_spec_and_base() {
    let mut app = hundred_lines();
    app.set_time_filter(since("12:00 - 12:30", Base::Now));
    let app = in_time_panel(app, &[]);
    assert_eq!(
        *time_panel(&app),
        TimePanel {
            spec: "12:00-12:30".to_string(),
            base: Base::Now,
            error: None,
        }
    );
}

#[test]
fn characters_typed_in_the_time_panel_go_into_its_spec_not_to_their_actions() {
    let app = in_time_panel(hundred_lines(), &typing("q1jG"));
    assert_eq!(time_panel(&app).spec, "q1jG");
    assert!(!app.should_quit());
    assert_eq!(pos(&app), (0, Some(0)));
    assert_eq!(app.filters().levels, Levels::default());
}

#[test]
fn backspace_in_the_time_panel_deletes_the_last_character_while_there_is_one() {
    let mut keys = typing("10m");
    keys.extend([key(KeyCode::Backspace); 4]);
    keys.extend(typing("5"));
    let app = in_time_panel(hundred_lines(), &keys);
    assert_eq!(time_panel(&app).spec, "5");
}

#[test]
fn tab_in_the_time_panel_toggles_the_base() {
    let tab = key(KeyCode::Tab);
    let app = in_time_panel(hundred_lines(), &[tab]);
    assert_eq!(time_panel(&app).base, Base::Now);
    let app = pressed(app, &[tab]);
    assert_eq!(time_panel(&app).base, Base::File);
}

#[test]
fn down_and_up_in_the_time_panel_step_through_the_presets_stopping_at_the_ends() {
    let mut app = in_time_panel(hundred_lines(), &[]);
    let mut specs = Vec::new();
    for code in [[KeyCode::Down; 7].as_slice(), &[KeyCode::Up]].concat() {
        app.key(key(code));
        specs.push(time_panel(&app).spec.clone());
    }
    assert_eq!(
        specs,
        ["1m", "5m", "10m", "30m", "60m", "24h", "24h", "60m"]
    );
}

#[test]
fn enter_applies_the_time_panel_spec_and_base_and_closes_it() {
    let log = log_at(&["11:59:00.000", "11:59:45.000", "12:00:00.000"]);
    let mut app = app_in(built_in("logback"), log.as_bytes());
    app.set_clock(|| "2026-10-07T12:00:30Z".parse().unwrap());
    let mut keys = typing("1m");
    keys.extend([key(KeyCode::Tab), key(KeyCode::Enter)]);
    let mut app = in_time_panel(app, &keys);
    wait_for_filter(&mut app);
    assert_eq!(app.panel(), None);
    assert_eq!(app.time_filter(), since("1m", Base::Now).as_ref());
    assert_eq!(rows(&app), rows_at(&["11:59:45.000", "12:00:00.000"]));
}

#[test]
fn enter_on_an_empty_time_panel_clears_the_time_filter() {
    let times = ["11:00:00.000", "12:00:00.000"];
    let app = timed(
        app_in(built_in("logback"), log_at(&times).as_bytes()),
        "12:00-",
    );
    let mut keys = vec![key(KeyCode::Backspace); "12:00-".len()];
    keys.push(key(KeyCode::Enter));
    let mut app = in_time_panel(app, &keys);
    wait_for_filter(&mut app);
    assert_eq!(app.time_filter(), None);
    assert_eq!(rows(&app), rows_at(&times));
}

#[test]
fn esc_closes_the_time_panel_without_applying_it() {
    let mut keys = typing("10m");
    keys.push(key(KeyCode::Esc));
    let app = in_time_panel(hundred_lines(), &keys);
    assert_eq!(app.panel(), None);
    assert_eq!(app.time_filter(), None);
}

#[test]
fn a_spec_that_does_not_parse_keeps_the_time_panel_open_with_the_reason() {
    let mut keys = typing("soon");
    keys.push(key(KeyCode::Enter));
    let app = in_time_panel(hundred_lines(), &keys);
    assert_eq!(
        time_panel(&app).error,
        Some(TimeSpec::parse("soon").unwrap_err())
    );
    assert_eq!(app.time_filter(), None);
}

#[test]
fn ctrl_c_quits_while_the_time_panel_is_open() {
    let app = in_time_panel(hundred_lines(), &[]);
    time_panel(&app);
    let app = pressed(app, &[ctrl('c')]);
    assert!(app.should_quit());
}

// --- text filter ---

const REGEX: matcher::Options = matcher::Options {
    regex: true,
    ignore_case: false,
};

/// `app` with the text filter prompt opened and `keys` pressed in it.
fn in_text_prompt(app: App, keys: &[KeyEvent]) -> App {
    let app = pressed(app, &[key(KeyCode::F(4))]);
    pressed(app, keys)
}

/// The open text filter prompt of `app`; fails the test if it is closed.
fn text_prompt(app: &App) -> &Prompt {
    match app.panel() {
        Some(Panel::Text(prompt)) => prompt,
        other => panic!("the text filter prompt is not open: {other:?}"),
    }
}

/// The pattern and options of `app`'s text filter, if it has one.
fn text_of(app: &App) -> Option<(String, matcher::Options)> {
    let text = app.filters().text.as_ref()?;
    Some((text.pattern().to_string(), text.options()))
}

#[test]
fn text_prompt_opens_with_the_current_pattern_and_options() {
    let options = matcher::Options {
        regex: true,
        ignore_case: true,
    };
    let mut app = hundred_lines();
    app.set_filters(Filters {
        text: Some(Matcher::new("Fail|Done", options).unwrap()),
        ..Filters::default()
    });
    let app = in_text_prompt(app, &[]);
    assert_eq!(
        *text_prompt(&app),
        Prompt {
            pattern: "Fail|Done".to_string(),
            options,
            error: None,
        }
    );
}

#[test]
fn characters_typed_in_the_text_prompt_go_into_its_pattern_not_to_their_actions() {
    let app = in_text_prompt(hundred_lines(), &typing("q1x&G"));
    assert_eq!(text_prompt(&app).pattern, "q1x&G");
    assert!(!app.should_quit());
    assert_eq!(pos(&app), (0, Some(0)));
    assert_eq!(app.filters().levels, Levels::default());
}

#[test]
fn backspace_in_the_text_prompt_deletes_the_last_character_while_there_is_one() {
    let mut keys = typing("ab");
    keys.extend([key(KeyCode::Backspace); 3]);
    keys.extend(typing("c"));
    let app = in_text_prompt(hundred_lines(), &keys);
    assert_eq!(text_prompt(&app).pattern, "c");
}

#[test]
fn ctrl_r_in_the_text_prompt_toggles_regex() {
    let app = in_text_prompt(hundred_lines(), &[ctrl('r')]);
    assert_eq!(text_prompt(&app).options, REGEX);
    assert_eq!(text_prompt(&app).pattern, "");
    let app = pressed(app, &[ctrl('r')]);
    assert_eq!(text_prompt(&app).options, matcher::Options::default());
}

#[test]
fn tab_and_ctrl_i_in_the_text_prompt_toggle_ignore_case() {
    let app = in_text_prompt(hundred_lines(), &[key(KeyCode::Tab)]);
    assert!(text_prompt(&app).options.ignore_case);
    let app = pressed(app, &[ctrl('i')]);
    assert!(!text_prompt(&app).options.ignore_case);
}

#[test]
fn enter_applies_the_text_prompt_on_top_of_the_other_filters_and_closes_it() {
    let app = applied(
        logback(&[STARTED, FAILED, DONE]),
        &[Action::ToggleLevel(Level::Warn)],
    );
    let mut keys = vec![ctrl('r'), key(KeyCode::Tab)];
    keys.extend(typing("started|done"));
    keys.push(key(KeyCode::Enter));
    let mut app = in_text_prompt(app, &keys);
    wait_for_filter(&mut app);
    assert_eq!(app.panel(), None);
    let options = matcher::Options {
        regex: true,
        ignore_case: true,
    };
    assert_eq!(text_of(&app), Some(("started|done".to_string(), options)));
    // DONE matches too, but its level is hidden.
    assert_eq!(rows(&app), [STARTED.trim_end()]);
}

#[test]
fn enter_on_an_empty_text_prompt_clears_the_text_filter() {
    let app = filtered(logback(&[STARTED, FAILED]), text_filter("Failed"));
    let mut keys = vec![key(KeyCode::Backspace); "Failed".len()];
    keys.push(key(KeyCode::Enter));
    let mut app = in_text_prompt(app, &keys);
    wait_for_filter(&mut app);
    assert_eq!(text_of(&app), None);
    assert_eq!(rows(&app), [STARTED.trim_end(), FAILED.trim_end()]);
}

#[test]
fn an_invalid_regex_keeps_the_text_prompt_open_with_the_reason() {
    let app = in_text_prompt(
        hundred_lines(),
        &[ctrl('r'), char_key('('), key(KeyCode::Enter)],
    );
    assert_eq!(
        text_prompt(&app).error,
        Some(Matcher::new("(", REGEX).unwrap_err())
    );
    assert_eq!(text_of(&app), None);
}

#[test]
fn esc_closes_the_text_prompt_without_applying_it() {
    let app = filtered(logback(&[STARTED, FAILED]), text_filter("Failed"));
    let app = in_text_prompt(app, &[char_key('x'), ctrl('r'), key(KeyCode::Esc)]);
    assert_eq!(app.panel(), None);
    assert_eq!(
        text_of(&app),
        Some(("Failed".to_string(), matcher::Options::default()))
    );
    assert_eq!(rows(&app), [FAILED.trim_end()]);
}

#[test]
fn ctrl_c_quits_while_the_text_prompt_is_open() {
    let app = in_text_prompt(hundred_lines(), &[]);
    text_prompt(&app);
    let app = pressed(app, &[ctrl('c')]);
    assert!(app.should_quit());
}

// --- every filter at once ---

/// A Logback entry on 2026-10-07.
fn entry(time: &str, level: &str, class: &str, message: &str) -> String {
    format!("2026-10-07 {time} [main] {level:<5} {class} - {message}\n")
}

/// `app` with a filter of every kind set, and their results in: levels but INFO,
/// classes `com.myco.*` but `!Sql`, the last 5 minutes of the file and, typed into
/// the text filter prompt, the text `time`.
fn with_every_filter(mut app: App) -> App {
    app.set_filters(Filters {
        levels: all_but(Level::Info),
        classes: Classes::parse(["com.myco.*", "!Sql"]).unwrap(),
        ..Filters::default()
    });
    app.set_time_filter(since("5m", Base::File));
    let mut keys = typing("time");
    keys.push(key(KeyCode::Enter));
    let mut app = in_text_prompt(app, &keys);
    wait_for_filter(&mut app);
    app
}

#[test]
fn every_filter_kind_applies_to_appended_entries_while_following() {
    let first = entry("12:00:00.000", "WARN", "com.myco.pay.PayClient", "timeout");
    let app = with_every_filter(followed(built_in("logback"), &first));
    assert_eq!(rows(&app), [first.trim_end()]);
    let passing = [
        entry(
            "12:06:00.000",
            "WARN",
            "com.myco.pay.PayClient",
            "Retry after timeout",
        ),
        entry(
            "12:06:40.000",
            "ERROR",
            "com.myco.pay.PayClient",
            "Payment timeout",
        ),
    ];
    let appended = [
        passing[0].clone(),
        entry(
            "12:06:10.000",
            "INFO",
            "com.myco.pay.PayClient",
            "Paid after timeout",
        ),
        entry("12:06:20.000", "ERROR", "org.hibernate.SQL", "Lock timeout"),
        entry("12:06:25.000", "ERROR", "com.myco.db.Sql", "Lock timeout"),
        entry(
            "12:06:30.000",
            "ERROR",
            "com.myco.pay.PayClient",
            "Payment failed",
        ),
        passing[1].clone(),
    ];
    // The first entry falls out of the last 5 minutes.
    let mut app = grown(app, &[first, appended.concat()].concat());
    wait_for_filter(&mut app);
    assert_eq!(
        rows(&app),
        passing.map(|entry| entry.trim_end().to_string())
    );
    assert_eq!(app.viewport().selected(), Some(1));
    assert!(app.following());
}

// --- clearing the filters ---

#[test]
fn clearing_the_filters_clears_every_kind_for_good() {
    let first = entry("12:00:00.000", "INFO", "org.example.App", "started");
    let app = with_every_filter(followed(built_in("logback"), &first));
    let mut app = applied(app, &[Action::ClearFilters]);
    assert_eq!(app.filters().levels, Levels::default());
    assert_eq!(app.filters().classes, Classes::default());
    assert_eq!(app.filters().time, None);
    assert_eq!(text_of(&app), None);
    assert_eq!(app.time_filter(), None);
    // A time filter left behind would come back as the last entry moves on.
    let later = entry("13:00:00.000", "DEBUG", "org.example.App", "later");
    app = grown(app, &[first.as_str(), &later].concat());
    wait_for_filter(&mut app);
    assert_eq!(rows(&app), [first.trim_end(), later.trim_end()]);
}

#[test]
fn clearing_the_filters_shows_every_entry_at_once() {
    let app = filtered(logback(&[STARTED, FAILED, DONE]), text_filter("Failed"));
    let app = applied(app, &[Action::ClearFilters]);
    assert!(!app.filtering());
    assert_eq!(
        rows(&app),
        [STARTED.trim_end(), FAILED.trim_end(), DONE.trim_end()]
    );
}

// --- level counters ---

/// The level counters with `counts` for their levels and 0 for the others.
fn counters(counts: &[(Level, u32)]) -> [u32; 6] {
    let mut counters = [0; 6];
    for &(level, count) in counts {
        counters[level as usize] = count;
    }
    counters
}

/// A headless entry, then an entry of INFO, ERROR (with a stack trace) and WARN, and
/// another WARN.
fn counted_log() -> App {
    logback(&[
        "starting up\n",
        STARTED,
        FAILED,
        EXCEPTION,
        FRAME,
        DONE,
        DONE,
    ])
}

#[test]
fn level_counters_count_the_entries_of_each_level_but_not_those_without_one() {
    assert_eq!(
        counted_log().level_counts(),
        counters(&[(Level::Info, 1), (Level::Warn, 2), (Level::Error, 1)])
    );
}

#[test]
fn level_counters_ignore_the_level_filter() {
    let mut app = applied(counted_log(), &[Action::ToggleLevel(Level::Warn)]);
    wait_for_filter(&mut app);
    assert_eq!(
        app.level_counts(),
        counters(&[(Level::Info, 1), (Level::Warn, 2), (Level::Error, 1)])
    );
}

#[test]
fn level_counters_under_only_a_level_filter_are_there_before_the_filter_results() {
    let app = applied(counted_log(), &[Action::ToggleLevel(Level::Warn)]);
    assert_eq!(
        app.level_counts(),
        counters(&[(Level::Info, 1), (Level::Warn, 2), (Level::Error, 1)])
    );
}

#[test]
fn level_counters_count_only_the_entries_that_pass_the_other_filters() {
    let app = filtered(counted_log(), text_filter("Done"));
    assert_eq!(app.level_counts(), counters(&[(Level::Warn, 2)]));
}

#[test]
fn a_last_line_completed_by_growth_is_counted_once() {
    let cut = &FAILED[..FAILED.len() - 3];
    let app = followed(built_in("logback"), &[STARTED, cut].concat());
    let app = grown(app, &[STARTED, FAILED].concat());
    assert_eq!(
        app.level_counts(),
        counters(&[(Level::Info, 1), (Level::Error, 1)])
    );
}

#[test]
fn a_last_line_that_turns_out_to_continue_the_entry_before_is_no_longer_counted() {
    // Cut short, the last line looks like the start of an INFO entry; written out, it
    // continues the one before.
    let partial = "2026-10-07 12:00:02.000 [main] INFO  com.example.App -";
    let app = followed(built_in("logback"), &[FAILED, partial].concat());
    assert_eq!(
        app.level_counts(),
        counters(&[(Level::Info, 1), (Level::Error, 1)])
    );
    let app = grown(app, &[FAILED, partial, "boom\n"].concat());
    assert_eq!(app.level_counts(), counters(&[(Level::Error, 1)]));
}

#[test]
fn level_counters_start_over_with_the_content_of_a_truncated_file() {
    let app = followed(built_in("logback"), &[STARTED, FAILED, DONE].concat());
    let app = truncated(app, DONE);
    assert_eq!(app.level_counts(), counters(&[(Level::Warn, 1)]));
}

#[test]
fn level_counters_after_cycling_the_preset_are_those_of_the_new_preset() {
    // Logback reads the first two lines, Logback basic the last one.
    let log = [
        STARTED,
        FAILED,
        "12:00:03.000 [main] WARN  com.example.App - Done\n",
    ]
    .concat();
    let mut app = cycled(app_in(built_in("logback"), log.as_bytes()));
    wait_for_index(&mut app);
    let fresh = app_in(app.settings().format, log.as_bytes());
    assert_eq!(app.level_counts(), fresh.level_counts());
    assert_eq!(app.level_counts(), counters(&[(Level::Warn, 1)]));
}

#[test]
fn level_counters_under_a_text_filter_take_in_the_matching_appended_entries() {
    let app = filtered(
        followed(built_in("logback"), &[STARTED, DONE].concat()),
        text_filter("Done"),
    );
    assert_eq!(app.level_counts(), counters(&[(Level::Warn, 1)]));
    let mut app = grown(app, &[STARTED, DONE, FAILED, DONE].concat());
    wait_for_filter(&mut app);
    assert_eq!(app.level_counts(), counters(&[(Level::Warn, 2)]));
}

#[test]
fn level_counters_take_in_appended_entries() {
    let app = grown(
        followed(built_in("logback"), STARTED),
        &[STARTED, FAILED, DONE].concat(),
    );
    assert_eq!(
        app.level_counts(),
        counters(&[(Level::Info, 1), (Level::Warn, 1), (Level::Error, 1)])
    );
}

// --- jumps ---

/// `app` after opening the prompt `opener` opens, typing `text` and pressing Enter.
fn jumped(app: App, opener: char, text: &str) -> App {
    let app = pressed(app, &[char_key(opener)]);
    jump_prompt(&app);
    let mut keys = typing(text);
    keys.push(key(KeyCode::Enter));
    pressed(app, &keys)
}

/// The text of the selected row of `app`.
fn selected_row(app: &App) -> String {
    app.line(app.viewport().selected().unwrap()).into_owned()
}

/// The open jump prompt of `app`; fails the test if it is closed.
fn jump_prompt(app: &App) -> &JumpPrompt {
    match app.panel() {
        Some(Panel::Jump(prompt)) => prompt,
        other => panic!("no jump prompt is open: {other:?}"),
    }
}

/// [`STARTED`] on line 1, [`FAILED`] on 2 with its folded stack trace on 3 and 4,
/// [`DONE`] on 5 with its folded stack trace on 6.
fn traces() -> App {
    let mut app = logback(&TWO_TRACES);
    app.resize(10);
    app
}

/// [`traces`] with `level` hidden and the filter's results in.
fn traces_without(level: Level) -> App {
    let mut app = applied(traces(), &[Action::ToggleLevel(level)]);
    wait_for_filter(&mut app);
    app
}

/// A Logback entry whose timestamp can't be read, an hour 99.
const UNDATED: &str = "2026-10-07 99:00:00.000 [main] INFO  com.example.App - undated\n";

#[test]
fn colon_and_t_open_empty_line_and_time_prompts() {
    for (opener, to, opens) in [
        (':', Jump::Line, Action::LinePrompt),
        ('t', Jump::Time, Action::TimePrompt),
    ] {
        assert_eq!(action(char_key(opener)), Some(opens));
        let app = pressed(hundred_lines(), &[char_key(opener)]);
        assert_eq!(
            *jump_prompt(&app),
            JumpPrompt {
                to,
                text: String::new(),
                error: None,
            }
        );
    }
}

#[test]
fn jump_prompt_text_is_typed_and_deleted_not_acted_on() {
    let mut keys = vec![char_key(':')];
    keys.extend(typing("q1jG"));
    keys.extend([key(KeyCode::Backspace); 5]);
    keys.extend(typing("42"));
    let app = pressed(hundred_lines(), &keys);
    assert_eq!(jump_prompt(&app).text, "42");
    assert!(!app.should_quit());
    assert_eq!(pos(&app), (0, Some(0)));
    assert_eq!(app.filters().levels, Levels::default());
}

#[test]
fn esc_closes_a_jump_prompt_without_jumping() {
    let mut keys = vec![char_key(':')];
    keys.extend(typing("50"));
    keys.push(key(KeyCode::Esc));
    let app = pressed(hundred_lines(), &keys);
    assert_eq!(app.panel(), None);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn ctrl_c_quits_while_a_jump_prompt_is_open() {
    let app = pressed(hundred_lines(), &[char_key('t')]);
    jump_prompt(&app);
    let app = pressed(app, &[ctrl('c')]);
    assert!(app.should_quit());
}

#[test]
fn enter_on_a_blank_jump_prompt_closes_it_without_moving() {
    let app = jumped(applied(hundred_lines(), &[Action::Down]), ':', " ");
    assert_eq!(app.panel(), None);
    assert_eq!(pos(&app), (0, Some(1)));
}

#[test]
fn a_jump_stops_following() {
    let mut app = twenty_lines_followed();
    assert!(app.following());
    app = jumped(app, ':', "3");
    assert!(!app.following());
    assert_eq!(app.viewport().selected(), Some(2));
}

#[test]
fn a_jump_in_a_file_without_rows_does_nothing() {
    for (opener, text) in [(':', "1"), ('t', "12:00")] {
        let app = jumped(app(b""), opener, text);
        assert_eq!(app.panel(), None);
        assert_eq!(app.viewport().selected(), None);
    }
}

#[test]
fn line_jump_selects_the_entry_of_that_line() {
    for line in ["5", " 5 "] {
        let app = jumped(traces(), ':', line);
        assert_eq!(app.viewport().selected(), Some(2), "{line:?}");
        assert_eq!(selected_row(&app), DONE.trim_end(), "{line:?}");
    }
}

#[test]
fn line_jump_while_the_tail_is_shown_stays_and_says_so() {
    let mut app = following_app(built_in("logback"), TWO_TRACES.concat().as_bytes());
    app.resize(10);
    let before = pos(&app);
    let app = jumped(app, ':', "1");
    assert_eq!(app.panel(), None);
    assert_eq!(pos(&app), before);
    assert!(app.following());
    assert_eq!(
        app.notice(),
        Some("line numbers are not known until the file is indexed")
    );
}

#[test]
fn line_jump_into_a_folded_stack_trace_selects_its_entry_and_keeps_it_folded() {
    let app = jumped(traces(), ':', "4");
    assert_eq!(selected_row(&app), FAILED.trim_end());
    assert_eq!(app.viewport().total(), 3);
}

#[test]
fn line_jump_into_an_unfolded_stack_trace_selects_that_line() {
    let app = applied(traces(), &[Action::Down, Action::ToggleFold, Action::Top]);
    let app = jumped(app, ':', "4");
    assert_eq!(app.viewport().selected(), Some(3));
    assert_eq!(selected_row(&app), FRAME.trim_end());
    assert_eq!(app.viewport().total(), 5);
}

#[test]
fn line_zero_selects_the_first_row() {
    let app = jumped(applied(traces(), &[Action::Bottom]), ':', "0");
    assert_eq!(app.viewport().selected(), Some(0));
}

#[test]
fn a_line_past_the_end_selects_the_last_row() {
    for line in ["7", "99999999999999999999999"] {
        let app = jumped(traces(), ':', line);
        assert_eq!(app.viewport().selected(), Some(2), "{line}");
    }
}

#[test]
fn a_line_number_that_is_not_one_keeps_the_prompt_open_with_the_reason() {
    for text in ["-1", "abc", "4.5"] {
        let app = jumped(traces(), ':', text);
        assert_eq!(
            *jump_prompt(&app),
            JumpPrompt {
                to: Jump::Line,
                text: text.to_string(),
                error: Some(format!("'{text}' is not a line number")),
            }
        );
        assert_eq!(pos(&app), (0, Some(0)), "{text}");
    }
}

#[test]
fn line_jump_into_an_entry_the_filters_hide_selects_the_next_entry_they_show() {
    let app = jumped(traces_without(Level::Error), ':', "3");
    assert_eq!(selected_row(&app), DONE.trim_end());
}

#[test]
fn line_jump_past_the_last_entry_the_filters_show_selects_it() {
    for line in ["5", "99"] {
        let app = jumped(traces_without(Level::Warn), ':', line);
        assert_eq!(app.viewport().selected(), Some(1), "{line}");
        assert_eq!(selected_row(&app), FAILED.trim_end(), "{line}");
    }
}

#[test]
fn time_jump_selects_the_first_entry_at_or_after_the_time() {
    let times = [
        "11:00:00.000",
        "12:29:59.999",
        "12:30:00.000",
        "12:30:30.000",
        "13:00:00.000",
    ];
    let app = applied(
        app_in(built_in("logback"), log_at(&times).as_bytes()),
        &[Action::Bottom],
    );
    let app = jumped(app, 't', "12:30");
    assert_eq!(app.panel(), None);
    assert_eq!(app.viewport().selected(), Some(2));
}

#[test]
fn a_time_without_a_date_is_on_the_date_of_the_last_entry() {
    let log = [
        on("2026-10-06", "23:00:00.000"),
        on("2026-10-07", "01:00:00.000"),
        on("2026-10-07", "12:00:00.000"),
    ]
    .concat();
    let app = jumped(app_in(built_in("logback"), log.as_bytes()), 't', "00:30");
    assert_eq!(app.viewport().selected(), Some(1));
}

#[test]
fn a_time_before_the_first_entry_selects_the_first_entry_with_a_timestamp() {
    let log = [
        "starting up\n".to_string(),
        log_at(&["11:00:00.000", "12:00:00.000"]),
    ]
    .concat();
    let app = applied(
        app_in(built_in("logback"), log.as_bytes()),
        &[Action::Bottom],
    );
    let app = jumped(app, 't', "00:00");
    assert_eq!(app.viewport().selected(), Some(1));
}

#[test]
fn a_time_after_the_last_entry_selects_the_last_row() {
    let log = log_at(&["11:00:00.000", "12:00:00.000", "13:00:00.000"]);
    let app = jumped(app_in(built_in("logback"), log.as_bytes()), 't', "23:00");
    assert_eq!(app.viewport().selected(), Some(2));
}

#[test]
fn an_entry_without_a_timestamp_counts_as_the_time_of_the_one_before_it() {
    let log = [at("12:00:00.000"), UNDATED.to_string(), at("12:30:00.000")].concat();
    let app = jumped(app_in(built_in("logback"), log.as_bytes()), 't', "12:10");
    assert_eq!(selected_row(&app), at("12:30:00.000").trim_end());
}

#[test]
fn time_jump_passes_over_entries_the_filters_hide() {
    let error = "2026-10-07 12:40:00.000 [main] ERROR com.example.App - boom\n";
    let log = [
        at("11:00:00.000"),
        at("12:30:00.000"),
        error.to_string(),
        at("13:00:00.000"),
    ]
    .concat();
    let mut app = applied(
        app_in(built_in("logback"), log.as_bytes()),
        &[Action::ToggleLevel(Level::Error)],
    );
    wait_for_filter(&mut app);
    let app = jumped(app, 't', "12:35");
    assert_eq!(selected_row(&app), at("13:00:00.000").trim_end());
}

#[test]
fn a_time_that_does_not_parse_keeps_the_prompt_open_with_the_reason() {
    let app = jumped(hundred_lines(), 't', "soon");
    assert_eq!(
        jump_prompt(&app).error,
        Some(TimeSpec::parse("soon").unwrap_err())
    );
}

#[test]
fn time_jump_in_a_log_without_timestamps_stays_and_says_so() {
    let app = jumped(applied(hundred_lines(), &[Action::Down]), 't', "12:00");
    assert_eq!(app.panel(), None);
    assert_eq!(pos(&app), (0, Some(1)));
    assert_eq!(app.notice(), Some("no entry has a timestamp"));
}

proptest! {
    #[test]
    fn any_text_in_a_jump_prompt_never_panics(
        opener in prop::sample::select(vec![':', 't']),
        text in prop_oneof![any::<String>(), "[0-9: .dhm-]{0,24}"],
    ) {
        let log = [at("12:00:00.000"), UNDATED.to_string(), at("12:30:00.000")].concat();
        jumped(app_in(built_in("logback"), log.as_bytes()), opener, &text);
    }
}

// --- search ---

/// A regex matching entries 7, 42 and 90 of [`hundred_lines`], rows 6, 41 and 89.
const THREE_STEPS: &str = r"step (7|42|90)$";

/// `app` after opening the search prompt with `opener` (`/` or `?`), typing the
/// regex `pattern` and pressing Enter, with the matches counted.
fn searched(app: App, opener: char, pattern: &str) -> App {
    let mut keys = vec![char_key(opener), ctrl('r')];
    keys.extend(typing(pattern));
    keys.push(key(KeyCode::Enter));
    let mut app = pressed(app, &keys);
    wait_for_search(&mut app);
    app
}

fn selected(app: &App) -> Option<usize> {
    app.viewport().selected()
}

/// The open search prompt of `app` and whether it searches backward; fails the test
/// if it is closed.
fn search_prompt(app: &App) -> (&Prompt, bool) {
    match app.panel() {
        Some(Panel::Search { prompt, backward }) => (prompt, *backward),
        other => panic!("the search prompt is not open: {other:?}"),
    }
}

/// The pattern of `app`'s search and whether it goes backward, if it has one.
fn search_of(app: &App) -> Option<(&str, bool)> {
    let search = app.search()?;
    Some((search.matcher.pattern(), search.backward))
}

#[test]
fn search_keys_are_slash_question_mark_n_and_capital_n() {
    assert_eq!(action(char_key('/')), Some(Action::SearchForward));
    assert_eq!(action(char_key('?')), Some(Action::SearchBackward));
    assert_eq!(action(char_key('n')), Some(Action::NextMatch));
    assert_eq!(action(char_key('N')), Some(Action::PreviousMatch));
}

#[test]
fn slash_and_question_mark_open_an_empty_forward_and_backward_search_prompt() {
    for (opener, backward) in [('/', false), ('?', true)] {
        let app = pressed(hundred_lines(), &[char_key(opener)]);
        let empty = Prompt {
            pattern: String::new(),
            options: matcher::Options::default(),
            error: None,
        };
        assert_eq!(search_prompt(&app), (&empty, backward));
    }
}

#[test]
fn search_prompt_opens_empty_with_the_last_search_options() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    let app = pressed(app, &[char_key('?')]);
    let (prompt, _) = search_prompt(&app);
    assert_eq!(prompt.pattern, "");
    assert_eq!(prompt.options, REGEX);
}

#[test]
fn search_prompt_types_characters_and_toggles_its_options() {
    let mut keys = vec![char_key('/')];
    keys.extend(typing("nN/?q"));
    keys.extend([ctrl('r'), key(KeyCode::Tab)]);
    let app = pressed(hundred_lines(), &keys);
    let (prompt, _) = search_prompt(&app);
    assert_eq!(prompt.pattern, "nN/?q");
    let both = matcher::Options {
        regex: true,
        ignore_case: true,
    };
    assert_eq!(prompt.options, both);
    assert!(!app.should_quit());
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn enter_sets_the_search_and_selects_the_next_matching_entry() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    assert_eq!(app.panel(), None);
    assert_eq!(search_of(&app), Some((THREE_STEPS, false)));
    assert_eq!(selected(&app), Some(6));
    assert_eq!(app.notice(), None);
}

#[test]
fn backward_search_selects_the_previous_matching_entry() {
    let app = jumped(hundred_lines(), ':', "51");
    let app = searched(app, '?', THREE_STEPS);
    assert_eq!(search_of(&app), Some((THREE_STEPS, true)));
    assert_eq!(selected(&app), Some(41));
    assert_eq!(app.notice(), None);
}

#[test]
fn n_selects_the_next_match_and_capital_n_the_previous_one() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected(&app), Some(41));
    let app = pressed(app, &[char_key('N')]);
    assert_eq!(selected(&app), Some(6));
}

#[test]
fn after_a_backward_search_n_goes_up_and_capital_n_down() {
    let app = searched(jumped(hundred_lines(), ':', "51"), '?', THREE_STEPS);
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected(&app), Some(6));
    let app = pressed(app, &[char_key('N')]);
    assert_eq!(selected(&app), Some(41));
}

#[test]
fn n_past_the_last_match_wraps_to_the_first_with_a_notice() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    let app = pressed(app, &[char_key('n'), char_key('n')]);
    assert_eq!((selected(&app), app.notice()), (Some(89), None));
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected(&app), Some(6));
    assert_eq!(app.notice(), Some("search wrapped"));
}

#[test]
fn capital_n_before_the_first_match_wraps_to_the_last_with_a_notice() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    let app = pressed(app, &[char_key('N')]);
    assert_eq!(selected(&app), Some(89));
    assert_eq!(app.notice(), Some("search wrapped"));
}

#[test]
fn a_search_without_matches_says_pattern_not_found_and_stays() {
    let app = applied(hundred_lines(), &[Action::Down; 3]);
    let app = searched(app, '/', "nowhere");
    assert_eq!(app.panel(), None);
    assert_eq!(selected(&app), Some(3));
    assert_eq!(app.notice(), Some("pattern not found"));
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected(&app), Some(3));
    assert_eq!(app.notice(), Some("pattern not found"));
}

#[test]
fn n_and_capital_n_without_a_search_do_nothing() {
    let app = applied(
        hundred_lines(),
        &[
            Action::Down,
            Action::Down,
            Action::Down,
            Action::NextMatch,
            Action::PreviousMatch,
        ],
    );
    assert_eq!(pos(&app), (0, Some(3)));
    assert_eq!(app.notice(), None);
}

#[test]
fn an_invalid_regex_keeps_the_search_prompt_open_with_the_reason() {
    let keys = [char_key('/'), ctrl('r'), char_key('('), key(KeyCode::Enter)];
    let app = pressed(hundred_lines(), &keys);
    let (prompt, _) = search_prompt(&app);
    assert_eq!(prompt.error, Some(Matcher::new("(", REGEX).unwrap_err()));
    assert!(app.search().is_none());
}

#[test]
fn esc_closes_the_search_prompt_and_keeps_the_last_search() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    let app = pressed(app, &[char_key('?'), char_key('x'), key(KeyCode::Esc)]);
    assert_eq!(app.panel(), None);
    assert_eq!(search_of(&app), Some((THREE_STEPS, false)));
    assert_eq!(selected(&app), Some(6));
}

#[test]
fn enter_on_an_empty_search_pattern_clears_the_search() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    let app = pressed(app, &[char_key('/'), key(KeyCode::Enter)]);
    assert_eq!(app.panel(), None);
    assert!(app.search().is_none());
    assert_eq!(app.match_count(), None);
    assert_eq!(selected(&app), Some(6));
}

#[test]
fn match_count_is_the_matches_up_to_the_selected_entry_of_all_matches() {
    let app = searched(hundred_lines(), '/', THREE_STEPS);
    assert_eq!(app.match_count(), Some((1, 3)));
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(app.match_count(), Some((2, 3)));
    let app = pressed(app, &[char_key('j')]);
    assert_eq!(app.match_count(), Some((2, 3)));
    let app = pressed(app, &[char_key('g')]);
    assert_eq!(app.match_count(), Some((0, 3)));
}

#[test]
fn search_covers_the_whole_first_line_not_only_the_message() {
    let mut app = logback(&[STARTED, FAILED, DONE]);
    app.resize(10);
    let app = searched(app, '/', "ERROR");
    assert_eq!(selected(&app), Some(1));
    assert_eq!(app.match_count(), Some((1, 1)));
}

#[test]
fn matches_in_folded_stack_traces_count_for_their_entry() {
    let app = searched(traces(), '/', r"App\.java");
    assert_eq!(selected_row(&app), FAILED.trim_end());
    assert_eq!(app.match_count(), Some((1, 2)));
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected_row(&app), DONE.trim_end());
}

#[test]
fn the_count_and_n_cover_only_the_entries_the_filters_show() {
    let mut app = logback(&[STARTED, FAILED, DONE]);
    app.resize(10);
    let app = searched(app, '/', r"com\.example");
    assert_eq!(app.match_count(), Some((2, 3)));
    let mut app = applied(app, &[Action::ToggleLevel(Level::Error)]);
    wait_for_search(&mut app);
    assert_eq!(app.match_count(), Some((1, 2)));
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected_row(&app), DONE.trim_end());
}

#[test]
fn entries_appended_while_following_are_searched() {
    let app = followed(built_in("logback"), &[STARTED, FAILED, DONE].concat());
    let app = searched(app, '/', "Failed");
    assert_eq!(selected(&app), Some(1));
    assert!(!app.following());
    let mut app = grown(app, &[STARTED, FAILED, DONE, FAILED].concat());
    wait_for_search(&mut app);
    assert_eq!(app.match_count(), Some((1, 2)));
    let app = pressed(app, &[char_key('n')]);
    assert_eq!(selected(&app), Some(3));
}

/// `app` after searching it for the plain `pattern` with `/`, before any poll.
fn search_entered(app: App, pattern: &str) -> App {
    let mut keys = vec![char_key('/')];
    keys.extend(typing(pattern));
    keys.push(key(KeyCode::Enter));
    pressed(app, &keys)
}

#[test]
fn counting_a_large_file_runs_in_the_background() {
    let mut app = app(numbered_log(5000).as_bytes());
    app.resize(10);
    let mut app = search_entered(app, "step 4999");
    assert!(app.searching());
    wait_for_search(&mut app);
    assert_eq!(selected(&app), Some(4998));
    assert_eq!(app.match_count(), Some((1, 1)));
}

#[test]
fn a_search_moves_only_once_the_matches_it_needs_are_counted() {
    let mut app = following_app(built_in("raw"), numbered_log(5000).as_bytes());
    app.resize(10);
    wait_for_index(&mut app);
    // From the last row, the match lies past the wrap, which takes every match.
    let mut app = search_entered(app, "step 4999");
    assert_eq!(selected(&app), Some(4999));
    assert!(app.following());
    wait_for_search(&mut app);
    assert_eq!(selected(&app), Some(4998));
    assert!(!app.following());
}

#[test]
fn a_move_before_the_matches_are_counted_drops_the_search_move() {
    let mut app = app(numbered_log(5000).as_bytes());
    app.resize(10);
    let app = search_entered(app, "step 4999");
    let mut app = pressed(app, &[char_key('j')]);
    wait_for_search(&mut app);
    assert_eq!(selected(&app), Some(1));
    assert_eq!(app.notice(), None);
}

/// A sliding time filter re-evaluates only from the entries that can drop out, so the
/// search keeps the matches before them while the new entries are counted.
#[test]
fn matches_stay_counted_while_a_sliding_time_filter_takes_in_appended_entries() {
    let times = [
        "12:00:00.000",
        "12:01:00.000",
        "12:02:00.000",
        "12:03:00.000",
    ];
    let app = timed(followed(built_in("logback"), &log_at(&times)), "10m");
    let mut app = searched(app, '/', "at 12:0[015]");
    assert_eq!(app.match_count(), Some((1, 2)));
    let grown = [&times[..], &["12:04:00.000", "12:05:00.000"]].concat();
    let source = test_support::source(log_at(&grown).as_bytes());
    app.file_changed(FileChange::Grew(source));
    // The entry appended that matches may be counted already.
    assert!(
        matches!(app.match_count(), Some((1, 2 | 3))),
        "{:?}",
        app.match_count()
    );
    wait_for_search(&mut app);
    assert_eq!(app.match_count(), Some((1, 3)));
}

#[test]
fn the_app_expects_updates_while_only_the_search_is_counting() {
    let mut app = app(numbered_log(5000).as_bytes());
    app.resize(10);
    let app = search_entered(app, "step 4999");
    assert!(app.searching() && !app.filtering() && app.progress().is_complete());
    assert!(app.expects_updates());
}

/// [`numbered_log`] of 100 entries searched for entries 21 and 2001, with entry 51
/// selected, then grown to 3,100 entries: more than are counted again at once.
fn searched_then_grown() -> App {
    let mut app = app(numbered_log(100).as_bytes());
    app.resize(10);
    let app = jumped(searched(app, '/', r"step (21|2001)$"), ':', "51");
    let source = test_support::source(numbered_log(3100).as_bytes());
    let mut app = app;
    app.file_changed(FileChange::Grew(source));
    assert!(app.searching());
    app
}

#[test]
fn a_backward_step_over_entries_already_counted_moves_at_once() {
    let app = pressed(searched_then_grown(), &[char_key('N')]);
    assert_eq!(selected(&app), Some(20));
}

#[test]
fn a_backward_step_waits_until_the_entries_before_the_selection_are_counted() {
    let app = jumped(searched_then_grown(), ':', "3001");
    let mut app = pressed(app, &[char_key('N')]);
    assert_eq!(selected(&app), Some(3000));
    wait_for_search(&mut app);
    assert_eq!(selected(&app), Some(2000));
}

// --- details pane ---

/// An entry with a stack trace of three lines, then an entry of one line.
const DETAILS_LOG: &str = concat!(
    "2026-10-07 12:00:00.000 [main] ERROR com.example.App - Failed\n",
    "java.lang.IllegalStateException: boom\n",
    "\tat com.example.App.run(App.java:10)\n",
    "\tat com.example.App.main(App.java:5)\n",
    "2026-10-07 12:00:01.000 [main] INFO  com.example.App - Recovered\n",
);

/// The first entry of `DETAILS_LOG`, as in the file.
fn first_details_entry() -> String {
    DETAILS_LOG
        .lines()
        .take(4)
        .map(|line| line.to_owned() + "\n")
        .collect()
}

/// `DETAILS_LOG` with the details pane shown, 2 lines high for an entry that takes
/// 10, the first entry selected and the focus on the table.
fn details_shown() -> App {
    let mut app = app_in(built_in("logback"), DETAILS_LOG.as_bytes());
    app.resize(10);
    app.apply(Action::Confirm);
    app.fit_details(2, 10);
    app
}

/// [`details_shown`] with the focus on the details pane.
fn details_focused() -> App {
    applied(details_shown(), &[Action::CycleFocus])
}

#[test]
fn details_pane_is_hidden_at_first() {
    let app = app_in(built_in("logback"), DETAILS_LOG.as_bytes());
    assert!(!app.shows_details());
    assert_eq!(app.focus(), Focus::Table);
}

#[test]
fn confirm_without_a_panel_shows_the_details_pane() {
    assert!(details_shown().shows_details());
}

#[test]
fn confirm_hides_the_shown_details_pane() {
    let app = applied(details_shown(), &[Action::Confirm]);
    assert!(!app.shows_details());
}

#[test]
fn confirm_in_a_panel_leaves_the_details_pane_hidden() {
    let app = app_in(built_in("logback"), DETAILS_LOG.as_bytes());
    let app = applied(app, &[Action::LevelPanel, Action::Confirm]);
    assert!(!app.shows_details());
}

#[test]
fn cycle_focus_moves_the_focus_to_the_details_pane_and_back() {
    let mut app = details_shown();
    app.apply(Action::CycleFocus);
    assert_eq!(app.focus(), Focus::Details);
    app.apply(Action::CycleFocus);
    assert_eq!(app.focus(), Focus::Table);
}

#[test]
fn cycle_focus_without_the_details_pane_keeps_the_focus_on_the_table() {
    let app = app_in(built_in("logback"), DETAILS_LOG.as_bytes());
    let app = applied(app, &[Action::CycleFocus]);
    assert_eq!(app.focus(), Focus::Table);
}

#[test]
fn hiding_the_details_pane_gives_the_focus_back_to_the_table() {
    let app = applied(details_focused(), &[Action::Confirm]);
    assert_eq!(app.focus(), Focus::Table);
}

#[test]
fn down_with_the_details_focused_scrolls_them_and_keeps_the_selection() {
    let app = applied(details_focused(), &[Action::Down]);
    assert_eq!(app.details_scroll(), 1);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn up_with_the_details_focused_scrolls_them_back() {
    let app = applied(details_focused(), &[Action::Down, Action::Down, Action::Up]);
    assert_eq!(app.details_scroll(), 1);
}

#[test]
fn page_down_with_the_details_focused_scrolls_them_by_their_height() {
    let app = applied(details_focused(), &[Action::PageDown]);
    assert_eq!(app.details_scroll(), 2);
}

#[test]
fn page_up_with_the_details_focused_scrolls_them_back_by_their_height() {
    let app = applied(details_focused(), &[Action::Down; 5]);
    let app = applied(app, &[Action::PageUp]);
    assert_eq!(app.details_scroll(), 3);
}

#[test]
fn details_scroll_stops_at_the_first_line() {
    let app = applied(details_focused(), &[Action::Down, Action::PageUp]);
    assert_eq!(app.details_scroll(), 0);
}

#[test]
fn details_scroll_stops_with_the_last_line_at_the_bottom() {
    let app = applied(details_focused(), &[Action::Down; 20]);
    assert_eq!(app.details_scroll(), 8);
}

#[test]
fn details_taking_fewer_lines_scroll_back_to_their_last_line() {
    let mut app = applied(details_focused(), &[Action::Down; 8]);
    app.fit_details(2, 5);
    assert_eq!(app.details_scroll(), 3);
}

#[test]
fn scrolling_the_details_up_keeps_following() {
    let mut app = followed(built_in("logback"), DETAILS_LOG);
    app.apply(Action::Confirm);
    app.fit_details(2, 10);
    let app = applied(app, &[Action::CycleFocus, Action::Down, Action::Up]);
    assert!(app.following());
}

#[test]
fn other_actions_with_the_details_focused_work_on_the_table() {
    let app = applied(details_focused(), &[Action::Bottom]);
    assert_eq!(app.viewport().selected(), Some(1));
}

#[test]
fn selecting_another_entry_scrolls_the_details_back_to_the_top() {
    let app = applied(
        details_focused(),
        &[Action::Down, Action::CycleFocus, Action::Down],
    );
    assert_eq!(app.viewport().selected(), Some(1));
    assert_eq!(app.details_scroll(), 0);
}

#[test]
fn details_entry_follows_the_selection() {
    let app = applied(details_shown(), &[Action::Down]);
    let second = DETAILS_LOG.lines().last().unwrap().to_owned() + "\n";
    assert_eq!(app.details_entry(), Some(second.as_bytes()));
}

#[test]
fn moving_between_the_lines_of_the_selected_entry_keeps_the_details_scroll() {
    let app = applied(
        details_shown(),
        &[
            Action::ToggleFold,
            Action::CycleFocus,
            Action::Down,
            Action::CycleFocus,
            Action::Down,
        ],
    );
    assert_eq!(app.viewport().selected(), Some(1));
    assert_eq!(app.details_scroll(), 1);
}

#[test]
fn details_entry_is_every_line_of_the_selected_entry_folded_or_not() {
    let entry = first_details_entry();
    assert_eq!(details_shown().details_entry(), Some(entry.as_bytes()));
}

#[test]
fn append_to_the_shown_entry_keeps_the_details_scroll_and_adds_its_lines() {
    let first = first_details_entry();
    let mut app = followed(built_in("logback"), &first);
    app.apply(Action::Confirm);
    app.fit_details(2, 10);
    let app = applied(app, &[Action::CycleFocus, Action::Down]);
    let longer = first + "\tat java.lang.Thread.run(Thread.java:1583)\n";
    let app = grown(app, &longer);
    assert_eq!(app.details_scroll(), 1);
    assert_eq!(app.details_entry(), Some(longer.as_bytes()));
}

// --- exception jumps ---

const RETRYING: &str = "2026-10-07 12:00:01.700 [main] INFO  com.example.App - Retrying\n";
const STOPPED: &str = "2026-10-07 12:00:03.000 [main] INFO  com.example.App - Stopped\n";

/// Five entries, of which [`FAILED`] (entry 1) and [`DONE`] (entry 3) have a stack
/// trace.
const EXCEPTIONS: [&str; 8] = [
    STARTED, FAILED, EXCEPTION, FRAME, RETRYING, DONE, FRAME, STOPPED,
];

/// [`EXCEPTIONS`] on a 10-line screen, every entry folded, with row `row` selected.
fn exceptions_at(row: usize) -> App {
    let mut app = logback(&EXCEPTIONS);
    app.resize(10);
    applied(app, &vec![Action::Down; row])
}

/// [`EXCEPTIONS`] with the `hidden` levels hidden, the filter's results in.
fn exceptions_without(hidden: &[Level]) -> App {
    let mut filters = Filters::default();
    for &level in hidden {
        filters.levels.set(level, false);
    }
    filtered(exceptions_at(0), filters)
}

/// The selected row of `app` and what its notice says.
fn landed(app: &App) -> (Option<usize>, Option<&str>) {
    (selected(app), app.notice())
}

#[test]
fn closing_bracket_selects_the_next_entry_with_an_exception() {
    let app = pressed(exceptions_at(0), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(1), None));
}

#[test]
fn opening_bracket_selects_the_previous_entry_with_an_exception() {
    let app = pressed(exceptions_at(4), &[char_key('[')]);
    assert_eq!(landed(&app), (Some(3), None));
}

#[test]
fn from_an_entry_with_an_exception_the_jumps_go_past_it() {
    let app = pressed(exceptions_at(1), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(3), None));
    let app = pressed(app, &[char_key('[')]);
    assert_eq!(landed(&app), (Some(1), None));
}

#[test]
fn closing_bracket_past_the_last_exception_wraps_to_the_first_with_a_notice() {
    let app = pressed(exceptions_at(3), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(1), Some("exceptions wrapped")));
}

#[test]
fn opening_bracket_before_the_first_exception_wraps_to_the_last_with_a_notice() {
    let app = pressed(exceptions_at(0), &[char_key('[')]);
    assert_eq!(landed(&app), (Some(3), Some("exceptions wrapped")));
}

#[test]
fn from_inside_an_unfolded_trace_a_jump_selects_the_first_line_of_the_next_exception() {
    let unfolded = [Action::UnfoldAll, Action::Down, Action::Down, Action::Down];
    let app = applied(exceptions_at(0), &unfolded);
    assert_eq!(selected_row(&app), FRAME.trim_end());
    let app = pressed(app, &[char_key(']')]);
    assert_eq!(landed(&app), (Some(5), None));
    assert_eq!(selected_row(&app), DONE.trim_end());
    assert_eq!(app.viewport().total(), 8);
}

#[test]
fn exception_jumps_pass_over_the_entries_the_filters_hide() {
    let app = pressed(exceptions_without(&[Level::Error]), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(2), None));
    assert_eq!(selected_row(&app), DONE.trim_end());
}

#[test]
fn the_only_exception_in_view_wraps_onto_itself_with_a_notice() {
    let app = pressed(exceptions_without(&[Level::Warn]), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(1), None));
    for bracket in [']', '['] {
        let app = pressed(
            exceptions_without(&[Level::Warn]),
            &[char_key(']'), char_key(bracket)],
        );
        assert_eq!(
            landed(&app),
            (Some(1), Some("exceptions wrapped")),
            "{bracket}"
        );
    }
}

#[test]
fn without_an_exception_in_view_the_jumps_say_no_exceptions_and_stay() {
    let without_traces = || applied(hundred_lines(), &[Action::Down; 3]);
    let traces_hidden = || {
        applied(
            exceptions_without(&[Level::Error, Level::Warn]),
            &[Action::Down],
        )
    };
    let cases: [(&dyn Fn() -> App, usize); 2] = [(&without_traces, 3), (&traces_hidden, 1)];
    for (app, row) in cases {
        for bracket in [']', '['] {
            let app = pressed(app(), &[char_key(bracket)]);
            assert_eq!(
                landed(&app),
                (Some(row), Some("no exceptions")),
                "{bracket}"
            );
        }
    }
}

#[test]
fn an_exception_jump_stops_following() {
    let app = followed(built_in("logback"), &EXCEPTIONS.concat());
    assert_eq!(selected(&app), Some(4));
    let app = pressed(app, &[char_key('[')]);
    assert_eq!(landed(&app), (Some(3), None));
    assert!(!app.following());
}

#[test]
fn exception_jumps_cover_the_last_entries_shown_before_the_index_is_complete() {
    let mut app = following_app(built_in("logback"), EXCEPTIONS.concat().as_bytes());
    app.resize(10);
    assert_eq!(selected(&app), Some(4));
    let app = pressed(app, &[char_key('['), char_key('[')]);
    assert_eq!(landed(&app), (Some(1), None));
}

// --- bookmarks ---

/// `app` with the entries on `rows` bookmarked, one after the other, with `m`.
fn marked(mut app: App, rows: &[usize]) -> App {
    for &row in rows {
        app = applied(app, &[Action::Top]);
        app = applied(app, &vec![Action::Down; row]);
        app = pressed(app, &[char_key('m')]);
    }
    app
}

/// [`EXCEPTIONS`] on a 10-line screen with the entries on `rows` bookmarked and row
/// `at` selected.
fn bookmarked_at(rows: &[usize], at: usize) -> App {
    let app = applied(marked(exceptions_at(0), rows), &[Action::Top]);
    applied(app, &vec![Action::Down; at])
}

/// The rows of `app` that show a bookmarked entry.
fn bookmarks(app: &App) -> Vec<usize> {
    (0..app.viewport().total())
        .filter(|&i| app.bookmarked(i))
        .collect()
}

/// `app` with the ERROR entries hidden, the filter's results in.
fn hiding_errors(app: App) -> App {
    let mut filters = Filters::default();
    filters.levels.set(Level::Error, false);
    filtered(app, filters)
}

/// [`EXCEPTIONS`] in `Raw`, every line an entry, on a 10-line screen.
fn exception_lines() -> App {
    let mut app = app(EXCEPTIONS.concat().as_bytes());
    app.resize(10);
    app
}

#[test]
fn m_bookmarks_the_selected_entry() {
    let app = pressed(exceptions_at(2), &[char_key('m')]);
    assert_eq!(bookmarks(&app), [2]);
    assert_eq!(landed(&app), (Some(2), Some("bookmarked")));
}

#[test]
fn m_on_a_bookmarked_entry_removes_its_bookmark() {
    let app = pressed(bookmarked_at(&[2], 2), &[char_key('m')]);
    assert_eq!(bookmarks(&app), Vec::<usize>::new());
    assert_eq!(app.notice(), Some("bookmark removed"));
}

#[test]
fn m_on_a_stack_frame_bookmarks_its_whole_entry() {
    let unfolded = [Action::UnfoldAll, Action::Down, Action::Down, Action::Down];
    let app = pressed(applied(exceptions_at(0), &unfolded), &[char_key('m')]);
    assert_eq!(bookmarks(&app), [1, 2, 3]);
}

#[test]
fn quote_selects_the_next_bookmark() {
    let app = pressed(bookmarked_at(&[1, 3], 1), &[char_key('\'')]);
    assert_eq!(landed(&app), (Some(3), None));
}

#[test]
fn double_quote_selects_the_previous_bookmark() {
    let app = pressed(bookmarked_at(&[1, 3], 3), &[char_key('"')]);
    assert_eq!(landed(&app), (Some(1), None));
}

#[test]
fn bookmark_jumps_past_the_last_or_first_bookmark_wrap_around_with_a_notice() {
    for (at, quote, to) in [(4, '\'', 1), (0, '"', 3)] {
        let app = pressed(bookmarked_at(&[1, 3], at), &[char_key(quote)]);
        assert_eq!(
            landed(&app),
            (Some(to), Some("bookmarks wrapped")),
            "{quote}"
        );
    }
}

#[test]
fn without_a_bookmark_in_view_the_jumps_say_no_bookmarks_and_stay() {
    let unmarked = || exceptions_at(2);
    let marks_hidden = || applied(hiding_errors(bookmarked_at(&[1], 0)), &[Action::Top]);
    let cases: [(&dyn Fn() -> App, usize); 2] = [(&unmarked, 2), (&marks_hidden, 0)];
    for (app, row) in cases {
        for quote in ['\'', '"'] {
            let app = pressed(app(), &[char_key(quote)]);
            assert_eq!(landed(&app), (Some(row), Some("no bookmarks")), "{quote}");
        }
    }
}

#[test]
fn bookmark_jumps_pass_over_the_entries_the_filters_hide() {
    let app = applied(hiding_errors(bookmarked_at(&[1, 3], 0)), &[Action::Top]);
    let app = pressed(app, &[char_key('\'')]);
    assert_eq!(landed(&app), (Some(2), None));
    assert_eq!(selected_row(&app), DONE.trim_end());
}

#[test]
fn a_backward_bookmark_jump_passes_over_a_bookmark_the_filters_hide() {
    let app = applied(hiding_errors(bookmarked_at(&[0, 1], 0)), &[Action::Bottom]);
    let app = pressed(app, &[char_key('"')]);
    assert_eq!(landed(&app), (Some(0), None));
}

#[test]
fn a_bookmark_the_filters_hide_is_kept_for_when_they_show_it_again() {
    let app = hiding_errors(bookmarked_at(&[1, 3], 0));
    assert_eq!(bookmarks(&app), [2]);
    let app = filtered(app, Filters::default());
    assert_eq!(bookmarks(&app), [1, 3]);
}

#[test]
fn a_bookmark_jump_stops_following() {
    let app = applied(
        followed(built_in("logback"), &EXCEPTIONS.concat()),
        &[Action::Up],
    );
    let app = pressed(app, &[char_key('m'), char_key('F')]);
    assert!(app.following());
    let app = pressed(app, &[char_key('"')]);
    assert_eq!(landed(&app), (Some(3), None));
    assert!(!app.following());
}

#[test]
fn cycling_the_preset_moves_a_bookmark_to_the_entry_that_holds_its_offset() {
    // Under `Raw` row 2 is the exception line, which Logback reads as part of entry 1;
    // row 5 starts entry 3.
    let mut app = cycled(marked(exception_lines(), &[2, 5]));
    wait_for_index(&mut app);
    assert_eq!(app.settings().format, built_in("logback"));
    assert_eq!(bookmarks(&app), [1, 3]);
}

#[test]
fn bookmark_jumps_land_on_the_entry_that_holds_a_bookmark_partway_into_it() {
    // Row 2 under `Raw` lies inside Logback's entry 1, not at its start.
    let mut app = cycled(marked(exception_lines(), &[2]));
    wait_for_index(&mut app);
    for (from, quote) in [(Action::Top, '\''), (Action::Bottom, '"')] {
        app = pressed(applied(app, &[from]), &[char_key(quote)]);
        assert_eq!(landed(&app), (Some(1), None), "{quote}");
    }
}

#[test]
fn unbookmarking_an_entry_removes_every_bookmark_inside_it() {
    let mut app = cycled(marked(exception_lines(), &[2, 3]));
    wait_for_index(&mut app);
    assert_eq!(bookmarks(&app), [1]);
    let app = applied(app, &[Action::Top, Action::Down]);
    let app = pressed(app, &[char_key('m')]);
    assert_eq!(bookmarks(&app), Vec::<usize>::new());
}

#[test]
fn truncation_deletion_and_another_file_at_the_path_remove_every_bookmark() {
    let content = EXCEPTIONS.concat();
    let changes: [fn(Source) -> FileChange; 3] = [
        FileChange::Truncated,
        |_| FileChange::Gone,
        FileChange::Replaced,
    ];
    for change in changes {
        let app = pressed(followed(built_in("logback"), &content), &[char_key('m')]);
        assert_eq!(bookmarks(&app), [4]);
        let app = changed(app, change(test_support::source(content.as_bytes())));
        assert_eq!(bookmarks(&app), Vec::<usize>::new());
    }
}

#[test]
fn a_bookmark_set_before_the_index_is_complete_stays_on_its_entry() {
    let mut app = following_app(built_in("logback"), EXCEPTIONS.concat().as_bytes());
    app.resize(10);
    let mut app = pressed(app, &[char_key('m')]);
    assert_eq!(bookmarks(&app), [4]);
    wait_for_index(&mut app);
    assert_eq!(bookmarks(&app), [4]);
}

// --- jumps while the filter is evaluating ---

/// A Logback log of `n` entries, `… - step 1` to `… - step n`, in which the entries
/// at the indexes in `traced` have a stack trace.
fn traced_log(n: usize, traced: &[usize]) -> String {
    (0..n)
        .map(|i| {
            let trace = if traced.contains(&i) {
                [EXCEPTION, FRAME].concat()
            } else {
                String::new()
            };
            format!(
                "2026-10-07 12:00:00.000 [main] INFO  com.example.App - step {}\n{trace}",
                i + 1
            )
        })
        .collect()
}

/// [`traced_log`] of `n` entries with traces at `traced`, on a 10-line screen.
fn traced_app(n: usize, traced: &[usize]) -> App {
    let mut app = app_in(built_in("logback"), traced_log(n, traced).as_bytes());
    app.resize(10);
    app
}

/// `app` filtered to the entries whose message contains `word`, before any of the
/// filter's results come in.
fn filter_set(mut app: App, word: &str) -> App {
    app.set_filters(text_filter(word));
    assert!(app.filtering());
    app
}

/// [`traced_log`] of 100 entries with a trace at entry 20, under a filter that shows
/// every entry, with entry `at` selected, then grown to 3,100 entries with another
/// trace at entry 3000: more than the filter evaluates again at once.
fn traced_then_grown(at: usize) -> App {
    let mut app = filter_set(traced_app(100, &[20]), "step");
    wait_for_filter(&mut app);
    let mut app = applied(app, &vec![Action::Down; at]);
    let source = test_support::source(traced_log(3100, &[20, 3000]).as_bytes());
    app.file_changed(FileChange::Grew(source));
    assert!(app.filtering());
    assert_eq!(selected(&app), Some(at));
    app
}

#[test]
fn an_exception_jump_waits_for_the_filter_and_lands_on_the_exception_it_finds() {
    let mut app = pressed(traced_then_grown(50), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(50), Some("searching…")));
    wait_for_filter(&mut app);
    assert_eq!(landed(&app), (Some(3000), None));
    assert!(!app.following());
}

#[test]
fn an_exception_jump_wraps_only_once_the_filter_has_evaluated_every_entry() {
    let mut app = pressed(traced_then_grown(10), &[char_key('[')]);
    assert_eq!(landed(&app), (Some(10), Some("searching…")));
    wait_for_filter(&mut app);
    assert_eq!(landed(&app), (Some(3000), Some("exceptions wrapped")));
}

#[test]
fn an_exception_jump_to_an_entry_already_evaluated_moves_at_once() {
    let app = pressed(traced_then_grown(10), &[char_key(']')]);
    assert_eq!(landed(&app), (Some(20), None));
}

#[test]
fn an_exception_jump_waits_for_the_index_and_lands_on_the_exception_it_finds() {
    let mut app = unindexed_app(built_in("logback"), EXCEPTIONS.concat().as_bytes());
    app.resize(10);
    let mut app = pressed(app, &[char_key(']')]);
    assert_eq!(app.notice(), Some("searching…"));
    wait_for_index(&mut app);
    assert_eq!(landed(&app), (Some(1), None));
}

#[test]
fn a_bookmark_jump_waits_for_the_filter_and_lands_on_the_bookmark_it_finds() {
    let app = filter_set(marked(traced_app(100, &[]), &[80]), "step");
    let mut app = pressed(app, &[char_key('\'')]);
    assert_eq!(app.notice(), Some("searching…"));
    wait_for_filter(&mut app);
    assert_eq!(landed(&app), (Some(80), None));
}

#[test]
fn a_jump_that_finds_nothing_once_the_filter_is_done_says_so() {
    for (jump, none) in [(']', "no exceptions"), ('\'', "no bookmarks")] {
        // Entry 3, `step 4`, has the only trace and the only bookmark.
        let app = filter_set(marked(traced_app(100, &[3]), &[3]), "step 5");
        let mut app = pressed(app, &[char_key(jump)]);
        assert_eq!(app.notice(), Some("searching…"), "{jump}");
        wait_for_filter(&mut app);
        assert_eq!(app.notice(), Some(none), "{jump}");
    }
}

#[test]
fn a_key_while_a_jump_waits_cancels_it() {
    let mut app = pressed(traced_then_grown(50), &[char_key(']'), char_key('j')]);
    wait_for_filter(&mut app);
    assert_eq!(landed(&app), (Some(51), None));
}

#[test]
fn a_click_while_a_jump_waits_cancels_it() {
    let app = mapped(pressed(traced_then_grown(50), &[char_key(']')]));
    let top = app.viewport().top();
    let mut app = clicked(app, &[(4, 0)]);
    wait_for_filter(&mut app);
    assert_eq!(landed(&app), (Some(top + 3), None));
}

#[test]
fn a_filter_change_while_a_jump_waits_cancels_it() {
    let app = pressed(traced_then_grown(50), &[char_key(']')]);
    let mut app = applied(app, &[Action::ToggleLevel(Level::Debug)]);
    wait_for_filter(&mut app);
    assert_eq!(landed(&app), (Some(0), None));
}

// --- histogram ---

/// The width the histogram is worked out for.
const HISTOGRAM_WIDTH: usize = 60;

/// An INFO, a WARN, an ERROR of another class and a FATAL entry, a second apart.
const HISTOGRAM_LOG: &str = concat!(
    "2026-10-07 10:00:00.000 [main] INFO  com.example.App - Started\n",
    "2026-10-07 10:00:01.000 [main] WARN  com.example.App - Slow\n",
    "2026-10-07 10:00:02.000 [main] ERROR com.example.Pay - Failed\n",
    "2026-10-07 10:00:03.000 [main] FATAL com.example.App - Down\n",
);

fn histogram_app() -> App {
    let mut app = app_in(built_in("logback"), HISTOGRAM_LOG.as_bytes());
    app.resize(10);
    app
}

fn histogram_counts(app: &mut App) -> Vec<u32> {
    app.histogram(HISTOGRAM_WIDTH).unwrap().counts.clone()
}

/// `app` with the histogram shown and worked out.
fn with_histogram(mut app: App) -> App {
    app.apply(Action::ToggleHistogram);
    app.histogram(HISTOGRAM_WIDTH);
    app
}

/// [`histogram_app`] with the histogram shown and the focus on it.
fn histogram_focused() -> App {
    applied(with_histogram(histogram_app()), &[Action::CycleFocus])
}

#[test]
fn capital_h_toggles_the_histogram() {
    assert_eq!(action(char_key('H')), Some(Action::ToggleHistogram));
}

#[test]
fn histogram_is_hidden_at_first() {
    assert!(!histogram_app().shows_histogram());
}

#[test]
fn toggle_histogram_shows_and_hides_it() {
    let mut app = applied(histogram_app(), &[Action::ToggleHistogram]);
    assert!(app.shows_histogram());
    app.apply(Action::ToggleHistogram);
    assert!(!app.shows_histogram());
}

#[test]
fn showing_the_histogram_without_timestamps_says_so() {
    let app = applied(hundred_lines(), &[Action::ToggleHistogram]);
    assert_eq!(app.notice(), Some("no entry has a timestamp"));
}

#[test]
fn histogram_counts_only_the_entries_the_filters_show() {
    let mut app = histogram_app();
    assert_eq!(histogram_counts(&mut app), [0, 1, 1, 1]);
    let classes = Classes::parse(["!com.example.Pay"]).unwrap();
    let mut app = filtered(
        app,
        Filters {
            classes,
            ..Filters::default()
        },
    );
    assert_eq!(histogram_counts(&mut app), [0, 1, 0, 1]);
}

/// An ERROR two seconds after the last entry of [`HISTOGRAM_LOG`].
const LATER_ERROR: &str = "2026-10-07 10:00:05.000 [main] ERROR com.example.App - Again\n";

/// [`HISTOGRAM_LOG`], followed, with the histogram shown and fed, grown by a line that
/// continues its last entry and by [`LATER_ERROR`].
fn histogram_grown(app: App) -> App {
    let mut app = with_histogram(app);
    assert!(app.histogram(HISTOGRAM_WIDTH).is_some());
    grown(app, &[HISTOGRAM_LOG, EXCEPTION, LATER_ERROR].concat())
}

#[test]
fn histogram_counts_appended_entries_once() {
    let mut app = histogram_grown(followed(built_in("logback"), HISTOGRAM_LOG));
    assert_eq!(histogram_counts(&mut app), [0, 1, 1, 1, 0, 1]);
}

#[test]
fn filtered_histogram_counts_appended_entries_once() {
    let classes = Classes::parse(["!com.example.Pay"]).unwrap();
    let filters = Filters {
        classes,
        ..Filters::default()
    };
    let app = filtered(followed(built_in("logback"), HISTOGRAM_LOG), filters);
    let mut app = histogram_grown(app);
    wait_for_filter(&mut app);
    assert_eq!(histogram_counts(&mut app), [0, 1, 0, 1, 0, 1]);
}

#[test]
fn histogram_counts_only_what_a_truncated_file_holds_now() {
    let app = with_histogram(followed(built_in("logback"), HISTOGRAM_LOG));
    let mut app = truncated(app, LATER_ERROR);
    assert_eq!(histogram_counts(&mut app), [1]);
}

#[test]
fn histogram_counts_only_what_a_replacing_file_holds() {
    let app = with_histogram(followed(built_in("logback"), HISTOGRAM_LOG));
    let replacing = test_support::source(LATER_ERROR.as_bytes());
    let mut app = changed(app, FileChange::Replaced(replacing));
    assert_eq!(histogram_counts(&mut app), [1]);
}

#[test]
fn histogram_counts_each_entry_once_across_preset_cycles() {
    let classes = Classes::parse(["!com.example.Pay"]).unwrap();
    let filters = Filters {
        classes,
        ..Filters::default()
    };
    let mut app = with_histogram(filtered(histogram_app(), filters));
    for _ in BUILT_IN {
        app.histogram(HISTOGRAM_WIDTH);
        app.apply(Action::CyclePreset);
        wait_for_filter(&mut app);
    }
    assert_eq!(app.settings().format, built_in("logback"));
    assert_eq!(histogram_counts(&mut app), [0, 1, 0, 1]);
}

#[test]
fn cycle_focus_goes_down_the_shown_panes_and_back_to_the_table() {
    let mut app = applied(with_histogram(histogram_app()), &[Action::Confirm]);
    let mut seen = Vec::new();
    for _ in 0..3 {
        app.apply(Action::CycleFocus);
        seen.push(app.focus());
    }
    assert_eq!(seen, [Focus::Histogram, Focus::Details, Focus::Table]);
}

#[test]
fn cycle_focus_skips_the_histogram_without_timestamps() {
    let app = applied(with_histogram(hundred_lines()), &[Action::CycleFocus]);
    assert_eq!(app.focus(), Focus::Table);
}

#[test]
fn hiding_the_histogram_gives_the_focus_back_to_the_table() {
    let app = applied(histogram_focused(), &[Action::ToggleHistogram]);
    assert_eq!(app.focus(), Focus::Table);
}

#[test]
fn right_with_the_histogram_focused_selects_the_next_bucket_instead_of_scrolling() {
    let app = applied(histogram_focused(), &[Action::ScrollRight]);
    assert_eq!(app.histogram_bucket(), 1);
    assert_eq!(app.scroll(), 0);
}

#[test]
fn left_with_the_histogram_focused_selects_the_bucket_before() {
    let app = applied(
        histogram_focused(),
        &[Action::ScrollRight, Action::ScrollRight, Action::ScrollLeft],
    );
    assert_eq!(app.histogram_bucket(), 1);
}

#[test]
fn bucket_selection_stops_at_the_first_and_the_last_bucket() {
    let app = applied(histogram_focused(), &[Action::ScrollLeft]);
    assert_eq!(app.histogram_bucket(), 0);
    let app = applied(app, &[Action::ScrollRight; 5]);
    assert_eq!(app.histogram_bucket(), 3);
}

#[test]
fn down_with_the_histogram_focused_moves_the_selection() {
    let app = applied(histogram_focused(), &[Action::Down]);
    assert_eq!(app.viewport().selected(), Some(1));
}

#[test]
fn histogram_counts_the_filters_results_as_they_come_in() {
    let mut app = with_histogram(histogram_app());
    app.set_filters(Filters {
        levels: Levels::only(&[Level::Warn, Level::Error]),
        ..Filters::default()
    });
    // Before the first results.
    app.histogram(HISTOGRAM_WIDTH);
    wait_for_filter(&mut app);
    assert_eq!(histogram_counts(&mut app), [1, 1]);
}

#[test]
fn histogram_fits_a_new_width() {
    let mut app = histogram_app();
    app.histogram(HISTOGRAM_WIDTH);
    // The four seconds share one 5 s bucket.
    assert_eq!(app.histogram(1).unwrap().counts, [3]);
}

#[test]
fn a_shown_histogram_with_entries_left_to_count_expects_updates() {
    let mut app = unindexed_app(built_in("logback"), HISTOGRAM_LOG.as_bytes());
    app.apply(Action::ToggleHistogram);
    wait_for_index(&mut app);
    assert!(app.expects_updates());
    app.histogram(HISTOGRAM_WIDTH);
    assert!(!app.expects_updates());
}

#[test]
fn showing_a_histogram_with_timestamps_sets_no_notice() {
    let mut app = histogram_app();
    app.key(char_key('H'));
    assert_eq!(app.notice(), None);
}

#[test]
fn the_selected_bucket_stays_within_a_narrower_histogram() {
    let mut app = applied(histogram_focused(), &[Action::ScrollRight; 3]);
    app.apply(Action::ToggleLevel(Level::Fatal));
    wait_for_filter(&mut app);
    assert_eq!(app.histogram(HISTOGRAM_WIDTH).unwrap().counts.len(), 3);
    assert_eq!(app.histogram_bucket(), 2);
}

#[test]
fn confirming_a_bucket_without_entries_gives_the_focus_back_to_the_table() {
    let log = [HISTOGRAM_LOG, LATER_ERROR].concat();
    let app = with_histogram(app_in(built_in("logback"), log.as_bytes()));
    // The fifth bucket, 10:00:04, has no entry.
    let mut app = applied(
        app,
        &[
            Action::CycleFocus,
            Action::ScrollRight,
            Action::ScrollRight,
            Action::ScrollRight,
            Action::ScrollRight,
            Action::Confirm,
        ],
    );
    wait_for_filter(&mut app);
    assert_eq!(app.histogram(HISTOGRAM_WIDTH), None);
    assert_eq!(app.focus(), Focus::Table);
}

/// [`histogram_focused`] after setting the time filter to the third bucket.
fn third_bucket_confirmed() -> App {
    applied(
        histogram_focused(),
        &[Action::ScrollRight, Action::ScrollRight, Action::Confirm],
    )
}

#[test]
fn confirm_with_the_histogram_focused_sets_the_time_filter_to_the_selected_bucket() {
    let app = third_bucket_confirmed();
    assert_eq!(
        app.time_filter().map(ToString::to_string).as_deref(),
        Some("2026-10-07 10:00:02..2026-10-07 10:00:02")
    );
    assert_eq!(
        app.filters().time,
        Some(TimeRange {
            start: "2026-10-07T10:00:02Z".parse().unwrap(),
            end: Some("2026-10-07T10:00:03Z".parse().unwrap()),
        })
    );
    assert!(!app.shows_details());
}

#[test]
fn confirming_a_bucket_selects_the_first_bucket_again() {
    assert_eq!(third_bucket_confirmed().histogram_bucket(), 0);
}

// --- export ---

#[test]
fn e_opens_the_export_prompt() {
    assert_eq!(action(char_key('e')), Some(Action::Export));
}

#[test]
fn f8_opens_the_export_prompt() {
    assert_eq!(action(key(KeyCode::F(8))), Some(Action::Export));
}

/// A viewer over `content` in `preset`, opened from `app.log` in `dir`, where it is
/// written, with `dir` as its current directory. Its index isn't complete yet.
fn exporting(dir: &tempfile::TempDir, preset: &'static Format, content: &str) -> App {
    let path = dir.path().join("app.log");
    fs::write(&path, content).unwrap();
    let mut app = App::new(
        &path,
        source::open(&path).unwrap(),
        Some(preset),
        test_support::zone(),
        test_support::mtime(),
        false,
    );
    app.set_current_dir(dir.path());
    app
}

/// [`TWO_TRACES`] in Logback, exporting to `dir`, with the whole file indexed.
fn exporting_traces(dir: &tempfile::TempDir) -> App {
    let mut app = exporting(dir, built_in("logback"), &TWO_TRACES.concat());
    wait_for_index(&mut app);
    app
}

/// `app` after opening the export prompt, typing `path` over its suggestion and
/// confirming.
fn exported_to(mut app: App, path: &str) -> App {
    app.key(char_key('e'));
    let suggested = match app.panel() {
        Some(Panel::Export(prompt)) => prompt.text.chars().count(),
        other => panic!("expected the export prompt, got {other:?}"),
    };
    let keys = [
        vec![key(KeyCode::Backspace); suggested],
        typing(path),
        vec![key(KeyCode::Enter)],
    ]
    .concat();
    pressed(app, &keys)
}

fn export_prompt(text: &str, error: Option<&str>, overwrite: bool) -> Panel {
    Panel::Export(ExportPrompt {
        text: text.to_owned(),
        error: error.map(str::to_owned),
        overwrite,
    })
}

#[test]
fn export_prompt_suggests_the_file_name_without_gz() {
    let app = pressed(
        test_support::gzip_app(&test_support::gzip(b"line\n")),
        &[char_key('e')],
    );
    assert_eq!(
        app.panel(),
        Some(&export_prompt("app.log.filtered.log", None, false))
    );
}

#[test]
fn export_writes_every_entry_the_filters_show_with_all_its_lines() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = filtered(exporting_traces(&dir), without_info());
    let app = exported_to(applied(app, &[Action::FoldAll]), "out.log");
    assert_eq!(app.panel(), None);
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        [FAILED, EXCEPTION, FRAME, DONE, FRAME].concat()
    );
}

/// What [`App::notice`] says while an export waits for the index and the filters.
const EXPORT_WAITS: &str = "export waits for the filters";

/// Polls `app` until its pending export is done. Fails the test after
/// [`INDEX_DEADLINE`].
fn wait_for_export(app: &mut App) {
    let deadline = Instant::now() + INDEX_DEADLINE;
    loop {
        app.poll_index();
        if app.notice() != Some(EXPORT_WAITS) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "exporting took longer than {INDEX_DEADLINE:?}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// [`TWO_TRACES`] without INFO, exported to `out.log` in `dir` before the index and
/// the filter are polled even once.
fn export_before_polling(dir: &tempfile::TempDir) -> App {
    let mut app = exporting(dir, built_in("logback"), &TWO_TRACES.concat());
    app.set_filters(without_info());
    exported_to(app, "out.log")
}

/// Asserts that `app` says its export to `out.log` in `dir` is cancelled and that no
/// file is written once the index and the filters are done.
fn assert_export_cancelled(mut app: App, dir: &tempfile::TempDir) {
    assert_eq!(app.notice(), Some("export cancelled"));
    wait_for_filter(&mut app);
    assert!(!dir.path().join("out.log").exists());
}

#[test]
fn export_waits_for_the_index_and_the_filters() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = export_before_polling(&dir);
    assert_eq!(app.notice(), Some(EXPORT_WAITS));
    assert_eq!(app.panel(), None);
    assert!(!dir.path().join("out.log").exists());
    wait_for_export(&mut app);
    assert_eq!(app.notice(), Some("exported 2 entries to out.log"));
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        [FAILED, EXCEPTION, FRAME, DONE, FRAME].concat()
    );
}

#[test]
fn esc_cancels_a_waiting_export() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = pressed(export_before_polling(&dir), &[key(KeyCode::Esc)]);
    assert_export_cancelled(app, &dir);
}

#[test]
fn changing_the_filters_cancels_a_waiting_export() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = pressed(export_before_polling(&dir), &[char_key('x')]);
    assert_export_cancelled(app, &dir);
}

#[test]
fn export_says_how_many_entries_it_wrote() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = exported_to(filtered(exporting_traces(&dir), without_info()), "out.log");
    assert_eq!(app.notice(), Some("exported 2 entries to out.log"));
}

#[test]
fn export_of_no_entries_says_nothing_to_export_and_writes_no_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = filtered(exporting_traces(&dir), text_filter("nowhere"));
    let app = exported_to(app, "out.log");
    assert_eq!(app.notice(), Some("nothing to export"));
    assert_eq!(app.panel(), None);
    assert!(!dir.path().join("out.log").exists());
}

/// [`TWO_TRACES`] exporting to `out.log` in `dir`, which already holds `old`.
fn exported_over_a_file(dir: &tempfile::TempDir) -> App {
    let app = exporting_traces(dir);
    fs::write(dir.path().join("out.log"), "old\n").unwrap();
    exported_to(app, "out.log")
}

#[test]
fn export_asks_before_overwriting_a_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = exported_over_a_file(&dir);
    assert_eq!(app.panel(), Some(&export_prompt("out.log", None, true)));
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        "old\n"
    );
}

#[test]
fn y_overwrites_the_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = pressed(exported_over_a_file(&dir), &[char_key('y')]);
    assert_eq!(app.panel(), None);
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        TWO_TRACES.concat()
    );
}

#[test]
fn ctrl_c_quits_while_asking_to_overwrite() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = pressed(exported_over_a_file(&dir), &[ctrl('c')]);
    assert!(app.should_quit());
}

#[test]
fn another_key_than_y_keeps_the_file_and_goes_back_to_the_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = pressed(exported_over_a_file(&dir), &[char_key('n')]);
    assert_eq!(app.panel(), Some(&export_prompt("out.log", None, false)));
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        "old\n"
    );
}

#[test]
fn export_refuses_the_viewed_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = exported_to(exporting_traces(&dir), "./app.log");
    let error = "cannot export to the file being viewed";
    assert_eq!(
        app.panel(),
        Some(&export_prompt("./app.log", Some(error), false))
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("app.log")).unwrap(),
        TWO_TRACES.concat()
    );
}

#[test]
fn export_to_a_directory_fails_without_asking_to_overwrite() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::create_dir(dir.path().join("logs")).unwrap();
    // A blank path is the current directory.
    for path in ["logs", ""] {
        let app = exported_to(exporting_traces(&dir), path);
        assert_eq!(
            app.notice(),
            Some("export failed: Is a directory (os error 21)"),
            "{path:?}"
        );
        assert_eq!(app.panel(), None, "{path:?}");
    }
}

#[test]
fn export_refuses_a_hard_link_to_the_viewed_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = exporting_traces(&dir);
    fs::hard_link(dir.path().join("app.log"), dir.path().join("link.log")).unwrap();
    let app = exported_to(app, "link.log");
    let error = "cannot export to the file being viewed";
    assert_eq!(
        app.panel(),
        Some(&export_prompt("link.log", Some(error), false))
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("app.log")).unwrap(),
        TWO_TRACES.concat()
    );
}

#[test]
fn export_failure_shows_in_the_notice() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let app = exported_to(exporting_traces(&dir), "locked/out.log");
    assert_eq!(
        app.notice(),
        Some("export failed: Permission denied (os error 13)")
    );
    assert_eq!(app.panel(), None);
}

/// [`export_before_polling`], with a file written to `out.log` while the export
/// waits, and the index and the filter done since.
fn file_appeared_while_waiting(dir: &tempfile::TempDir) -> App {
    let mut app = export_before_polling(dir);
    fs::write(dir.path().join("out.log"), "old\n").unwrap();
    wait_for_filter(&mut app);
    app
}

#[test]
fn waiting_export_asks_before_overwriting_a_file_that_appeared() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = file_appeared_while_waiting(&dir);
    assert_eq!(app.panel(), Some(&export_prompt("out.log", None, true)));
    assert_eq!(app.notice(), None);
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        "old\n"
    );
}

#[test]
fn y_overwrites_a_file_that_appeared_while_the_export_waited() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = pressed(file_appeared_while_waiting(&dir), &[char_key('y')]);
    assert_eq!(app.panel(), None);
    assert_eq!(app.notice(), Some("exported 2 entries to out.log"));
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        [FAILED, EXCEPTION, FRAME, DONE, FRAME].concat()
    );
}

#[test]
fn file_that_appeared_is_asked_about_once_the_open_panel_closes() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = export_before_polling(&dir);
    fs::write(dir.path().join("out.log"), "old\n").unwrap();
    let mut app = pressed(app, &[char_key('/'), char_key('a')]);
    wait_for_filter(&mut app);
    assert_eq!(search_prompt(&app).0.pattern, "a");
    assert_eq!(
        fs::read_to_string(dir.path().join("out.log")).unwrap(),
        "old\n"
    );
    let mut app = pressed(app, &[key(KeyCode::Esc)]);
    app.poll_index();
    assert_eq!(app.panel(), Some(&export_prompt("out.log", None, true)));
}

#[test]
fn waiting_export_over_a_file_agreed_to_does_not_ask_again() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("out.log"), "old\n").unwrap();
    let mut app = pressed(export_before_polling(&dir), &[char_key('y')]);
    wait_for_filter(&mut app);
    assert_eq!(app.panel(), None);
    assert_eq!(app.notice(), Some("exported 2 entries to out.log"));
}

/// Bytes a [`FailingFile`] takes before it fails.
const WRITTEN_BEFORE_FAILING: usize = 10;

/// A file that takes [`WRITTEN_BEFORE_FAILING`] bytes, then fails every write as a
/// full disk does.
struct FailingFile {
    file: File,
    left: usize,
}

impl Write for FailingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other("no space left on device"));
        }
        let written = self.file.write(&buf[..buf.len().min(self.left)])?;
        self.left -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn create_failing_file(path: &Path) -> io::Result<Box<dyn Write>> {
    Ok(Box::new(FailingFile {
        file: File::create(path)?,
        left: WRITTEN_BEFORE_FAILING,
    }))
}

#[test]
fn export_failing_half_way_removes_the_partial_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = exporting_traces(&dir);
    app.set_export_opener(create_failing_file);
    let app = exported_to(app, "out.log");
    assert_eq!(app.notice(), Some("export failed: no space left on device"));
    assert!(!dir.path().join("out.log").exists());
}

// --- mouse ---

fn mouse(kind: MouseEventKind, column: u16, line: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row: line,
        modifiers: KeyModifiers::NONE,
    }
}

fn left_click(column: u16, line: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), column, line)
}

/// Left clicks at column 5 of screen line `line`, each `ms` milliseconds after the
/// first.
fn clicked(mut app: App, clicks: &[(u16, u64)]) -> App {
    let start = Instant::now();
    for &(line, ms) in clicks {
        app.mouse(left_click(5, line), start + Duration::from_millis(ms));
    }
    app
}

fn wheel(mut app: App, kind: MouseEventKind) -> App {
    app.mouse(mouse(kind, 5, 5), Instant::now());
    app
}

/// A map of an 80-column screen with the header on line 0 and the rows from line 1
/// on, `lines` being the row drawn on each.
fn rows_map(lines: impl IntoIterator<Item = usize>) -> HitMap {
    let lines: Vec<usize> = lines.into_iter().collect();
    let height = u16::try_from(lines.len()).unwrap();
    HitMap {
        rows: Rect::new(0, 1, 80, height),
        lines,
        ..HitMap::default()
    }
}

/// `app` with the map of a screen showing its viewport's rows from line 1 on.
fn mapped(mut app: App) -> App {
    let viewport = app.viewport();
    let shown = viewport.top()..viewport.total().min(viewport.top() + viewport.height());
    app.set_hit_map(rows_map(shown));
    app
}

/// [`DETAILS_LOG`], every entry folded, so that it has 2 rows, mapped from line 1.
fn foldable() -> App {
    let mut app = app_in(built_in("logback"), DETAILS_LOG.as_bytes());
    app.resize(10);
    mapped(app)
}

#[test]
fn click_selects_the_row_drawn_under_it() {
    let app = clicked(
        mapped(applied(hundred_lines(), &[Action::Bottom])),
        &[(3, 0)],
    );
    assert_eq!(pos(&app), (90, Some(92)));
}

#[test]
fn click_stops_following_even_on_the_last_row() {
    let app = clicked(mapped(twenty_lines_followed()), &[(10, 0)]);
    assert_eq!(pos(&app), (10, Some(19)));
    assert!(!app.following());
}

#[test]
fn click_before_the_matches_are_counted_drops_the_search_move() {
    let mut app = app(numbered_log(5000).as_bytes());
    app.resize(10);
    let app = search_entered(app, "step 4999");
    let mut app = clicked(mapped(app), &[(4, 0)]);
    wait_for_search(&mut app);
    assert_eq!(selected(&app), Some(3));
}

#[test]
fn click_on_a_row_gone_since_the_screen_was_drawn_does_nothing() {
    let mut app = hundred_lines();
    // One past the last row.
    app.set_hit_map(rows_map([100]));
    let app = clicked(app, &[(1, 0)]);
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn right_click_does_nothing() {
    let mut app = mapped(hundred_lines());
    app.mouse(
        mouse(MouseEventKind::Down(MouseButton::Right), 5, 5),
        Instant::now(),
    );
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn mouse_does_nothing_while_a_panel_is_open() {
    let app = clicked(
        mapped(applied(hundred_lines(), &[Action::LevelPanel])),
        &[(5, 0)],
    );
    assert_eq!(pos(&app), (0, Some(0)));
    assert!(matches!(app.panel(), Some(Panel::Levels(_))));
}

#[test]
fn second_click_on_a_row_within_400_ms_toggles_its_fold() {
    let app = clicked(foldable(), &[(1, 0), (1, 400)]);
    assert_eq!(app.viewport().total(), 5);
    assert_eq!(app.viewport().selected(), Some(0));
}

#[test]
fn second_click_after_400_ms_only_selects() {
    let app = clicked(foldable(), &[(1, 0), (1, 401)]);
    assert_eq!(app.viewport().total(), 2);
}

#[test]
fn clicks_on_two_lines_of_a_wrapped_row_are_a_double_click() {
    let mut app = foldable();
    app.set_hit_map(rows_map([0, 0, 1]));
    let app = clicked(app, &[(1, 0), (2, 100)]);
    assert_eq!(app.viewport().total(), 5);
}

#[test]
fn clicks_on_two_rows_are_no_double_click() {
    let app = clicked(foldable(), &[(1, 0), (2, 100)]);
    assert_eq!(app.viewport().total(), 2);
    assert_eq!(app.viewport().selected(), Some(1));
}

#[test]
fn key_between_two_clicks_on_the_same_row_makes_no_double_click() {
    let mut app = foldable();
    let start = Instant::now();
    app.mouse(left_click(5, 1), start);
    // Unfolds the first entry, so the rows below it are numbered anew.
    app.key(char_key(' '));
    app.mouse(left_click(5, 1), start + Duration::from_millis(100));
    assert_eq!(app.viewport().total(), 5);
}

#[test]
fn click_after_a_double_click_starts_over() {
    let app = clicked(foldable(), &[(1, 0), (1, 100), (1, 200)]);
    assert_eq!(app.viewport().total(), 5);
}

#[test]
fn click_on_a_histogram_bucket_sets_the_time_filter_to_it() {
    let mut app = with_histogram(histogram_app());
    app.set_hit_map(HitMap {
        buckets: Some(Rect::new(0, 1, 4, 1)),
        ..rows_map(0..4)
    });
    app.mouse(left_click(2, 1), Instant::now());
    assert_eq!(
        app.time_filter().map(ToString::to_string).as_deref(),
        Some("2026-10-07 10:00:02..2026-10-07 10:00:02")
    );
}

#[test]
fn wheel_down_scrolls_the_screen_and_the_selection_three_rows() {
    let app = wheel(mapped(hundred_lines()), MouseEventKind::ScrollDown);
    assert_eq!(pos(&app), (3, Some(3)));
}

#[test]
fn wheel_up_scrolls_the_screen_and_the_selection_three_rows() {
    let app = mapped(applied(hundred_lines(), &[Action::Bottom]));
    let app = wheel(app, MouseEventKind::ScrollUp);
    assert_eq!(pos(&app), (87, Some(96)));
}

#[test]
fn wheel_up_stops_following() {
    let app = wheel(mapped(twenty_lines_followed()), MouseEventKind::ScrollUp);
    assert!(!app.following());
}

#[test]
fn wheel_over_the_details_pane_scrolls_it_three_lines() {
    let mut app = details_shown();
    app.set_hit_map(HitMap {
        details: Some(Rect::new(0, 3, 80, 5)),
        ..rows_map([0, 1])
    });
    app.mouse(mouse(MouseEventKind::ScrollDown, 5, 4), Instant::now());
    assert_eq!(app.details_scroll(), 3);
    assert_eq!(app.viewport().selected(), Some(0));
}

// --- clipboard ---

#[test]
fn y_copies_the_selected_entry() {
    assert_eq!(action(char_key('y')), Some(Action::Copy));
}

/// A clipboard that keeps the texts it is handed and answers each with `answer`.
#[derive(Debug)]
struct FakeClipboard {
    handed: Rc<RefCell<Vec<String>>>,
    answer: fn() -> Result<Via, CopyError>,
}

impl Clipboard for FakeClipboard {
    fn copy(&mut self, text: &str) -> Result<Via, CopyError> {
        self.handed.borrow_mut().push(text.to_owned());
        (self.answer)()
    }
}

/// `app` after `y` with a clipboard that answers `answer`, and the texts the
/// clipboard was handed.
fn copied(mut app: App, answer: fn() -> Result<Via, CopyError>) -> (App, Vec<String>) {
    let handed = Rc::default();
    app.set_clipboard(Box::new(FakeClipboard {
        handed: Rc::clone(&handed),
        answer,
    }));
    app.key(char_key('y'));
    let handed = handed.borrow().clone();
    (app, handed)
}

/// `DETAILS_LOG` on a 10-line screen, every entry folded, the first one selected.
fn folded_details_log() -> App {
    let mut app = app_in(built_in("logback"), DETAILS_LOG.as_bytes());
    app.resize(10);
    app.apply(Action::FoldAll);
    app
}

#[test]
fn copy_puts_every_line_of_the_selected_entry_on_the_clipboard() {
    let (app, handed) = copied(folded_details_log(), || Ok(Via::Osc52));
    assert_eq!(handed, [first_details_entry().trim_end()]);
    assert_eq!(app.notice(), Some("copied to clipboard"));
}

#[test]
fn copy_with_no_row_selected_copies_nothing() {
    let mut app = app(b"");
    app.resize(10);
    let (app, handed) = copied(app, || Ok(Via::Native));
    assert_eq!(handed, Vec::<String>::new());
    assert_eq!(app.notice(), Some("nothing to copy"));
}

#[test]
fn copy_without_a_clipboard_says_so() {
    let mut app = folded_details_log();
    app.key(char_key('y'));
    assert_eq!(app.notice(), Some("no clipboard"));
}

#[test]
fn copy_of_an_entry_too_large_for_osc52_says_so() {
    let (app, _) = copied(folded_details_log(), || Err(CopyError::TooLarge));
    assert_eq!(
        app.notice(),
        Some("entry too large for OSC 52 (over 100 KB)")
    );
}

#[test]
fn copy_failure_shows_in_the_notice() {
    let (app, _) = copied(folded_details_log(), || {
        Err(CopyError::Terminal(io::Error::other("broken pipe")))
    });
    assert_eq!(app.notice(), Some("copy failed: broken pipe"));
}

// --- help overlay ---

/// How far the help overlay is scrolled, `None` while it is closed.
fn help_scroll(app: &App) -> Option<usize> {
    match app.panel() {
        Some(Panel::Help(help)) => Some(help.scroll),
        _ => None,
    }
}

/// How many lines the help overlay has.
fn help_lines() -> usize {
    Keymap::default().help().len()
}

/// `app` with the help overlay open, showing `height` lines at a time, and `keys`
/// pressed in it.
fn in_help(app: App, height: usize, keys: &[KeyEvent]) -> App {
    let mut app = pressed(app, &[key(KeyCode::F(1))]);
    app.fit_help(height);
    pressed(app, keys)
}

#[test]
fn f1_opens_the_help_overlay_at_its_first_line() {
    let app = pressed(hundred_lines(), &[key(KeyCode::F(1))]);
    assert_eq!(help_scroll(&app), Some(0));
}

#[test]
fn esc_f1_and_q_close_the_help_overlay_without_quitting() {
    for close in [key(KeyCode::Esc), key(KeyCode::F(1)), char_key('q')] {
        let app = in_help(hundred_lines(), 10, &[close]);
        assert_eq!(help_scroll(&app), None, "{close:?}");
        assert!(!app.should_quit(), "{close:?}");
    }
}

#[test]
fn ctrl_c_quits_with_the_help_overlay_open() {
    let app = in_help(hundred_lines(), 10, &[ctrl('c')]);
    assert!(app.should_quit());
}

#[test]
fn j_k_and_the_arrows_scroll_the_help_overlay_by_a_line() {
    let keys = [
        char_key('j'),
        char_key('j'),
        key(KeyCode::Down),
        char_key('k'),
    ];
    let app = in_help(hundred_lines(), 10, &keys);
    assert_eq!(help_scroll(&app), Some(2));
    let app = pressed(app, &[key(KeyCode::Up)]);
    assert_eq!(help_scroll(&app), Some(1));
}

#[test]
fn page_keys_scroll_the_help_overlay_by_its_height() {
    let keys = [
        key(KeyCode::PageDown),
        key(KeyCode::PageDown),
        key(KeyCode::PageUp),
    ];
    let app = in_help(hundred_lines(), 10, &keys);
    assert_eq!(help_scroll(&app), Some(10));
}

#[test]
fn help_overlay_does_not_scroll_above_its_first_line() {
    let keys = [char_key('j'), key(KeyCode::PageUp), char_key('k')];
    let app = in_help(hundred_lines(), 10, &keys);
    assert_eq!(help_scroll(&app), Some(0));
}

#[test]
fn help_overlay_does_not_scroll_past_its_last_line_at_the_bottom() {
    let mut keys = vec![key(KeyCode::PageDown); help_lines()];
    keys.push(char_key('j'));
    let app = in_help(hundred_lines(), 10, &keys);
    assert_eq!(help_scroll(&app), Some(help_lines() - 10));
}

#[test]
fn help_overlay_that_shows_every_line_does_not_scroll() {
    let keys = [char_key('j'), key(KeyCode::PageDown)];
    let app = in_help(hundred_lines(), help_lines(), &keys);
    assert_eq!(help_scroll(&app), Some(0));
}

#[test]
fn a_taller_help_overlay_scrolls_back_to_keep_its_last_line_at_the_bottom() {
    let keys = vec![key(KeyCode::PageDown); help_lines()];
    let mut app = in_help(hundred_lines(), 10, &keys);
    app.fit_help(20);
    assert_eq!(help_scroll(&app), Some(help_lines() - 20));
}

#[test]
fn other_keys_do_nothing_while_the_help_overlay_is_open() {
    let keys = [
        char_key('G'),
        char_key('/'),
        key(KeyCode::F(2)),
        char_key('x'),
    ];
    let app = in_help(hundred_lines(), 10, &keys);
    assert_eq!(help_scroll(&app), Some(0));
    assert_eq!(pos(&app), (0, Some(0)));
}

#[test]
fn help_overlay_opens_again_at_its_first_line() {
    let keys = [char_key('j'), key(KeyCode::Esc), key(KeyCode::F(1))];
    let app = in_help(hundred_lines(), 10, &keys);
    assert_eq!(help_scroll(&app), Some(0));
}

#[test]
fn f1_does_nothing_in_another_panel() {
    let app = pressed(hundred_lines(), &[key(KeyCode::F(2)), key(KeyCode::F(1))]);
    assert!(matches!(app.panel(), Some(Panel::Levels(_))));
}

// --- thread column ---

/// PRD §7.2: a notice instead of an empty column. `raw` has no columns at all.
#[test]
fn toggling_the_thread_column_of_a_format_without_threads_only_says_so() {
    let log = b"2026-10-07 12:00:00.123 [INF] Order 42 created\n";
    for format in [no_thread_or_logger(), built_in("raw")] {
        let app = applied(app_in(format, log), &[Action::ToggleThread]);
        assert!(!app.shows_thread(), "{format:?}");
        assert_eq!(
            app.notice(),
            Some("the format has no thread column"),
            "{format:?}"
        );
    }
}

// --- configuration ---

/// A remapped action answers to its new key and no longer to its default `w`.
#[test]
fn configured_keys_decide_what_keys_do() {
    let mut app = hundred_lines();
    app.configure(&Config::parse("[keys]\ntoggle_wrap = \"v\"\n").unwrap());
    app.key(char_key('v'));
    assert!(app.wraps());
    app.key(char_key('w'));
    assert!(app.wraps());
}

#[test]
fn configure_takes_the_ui_toggles_from_the_config() {
    for on in [true, false] {
        let text = format!(
            "[ui]\nshow_thread = {on}\nshow_histogram = {on}\nshow_line_numbers = {on}\nwrap = {on}\n"
        );
        let mut app = hundred_lines();
        app.configure(&Config::parse(&text).unwrap());
        let shown = (
            app.shows_thread(),
            app.shows_histogram(),
            app.shows_line_numbers(),
            app.wraps(),
        );
        assert_eq!(shown, (on, on, on, on));
    }
}

// --- JSON lines ---

/// The JSON fixture `name`, its format detected, on a 10-line screen, with the whole
/// file indexed.
fn json_app(name: &str) -> App {
    let mut app = detected_app(&json_fixture(name));
    app.resize(10);
    app
}

/// The line numbers in the file of the rows of `app`.
fn row_numbers(app: &App) -> Vec<u64> {
    (0..app.viewport().total())
        .map(|i| app.row(i).and_then(|row| row.number).unwrap())
        .collect()
}

/// pino's levels are numbers.
const PINO_LEVEL_COUNTS: [u32; 6] = [1, 1, 3, 1, 1, 1];

#[test]
fn level_filter_on_a_json_log_reads_the_levels_from_its_level_key() {
    let filters = Filters {
        levels: Levels::only(&[Level::Error, Level::Fatal]),
        ..Filters::default()
    };
    let app = filtered(json_app("pino.log"), filters);
    assert_eq!(row_numbers(&app), [5, 7]);
}

#[test]
fn time_filter_on_a_json_log_reads_the_timestamps_from_its_timestamp_key() {
    // zap writes epoch seconds.
    let time = TimeRange {
        start: "2026-03-14T09:12:02Z".parse().unwrap(),
        end: Some("2026-03-14T09:12:05Z".parse().unwrap()),
    };
    let filters = Filters {
        time: Some(time),
        ..Filters::default()
    };
    let app = filtered(json_app("zap.log"), filters);
    assert_eq!(row_numbers(&app), [3, 4, 5]);
}

#[test]
fn logger_filter_on_a_json_log_reads_the_loggers_from_its_logger_key() {
    let filters = Filters {
        classes: Classes::parse(["com.acme.orders.OrderService"]).unwrap(),
        ..Filters::default()
    };
    let app = filtered(json_app("logstash-logback.log"), filters);
    assert_eq!(row_numbers(&app), [3, 6]);
}

#[test]
fn exception_jump_on_a_json_log_lands_on_the_entry_with_an_exception_field() {
    let app = pressed(json_app("ms-json-console.log"), &[char_key(']')]);
    assert_eq!(selected(&app), Some(4));
}

#[test]
fn histogram_of_a_json_log_counts_its_warnings_and_errors_over_its_timestamps() {
    // slog: 08:12:01 to 08:12:07 UTC in 1 s buckets, a WARN at :03 and an ERROR at :04.
    let mut app = json_app("slog.log");
    let histogram = app.histogram(HISTOGRAM_WIDTH).unwrap();
    assert_eq!(histogram.start, "2026-03-14T08:12:01Z".parse().unwrap());
    assert_eq!(histogram.counts, [0, 0, 1, 1, 0, 0, 0]);
}

#[test]
fn json_named_by_the_format_reads_the_keys_the_file_uses() {
    let app = app_in(built_in("json"), &json_fixture("pino.log"));
    assert_eq!(app.level_counts(), PINO_LEVEL_COUNTS);
}

#[test]
fn cycling_into_json_reads_the_keys_the_file_uses() {
    let mut app = cycled(app_in(
        built_in("wildfly-console"),
        &json_fixture("pino.log"),
    ));
    wait_for_index(&mut app);
    assert_eq!(app.settings().format, built_in("json"));
    assert_eq!(app.level_counts(), PINO_LEVEL_COUNTS);
}

#[test]
fn text_filter_on_a_json_log_matches_the_decoded_message() {
    let app = filtered(json_app("pino.log"), text_filter(r#""X-Legacy""#));
    assert_eq!(row_numbers(&app), [4]);
}

/// The exception's lines are the entry's stack trace, which the text filter covers.
#[test]
fn text_filter_on_a_json_log_matches_the_decoded_exception() {
    let filter = text_filter("\tat com.acme.orders.OrderService.create");
    let app = filtered(json_app("logstash-logback.log"), filter);
    assert_eq!(row_numbers(&app), [5]);
}

/// As in a text format, the text filter reads the message, not the other columns.
#[test]
fn text_filter_on_a_json_log_leaves_out_the_other_fields() {
    let app = filtered(json_app("pino.log"), text_filter("web-1"));
    assert_eq!(row_numbers(&app), Vec::<u64>::new());
}

// --- logfmt ---

/// The logfmt fixture `name` read as logfmt, on a 10-line screen, with the whole file
/// indexed.
fn logfmt_app(name: &str) -> App {
    let mut app = app_in(built_in("logfmt"), &test_support::logfmt_fixture(name));
    app.resize(10);
    app
}

/// logrus writes `\"X-Legacy\"` in its fourth message.
#[test]
fn text_filter_on_a_logfmt_log_matches_the_decoded_message() {
    let app = filtered(logfmt_app("logrus.log"), text_filter(r#""X-Legacy""#));
    assert_eq!(row_numbers(&app), [4]);
}

/// As in a text format, the text filter reads the message, not the other fields:
/// `insufficient` is only in logrus' `error` field.
#[test]
fn text_filter_on_a_logfmt_log_leaves_out_the_other_fields() {
    let app = filtered(logfmt_app("logrus.log"), text_filter("insufficient"));
    assert_eq!(row_numbers(&app), Vec::<u64>::new());
}
