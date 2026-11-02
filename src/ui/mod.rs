//! Drawing an [`App`]: a header line, the file's lines and a footer with key hints.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::iter;

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Action, App, ClassPanel, Jump, LevelPanel, Panel, Prompt, TimePanel};
use crate::filter::{Classes, Levels, TimeFilter, TimeSpec};
use crate::follow::Liveness;
use crate::format::Level;
use crate::keymap::{self, Keymap};
use crate::mouse::HitMap;
use crate::source::Origin;
use layout::Screen;

mod abbrev;
mod details;
mod histogram;
mod layout;
mod structured;
mod table;

pub use abbrev::abbreviate;

#[cfg(test)]
mod tests;

/// Whether the screen may use colors and text attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Styled,
    /// No colors, no bold, no reverse video: every cell in the terminal's default style.
    Plain,
}

impl Theme {
    /// The theme for the value of the `NO_COLOR` environment variable: any non-empty
    /// value turns styling off (<https://no-color.org>).
    pub fn from_no_color(value: Option<&OsStr>) -> Self {
        match value {
            Some(value) if !value.is_empty() => Self::Plain,
            _ => Self::Styled,
        }
    }

    /// `style` if this theme allows styling, the terminal's default style otherwise.
    fn paint(self, style: Style) -> Style {
        match self {
            Self::Styled => style,
            Self::Plain => Style::new(),
        }
    }
}

const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;
const TOO_SMALL: &str = "terminal too small (80×24 needed)";
const NO_MATCHES: &str = "no entries match the current filters";
const LEVEL_PANEL_HINTS: &str = "j/k Move  Space/1-6 Toggle  Enter Apply  Esc Cancel";
const CLASS_PANEL_HINTS: &str = "↑/↓ Line (↓ at the end adds one)  Enter Apply  Esc Cancel";
const TIME_PANEL_HINTS: &str = "↑/↓ Preset  Tab File/now  Enter Apply  Esc Cancel";
const TEXT_PROMPT_HINTS: &str = "Ctrl-r Regex  Tab Ignore case  Enter Apply  Esc Cancel";
const SEARCH_PROMPT_HINTS: &str = "Ctrl-r Regex  Tab Ignore case  Enter Search  Esc Cancel";
const TAB_STOP: usize = 4;

/// Draws the whole screen: the header (`vilog ─ <file name> ─ <size> ─ <preset>`, the
/// preset the index reads the file in, then the follow status, then
/// ` ─ format not recognized, showing raw` while `Raw` is shown because detection
/// found no preset, then ` ─ indexing <percent>%` until the index is complete), the
/// rows on screen with `▶` in front of the selected one and `◆` in front of the
/// first line of every bookmarked entry but the selected one ([`App::bookmarked`]),
/// and the key hints in the footer.
///
/// While [`App::shows_line_numbers`], each row starts with its line number in the
/// file, in dark gray, right-aligned as wide as the last line number of the entries
/// needs (at least 4) and followed by a space; blank while it isn't known.
///
/// The follow status is ` ─ READ-ONLY ○` for a gzip file, which is never followed,
/// then ` ─ truncated .gz` if it ended early or was corrupt ([`App::origin`]). For a
/// plain file it is ` ─ WAITING FOR FILE ○` while the file is gone from its path,
/// otherwise ` ─ FOLLOW ` and the dot while the app follows the file,
/// ` ─ PAUSED +N new ` and the dot while it doesn't and N new entries came in
/// ([`App::new_entries`]), and ` ─ PAUSED ` and the dot while it doesn't and none
/// came. The dot shows the file's [liveness](App::liveness): `●` while it is live,
/// `◐` while it is quiet and `○` while it is idle. Its cell keeps the header's bold
/// reverse video with a green, yellow or dark gray background, so that it shows in
/// that colour on the bar; under the plain theme the shape alone tells them apart.
///
/// The row that marks a truncation shows `── file truncated ──` after the gutter.
///
/// Under `Raw` a row is the entry's first line. Under the other presets it is a table
/// row, columns separated by a space: the timestamp in the app's zone
/// (the configured format, `%H:%M:%S%.3f` below 100 columns), the level (5 wide,
/// in the style of [`App::colors`]), the thread if shown (15 wide, a longer name keeps
/// its end after `…`), the logger abbreviated to fit (30% of the width, from 24 to 40 columns,
/// cut with `…` if even the abbreviation is too wide) and the message in the rest.
/// A column the format lacks (see [`Format::has`](crate::format::Format::has)) is left out and
/// the message takes its width.
/// The headless entry has only a message, its first line. A folded entry's message
/// ends with ` [+N]`, N being the lines it hides; an unfolded entry's continuation
/// lines are rows of their own, in the message column. Messages are cut at the
/// right edge, or wrapped onto the following lines of the message column; scrolled
/// right, they stop once the longest message on screen ends at the right edge.
/// The lines in between follow the frame's height, so this resizes `app`.
///
/// Below 80×24 the lines are replaced by `terminal too small (80×24 needed)` and `app`
/// keeps its size, so a briefly small terminal doesn't move the view. They are
/// replaced by `no entries match the current filters` once the filters' results are
/// in and no entry passes ([`App::nothing_matches`]).
///
/// While the level panel ([`App::panel`]) is open it is drawn over the middle of
/// the body, a box 16 columns wide and 8 high titled ` Levels `: one line per level,
/// TRACE to FATAL, as ` ▶[x] 1 TRACE ` (`▶` on the cursor's line, `[ ]` for a level
/// the panel hides, then the key that toggles it and the name). The footer then
/// shows the panel's keys, `j/k Move  Space/1-6 Toggle  Enter Apply  Esc Cancel`.
///
/// The logger panel is drawn the same way, a box 44 columns wide and 10 high titled
/// ` Loggers `: one line per rule line of the panel, as ` ▶com.myco.*` (`▶` on the
/// cursor's line). The footer then shows `↑/↓ Line (↓ at the end adds one)  Enter
/// Apply  Esc Cancel`, or, while the panel has an [error](crate::app::ClassPanel::error),
/// why its rules don't parse.
///
/// The time panel is drawn the same way, a box 40 columns wide and 4 high titled
/// ` Time `: the spec as typed, as ` ▶10m`, and below it the filter it makes, as
/// `  last 10m (file)`, blank while it makes none. The footer then shows `↑/↓
/// Preset  Tab File/now  Enter Apply  Esc Cancel`, or the panel's
/// [error](crate::app::TimePanel::error).
///
/// The text filter prompt is drawn the same way, a box 60 columns wide and 4 high
/// titled ` Text filter `: the pattern as typed, as ` ▶time.*out`, and below it the
/// options, as `  [x] regex  [ ] ignore case`. The footer then shows `Ctrl-r Regex
/// Tab Ignore case  Enter Apply  Esc Cancel`, or the prompt's
/// [error](crate::app::Prompt::error).
///
/// The search prompt is drawn the same way, titled ` Search ` or ` Search backward `,
/// with the footer `Ctrl-r Regex  Tab Ignore case  Enter Search  Esc Cancel`.
///
/// While a jump prompt is open, the footer shows it in place of the key hints: `:`
/// and the text typed for a line, `t ` and the text for a time, followed by ` ─ `
/// and the prompt's [error](crate::app::JumpPrompt::error) while it has one.
///
/// The export prompt is shown the same way, as `export to: ` and the path typed,
/// or, while it [asks to overwrite](crate::app::ExportPrompt::overwrite),
/// `overwrite <path>? y/n`.
///
/// While the help overlay is open, it is drawn the same way, a box titled ` Help `,
/// as wide as its widest line and the border need (the widest keys and the widest
/// description plus 5 columns) and as high as its lines and the border need, but no
/// wider and no higher than the body: one line per line of [`Keymap::help`](crate::keymap::Keymap::help), from the first
/// that [`HelpPanel::scroll`](crate::app::HelpPanel::scroll) leaves, as ` k ↑
/// Move up`: a space, the keys left-aligned in a column as wide as the widest keys,
/// two spaces and the description. It tells the app how many lines it shows
/// ([`App::fit_help`]). The footer then shows `j/k Scroll  PgUp/PgDn Page
/// F1/Esc/q Close`.
///
/// While the app [shows the details pane](App::shows_details), the bottom of the
/// body is a box titled ` Details `, a third of the screen's height high (at least 5
/// lines, and at most what leaves the rows 5), with a plain border (`┌─┐`), or a thick
/// one (`┏━┓`) while it has the [focus](App::focus). Inside, one column in from each
/// side, it shows the selected entry ([`App::details_entry`]): under a preset, a
/// first line with the full logger and the thread in parentheses, as
/// `com.example.App (main)` (a part the entry lacks is left out, and a thread alone
/// stays in parentheses), then the message of the entry's first line, then every
/// continuation line; the headless entry, and every entry under `Raw`, show their
/// lines as they are. Lines are made safe to show like the rows and wrapped at the
/// box's inner width, cut at the column rather than at a word. The pane is scrolled as
/// [`App::details_scroll`] says, and tells the app its inner height and how many
/// wrapped lines the entry takes ([`App::fit_details`]).
///
/// While the app [shows the histogram](App::shows_histogram) and has one, the line
/// below the filter bar, or below the header without one, is the histogram row, taken
/// from the body. Its first columns, all but the last 20, show the histogram
/// ([`App::histogram`] for that many columns), one column per bucket from the left:
/// blank for a bucket without entries, otherwise one of `▁▂▃▄▅▆▇█` for its count in
/// eighths of the largest count, rounded up, so that the largest gets `█` and every
/// other bucket with an entry at least `▁`. Right-aligned at the end of the line is
/// the largest count and the bucket size, as `max 9/5m` (sizes as `1s`, `10m`, `6h`,
/// `1d`, and merged days as `37d`). While the histogram has the [focus](App::focus),
/// the selected bucket ([`App::histogram_bucket`]) is in reverse video and the end of
/// the line is its count and its start in the app's zone, as `1 at 10:10:00`, or as
/// `1 at 2026-10-07` for buckets of a day or more. Below 80×24 there is no histogram
/// row.
///
/// While the app has a search, the header has ` ─ match k/N` ([`App::match_count`])
/// after the status and before the notice.
///
/// While the app has a [notice](App::notice), the header ends with ` ─ ` and it.
/// The level counters ([`App::level_counts`]) are right-aligned on the header line,
/// FATAL to TRACE as `F:1 E:37 W:212 I:5000 D:12 T:3`, levels with no entries left
/// out. The title comes first: the counters only get the columns it leaves after a
/// space, and are left out entirely when they don't fit there.
///
/// While a filter is active, the line below the header is the filter bar, taken from
/// the body: `Filters: ` and the active filters two spaces apart, in this order,
/// `level[W E F]` (the first letters of the levels shown), `logger[com.myco.*
/// !org.hibernate]` (the logger rules), `time[last 10m (file)]` (the time filter) and
/// `text[timeout]` (the pattern; `text[time.*out (regex, ignore case)]` with the
/// options that are on). Without filters there is no bar.
pub fn render(frame: &mut Frame, app: &mut App, theme: Theme) {
    let area = frame.area();
    let filters = filter_bar(app);
    let histogram = histogram::row(app, area, theme);
    let buckets = histogram.as_ref().map(|&(_, buckets)| buckets);
    let screen = Screen::new(
        area,
        filters.is_some(),
        histogram.is_some(),
        app.shows_details(),
    );
    let Screen {
        header,
        rows: body,
        footer,
        ..
    } = screen;

    let progress = app.progress();
    let indexing = if progress.is_complete() {
        String::new()
    } else {
        format!(" ─ indexing {}%", progress.percent())
    };
    let notice = if app.format_unrecognized() {
        " ─ format not recognized, showing raw"
    } else {
        ""
    };
    let key_notice = app
        .notice()
        .map_or(String::new(), |notice| format!(" ─ {}", printable(notice)));
    let matches = app
        .match_count()
        .map_or(String::new(), |(position, total)| {
            format!(" ─ match {position}/{total}")
        });
    let (status, after_dot) = follow_status(app);
    let before_dot = format!(
        "vilog ─ {} ─ {} ─ {}{status} ",
        printable(app.file_name()),
        human_size(app.size()),
        app.settings().format.name,
    );
    let after_dot = format!("{after_dot}{notice}{indexing}{matches}{key_notice}");
    let bar = Style::new().reversed();
    let (dot, colour) = liveness_dot(app.liveness());
    let title_width = before_dot.width() + dot.width() + after_dot.width();
    let title = Line::from(vec![
        Span::raw(before_dot),
        Span::styled(dot, theme.paint(bar.bold().bg(colour))),
        Span::raw(after_dot),
    ]);
    frame.render_widget(title.style(theme.paint(bar.bold())), header);
    let counters = level_counters(app.level_counts());
    let room = usize::from(area.width).saturating_sub(title_width + 1);
    if !counters.is_empty() && counters.width() <= room {
        frame.render_widget(
            Line::from(counters)
                .right_aligned()
                .style(theme.paint(bar.bold())),
            header,
        );
    }

    let fits = screen.fits;
    if let (Some(bar), Some(filters)) = (screen.filters, filters) {
        frame.render_widget(Line::from(filters), bar);
    }
    if let (Some(area), Some((row, _))) = (screen.histogram, histogram) {
        frame.render_widget(row, area);
    }
    let lines = if !fits {
        frame.render_widget(Line::from(TOO_SMALL), body);
        Vec::new()
    } else if app.nothing_matches() {
        frame.render_widget(Line::from(NO_MATCHES), body);
        Vec::new()
    } else {
        table::render(frame, app, theme, body)
    };
    app.set_hit_map(HitMap {
        rows: body,
        lines,
        buckets: screen
            .histogram
            .zip(buckets)
            .map(|(row, width)| Rect { width, ..row }),
        details: screen.details,
    });
    if let Some(area) = screen.details {
        details::render(frame, app, area);
    }
    if fits && matches!(app.panel(), Some(Panel::Help(_))) {
        render_help_panel(frame, app, body);
    }
    let hints = match app.panel() {
        None => key_hints(app.keymap()).into(),
        Some(Panel::Levels(panel)) => {
            if fits {
                render_level_panel(frame, panel, body);
            }
            LEVEL_PANEL_HINTS.into()
        }
        Some(Panel::Classes(panel)) => {
            if fits {
                render_class_panel(frame, panel, body);
            }
            panel_hints(panel.error.as_deref(), CLASS_PANEL_HINTS)
        }
        Some(Panel::Time(panel)) => {
            if fits {
                render_time_panel(frame, panel, body);
            }
            panel_hints(panel.error.as_deref(), TIME_PANEL_HINTS)
        }
        Some(Panel::Text(prompt)) => {
            if fits {
                render_prompt(frame, prompt, " Text filter ", body);
            }
            panel_hints(prompt.error.as_deref(), TEXT_PROMPT_HINTS)
        }
        Some(Panel::Jump(prompt)) => {
            let opener = match prompt.to {
                Jump::Line => ":",
                Jump::Time => "t ",
            };
            let error = prompt.error.as_deref().map(|error| format!(" ─ {error}"));
            format!("{opener}{}{}", prompt.text, error.unwrap_or_default()).into()
        }
        Some(Panel::Export(prompt)) if prompt.overwrite => {
            format!("overwrite {}? y/n", printable(&prompt.text)).into()
        }
        Some(Panel::Export(prompt)) => {
            let error = prompt.error.as_deref().map(|error| format!(" ─ {error}"));
            format!(
                "export to: {}{}",
                printable(&prompt.text),
                error.unwrap_or_default()
            )
            .into()
        }
        Some(Panel::Search { prompt, backward }) => {
            if fits {
                let title = if *backward {
                    " Search backward "
                } else {
                    " Search "
                };
                render_prompt(frame, prompt, title, body);
            }
            panel_hints(prompt.error.as_deref(), SEARCH_PROMPT_HINTS)
        }
        Some(Panel::Help(_)) => help_hints(app.keymap()).into(),
    };
    frame.render_widget(Line::from(hints).style(theme.paint(bar)), footer);
}

fn first_key(keymap: &Keymap, action: Action) -> String {
    keymap
        .bindings()
        .iter()
        .find(|binding| binding.action == action)
        .map(|binding| keymap::key_name(binding.code, binding.modifiers))
        .unwrap_or_default()
}

/// The first keys of two actions as `a/b`, with `Ctrl-u/d` for `Ctrl-u/Ctrl-d`.
fn pair(keymap: &Keymap, first: Action, second: Action) -> String {
    let (first, second) = (first_key(keymap, first), first_key(keymap, second));
    match second.strip_prefix("Ctrl-") {
        Some(rest) if first.starts_with("Ctrl-") => format!("{first}/{rest}"),
        _ => format!("{first}/{second}"),
    }
}

fn key_hints(keymap: &Keymap) -> String {
    format!(
        "{} Move  {} Page  {} Half page  {} Top/Bottom  {} Quit",
        pair(keymap, Action::Down, Action::Up),
        pair(keymap, Action::PageUp, Action::PageDown),
        pair(keymap, Action::HalfPageUp, Action::HalfPageDown),
        pair(keymap, Action::Top, Action::Bottom),
        first_key(keymap, Action::Quit),
    )
}

fn help_hints(keymap: &Keymap) -> String {
    format!(
        "{} Scroll  {} Page  {}/{}/q Close",
        pair(keymap, Action::Down, Action::Up),
        pair(keymap, Action::PageUp, Action::PageDown),
        first_key(keymap, Action::Help),
        first_key(keymap, Action::Cancel),
    )
}

fn render_help_panel(frame: &mut Frame, app: &mut App, body: Rect) {
    let help = app.keymap().help();
    let height = u16::try_from(help.len() + 2).map_or(body.height, |h| h.min(body.height));
    let keys_width = help.iter().map(|(keys, _)| keys.width()).max().unwrap_or(0);
    let widest = help.iter().map(|(_, d)| d.width()).max().unwrap_or(0);
    // A space, the keys, two spaces, the description, and the two borders.
    let width = u16::try_from(keys_width + widest + 5).map_or(body.width, |w| w.min(body.width));
    let area = body.centered(Constraint::Length(width), Constraint::Length(height));
    let inner = usize::from(area.height.saturating_sub(2));
    app.fit_help(inner);
    let scroll = match app.panel() {
        Some(Panel::Help(panel)) => panel.scroll,
        _ => 0,
    };
    let lines: Vec<Line> = help
        .iter()
        .skip(scroll)
        .take(inner)
        .map(|(keys, description)| Line::from(format!(" {keys:<keys_width$}  {description}")))
        .collect();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(Block::bordered().title(" Help ")),
        area,
    );
}

fn render_level_panel(frame: &mut Frame, panel: &LevelPanel, body: Rect) {
    let lines: Vec<Line> = Level::ALL
        .iter()
        .zip(1..)
        .map(|(&level, key)| {
            let cursor = if level == panel.cursor { '▶' } else { ' ' };
            let check = if panel.levels.is_on(level) { 'x' } else { ' ' };
            Line::from(format!(" {cursor}[{check}] {key} {:<5} ", level.name()))
        })
        .collect();
    let area = body.centered(Constraint::Length(16), Constraint::Length(8));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(Block::bordered().title(" Levels ")),
        area,
    );
}

fn render_class_panel(frame: &mut Frame, panel: &ClassPanel, body: Rect) {
    let lines: Vec<Line> = panel
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let cursor = if i == panel.cursor { '▶' } else { ' ' };
            Line::from(format!(" {cursor}{}", printable(line)))
        })
        .collect();
    let area = body.centered(Constraint::Length(44), Constraint::Length(10));
    // Keeps the cursor's line in view once there are more lines than the box holds.
    let scroll = panel
        .cursor
        .saturating_sub(usize::from(area.height.saturating_sub(3)));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::bordered().title(" Loggers "))
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        area,
    );
}

/// The footer of a panel: why what it edits doesn't apply, if it doesn't, else its
/// key hints.
fn panel_hints<'a>(error: Option<&'a str>, hints: &'static str) -> Cow<'a, str> {
    error.map_or(hints.into(), printable)
}

fn render_time_panel(frame: &mut Frame, panel: &TimePanel, body: Rect) {
    // What the spec makes, once it makes something.
    let filter = TimeSpec::parse(&panel.spec).map(|spec| {
        TimeFilter {
            spec,
            base: panel.base,
        }
        .to_string()
    });
    let lines = vec![
        Line::from(format!(" ▶{}", printable(&panel.spec))),
        Line::from(format!("  {}", filter.unwrap_or_default())),
    ];
    let area = body.centered(Constraint::Length(40), Constraint::Length(4));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(Block::bordered().title(" Time ")),
        area,
    );
}

fn render_prompt(frame: &mut Frame, prompt: &Prompt, title: &str, body: Rect) {
    let check = |on| if on { 'x' } else { ' ' };
    let lines = vec![
        Line::from(format!(" ▶{}", printable(&prompt.pattern))),
        Line::from(format!(
            "  [{}] regex  [{}] ignore case",
            check(prompt.options.regex),
            check(prompt.options.ignore_case)
        )),
    ];
    let area = body.centered(Constraint::Length(60), Constraint::Length(4));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(Block::bordered().title(title)),
        area,
    );
}

/// The header's level counters, FATAL first, without the levels that have no entries.
fn level_counters(counts: [u32; 6]) -> String {
    let counters: Vec<String> = Level::ALL
        .iter()
        .rev()
        .filter(|&&level| counts[level as usize] > 0)
        .map(|&level| format!("{}:{}", &level.name()[..1], counts[level as usize]))
        .collect();
    counters.join(" ")
}

/// The filter bar: the active filters, `None` while there are none.
fn filter_bar(app: &App) -> Option<String> {
    let filters = app.filters();
    let mut active = Vec::new();
    if filters.levels != Levels::default() {
        let shown: Vec<_> = Level::ALL
            .iter()
            .filter(|&&level| filters.levels.is_on(level))
            .map(|level| &level.name()[..1])
            .collect();
        active.push(format!("level[{}]", shown.join(" ")));
    }
    if filters.classes != Classes::default() {
        active.push(format!(
            "logger[{}]",
            printable(&filters.classes.rules().join(" "))
        ));
    }
    if let Some(time) = app.time_filter() {
        active.push(format!("time[{time}]"));
    }
    if let Some(text) = &filters.text {
        let options = text.options();
        let named: Vec<_> = [
            (options.regex, "regex"),
            (options.ignore_case, "ignore case"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        let named = if named.is_empty() {
            String::new()
        } else {
            format!(" ({})", named.join(", "))
        };
        active.push(format!("text[{}{named}]", printable(text.pattern())));
    }
    (!active.is_empty()).then(|| format!("Filters: {}", active.join("  ")))
}

/// The follow status of the header, with the ` ─ ` before it, split where the
/// liveness dot goes: what comes before it and what after.
fn follow_status(app: &App) -> (Cow<'static, str>, &'static str) {
    match app.origin() {
        Origin::Gzip => (" ─ READ-ONLY".into(), ""),
        Origin::TruncatedGzip => (" ─ READ-ONLY".into(), " ─ truncated .gz"),
        Origin::Plain => (plain_follow_status(app), ""),
    }
}

fn plain_follow_status(app: &App) -> Cow<'static, str> {
    if app.waiting_for_file() {
        " ─ WAITING FOR FILE".into()
    } else if app.following() {
        " ─ FOLLOW".into()
    } else {
        match app.new_entries() {
            0 => " ─ PAUSED".into(),
            n => format!(" ─ PAUSED +{n} new").into(),
        }
    }
}

/// The dot for a liveness and the colour it shows in on the reversed header.
fn liveness_dot(liveness: Liveness) -> (&'static str, Color) {
    match liveness {
        Liveness::Live => ("●", Color::Green),
        Liveness::Quiet => ("◐", Color::Yellow),
        Liveness::Idle => ("○", Color::DarkGray),
    }
}

/// A byte count for people, 1024-based with one decimal: `0 B`, `1023 B`, `1.0 KB`,
/// `1.8 GB`, up to `EB`. Rounds to the nearest tenth; a value that rounds to 1024 is
/// shown in the next unit.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // At most 6, as u64::MAX is just under 16 EB, so it always indexes UNITS.
    let mut power = bytes.ilog(1024);
    let scale = 1024u128.pow(power);
    // u128, so that multiplying by 10 can't overflow.
    let mut tenths = (u128::from(bytes) * 10 + scale / 2) / scale;
    if tenths == 10 * 1024 {
        power += 1;
        tenths = 10;
    }
    format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[power as usize])
}

/// Text that is safe to put on the screen: tabs expand to the next 4-column tab stop
/// (wide characters count as two columns) and every other control character, C0, DEL
/// or C1, becomes `�`. Text without either is returned as is.
pub fn printable(text: &str) -> Cow<'_, str> {
    printable_from(text, 0).0
}

/// [`printable`] for text that starts at `column`, and the column it ends at, so that
/// text shown piece by piece keeps its tab stops.
fn printable_from(text: &str, column: usize) -> (Cow<'_, str>, usize) {
    let width = |c: char| c.width().unwrap_or(0);
    if !text.chars().any(char::is_control) {
        return (
            Cow::Borrowed(text),
            column + text.chars().map(width).sum::<usize>(),
        );
    }
    let mut shown = String::with_capacity(text.len());
    let mut column = column;
    for c in text.chars() {
        if c == '\t' {
            let stop = (column + 1).next_multiple_of(TAB_STOP);
            shown.extend(iter::repeat_n(' ', stop - column));
            column = stop;
        } else {
            let c = if c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            };
            shown.push(c);
            column += width(c);
        }
    }
    (Cow::Owned(shown), column)
}
