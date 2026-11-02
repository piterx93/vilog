//! The rows between the header and the footer: each entry's first line, cut into the
//! columns of its preset, or whole under `Raw`, and the continuation lines of the
//! unfolded entries.

use std::borrow::Cow;
use std::iter;
use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Theme, abbreviate, printable, printable_from, structured};
use crate::app::{App, Search, ShownLine};
use crate::format::{Column, EntryParser, Mapping, Syntax, strip_ansi};
use crate::source;

/// The width of the `▶` marker of the selected row.
const MARKER: usize = 1;
/// The narrowest the line numbers are, so that most files never widen them.
const MIN_NUMBER_WIDTH: usize = 4;
const LEVEL_WIDTH: usize = 5;
const THREAD_WIDTH: usize = 15;
const MIN_LOGGER_WIDTH: usize = 24;
const MAX_LOGGER_WIDTH: usize = 40;
/// The narrowest screen whose timestamps show the date.
const DATE_FROM_WIDTH: usize = 100;
/// Timestamp formats and the width of what they print.
const TIME: (&str, usize) = ("%H:%M:%S%.3f", 12);

/// Draws the rows on screen into `body`, reading only their entries. Sets the app's
/// screen to the rows that fit and keeps its scroll from running past the widest
/// message on screen. Returns the row drawn on each screen line.
pub(super) fn render(frame: &mut Frame, app: &mut App, theme: Theme, body: Rect) -> Vec<usize> {
    let columns = Columns::new(app, usize::from(body.width));
    let height = usize::from(body.height);
    if app.wraps() {
        fit_wrapped(app, &columns, theme, height);
    } else {
        app.resize(height);
    }

    let viewport = app.viewport();
    let top = viewport.top();
    let shown = top..viewport.total().min(top + height);
    let rows: Vec<Row> = shown.map(|i| columns.row(app, i, theme)).collect();
    // Wrapped, every message is seen whole.
    let overflow = if app.wraps() {
        0
    } else {
        let widest = rows.iter().map(|row| row.text.width()).max();
        widest.unwrap_or(0).saturating_sub(columns.message)
    };
    app.limit_scroll(overflow);

    let selected = app.viewport().selected();
    let lines = (top..).zip(&rows).flat_map(|(i, row)| {
        let selected = selected == Some(i);
        let style = if selected {
            theme.paint(Style::new().reversed())
        } else {
            Style::new()
        };
        let lines = row.lines(&columns, app, selected, theme);
        lines.into_iter().map(move |line| (i, line.style(style)))
    });
    let mut drawn = Vec::new();
    for ((i, line), area) in lines.zip(body.rows()) {
        frame.render_widget(line, area);
        drawn.push(i);
    }
    drawn
}

/// Wrapped rows take as many screen lines as their messages need, so how many rows
/// fit depends on which rows they are. Gives the app's screen the rows that fit from
/// its top, or those that fit ending with the selection when it lies below them, or
/// ending with the last row when they reach it, so that the screen doesn't run past
/// the last row.
fn fit_wrapped(app: &mut App, columns: &Columns, theme: Theme, height: usize) {
    let fitting = |rows: &mut dyn Iterator<Item = usize>| {
        let mut used = 0;
        rows.take_while(|&i| {
            // A row taller than the screen shows its first lines.
            used += columns.row(app, i, theme).height(columns).min(height);
            used <= height
        })
        .count()
    };
    let viewport = app.viewport();
    let (top, total) = (viewport.top(), viewport.total());
    let mut rows = fitting(&mut (top..total));
    if top + rows == total {
        rows = fitting(&mut (0..total).rev());
    } else if let Some(selected) = viewport.selected()
        && selected >= top + rows
    {
        rows = fitting(&mut (0..=selected).rev());
    }
    app.resize(rows);
}

/// How wide the columns are on a screen of some width, in terminal columns.
struct Columns {
    /// The width of the line numbers, `None` while they are hidden.
    numbers: Option<usize>,
    /// The width of the line numbers with the space after them and the marker.
    gutter: usize,
    /// `None` under `Raw`, which has only the message column.
    table: Option<Table>,
    /// The width of the columns between the gutter and the message.
    lead: usize,
    /// The width of the message column.
    message: usize,
}

/// The columns before the message, under a preset that has them. A width is `None`
/// for a column the format doesn't have, or the thread while it is hidden.
struct Table {
    parser: EntryParser,
    /// How the lines are written and the keys of the columns, for a JSON lines or
    /// logfmt file.
    structured: Option<(Syntax, Mapping)>,
    timestamp_format: String,
    timestamp: Option<usize>,
    level: Option<usize>,
    thread: Option<usize>,
    logger: Option<usize>,
}

impl Table {
    /// The width of the columns before the message, each with the space after it.
    fn width(&self) -> usize {
        [self.timestamp, self.level, self.thread, self.logger]
            .into_iter()
            .flatten()
            .map(|width| width + 1)
            .sum()
    }
}

impl Columns {
    fn new(app: &App, width: usize) -> Self {
        let table = app.settings().format.entry_parser().map(|parser| {
            let (timestamp_format, timestamp) = if width < DATE_FROM_WIDTH {
                (TIME.0.to_owned(), TIME.1)
            } else {
                let timestamps = app.timestamps();
                (timestamps.format.clone(), timestamps.width)
            };
            let format = app.settings().format;
            let column = |column, width| format.has(column).then_some(width);
            Table {
                parser,
                structured: format
                    .kind
                    .syntax()
                    .map(|syntax| (syntax, app.settings().mapping)),
                timestamp_format,
                timestamp: column(Column::Timestamp, timestamp),
                level: column(Column::Level, LEVEL_WIDTH),
                thread: column(Column::Thread, THREAD_WIDTH).filter(|_| app.shows_thread()),
                logger: column(
                    Column::Logger,
                    (width * 3 / 10).clamp(MIN_LOGGER_WIDTH, MAX_LOGGER_WIDTH),
                ),
            }
        });
        let lead = table.as_ref().map_or(0, Table::width);
        // As wide as the file's last line needs, rather than the lines on screen, so
        // that the columns don't move while scrolling.
        let numbers = app.shows_line_numbers().then(|| {
            let digits = app
                .last_line_number()
                .map_or(0, |last| last.to_string().len());
            digits.max(MIN_NUMBER_WIDTH)
        });
        let gutter = numbers.map_or(0, |numbers| numbers + 1) + MARKER;
        Columns {
            numbers,
            gutter,
            table,
            lead,
            message: width.saturating_sub(gutter + lead),
        }
    }

    /// Row `i` of `app`, its text made safe to show.
    fn row(&self, app: &App, i: usize, theme: Theme) -> Row {
        let shown = app.row(i);
        let search = app.search().filter(|_| theme == Theme::Styled);
        let mut row = match shown {
            Some(shown) => self.entry_row(app, shown, search, theme),
            None => Row::new(Vec::new(), app.line(i).into_owned(), 0),
        };
        row.number = shown.and_then(|shown| shown.number);
        row.bookmarked = shown.is_some_and(|shown| shown.first) && app.bookmarked(i);
        if search.is_some() && app.hides_match(i) {
            // Past the space that sets the marker apart from the message.
            row.highlights.push(row.marker + 1..row.text.len());
        }
        row
    }

    fn entry_row(&self, app: &App, shown: ShownLine, search: Option<&Search>, theme: Theme) -> Row {
        // `Raw` doesn't look into lines, so escape codes are shown, made harmless.
        let Some(table) = &self.table else {
            let row = Row::new(Vec::new(), text(shown.line), shown.hidden);
            return row.highlighted(search, shown.line, 0..shown.line.len());
        };
        let line = strip_ansi(shown.line);
        let parts = shown.first.then(|| table.parts(&line)).flatten();
        let Some(parts) = parts else {
            // Continuation lines and the headless entry have nothing but a message.
            let blank = Span::raw(" ".repeat(self.lead));
            let row = Row::new(vec![blank], text(&line), shown.hidden);
            return row.highlighted(search, &line, 0..line.len());
        };
        let record = shown.record;
        let mut cells = Vec::new();
        if let Some(width) = table.timestamp {
            let timestamp = record.timestamp().map_or_else(String::new, |timestamp| {
                let local = app.settings().zone.to_datetime(timestamp);
                local.strftime(&table.timestamp_format).to_string()
            });
            cells.push(Span::raw(cell(&timestamp, width) + " "));
        }
        if let Some(width) = table.level {
            let level = record
                .level()
                .map_or(parts.level, |level| level.name().to_owned());
            let style = theme.paint(app.colors().level(record.level()));
            cells.push(Span::styled(cell(&level, width), style));
            cells.push(Span::raw(" "));
        }
        if let Some(width) = table.thread {
            cells.push(Span::raw(cell_keeping_end(&parts.thread, width) + " "));
        }
        if let Some(width) = table.logger {
            cells.push(Span::raw(
                cell(&abbreviate(&parts.logger, width), width) + " ",
            ));
        }
        let row = Row::new(
            cells,
            text(&parts.source[parts.message.clone()]),
            shown.hidden,
        );
        row.highlighted(search, &parts.source, parts.message)
    }
}

/// The columns of a first line that the table shows besides the timestamp, and the
/// message in `source[message]`.
struct Parts<'a> {
    level: String,
    thread: String,
    logger: String,
    source: Cow<'a, [u8]>,
    message: Range<usize>,
}

impl Table {
    /// The parts of `line` if it starts an entry, else `None`. A column the format has
    /// may still be empty in an entry.
    fn parts<'a>(&self, line: &'a [u8]) -> Option<Parts<'a>> {
        if let Some((syntax, mapping)) = &self.structured {
            let columns = mapping.text(*syntax, line)?;
            let message = structured::row_message(line, &columns);
            return Some(Parts {
                level: String::new(),
                thread: structured::column(line, columns.thread.as_ref()),
                logger: structured::column(line, columns.logger.as_ref()),
                message: 0..message.len(),
                source: message,
            });
        }
        let head = self.parser.parse(line)?;
        let part = |span: Option<Range<usize>>| span.map_or_else(String::new, |s| text(&line[s]));
        Some(Parts {
            level: part(head.level),
            thread: part(head.thread),
            logger: part(head.logger),
            source: Cow::Borrowed(line),
            message: head.message,
        })
    }
}

/// One row's text: the cells before the message, each padded to its column with the
/// space after it, and the message followed by the marker of the lines folded away.
struct Row {
    cells: Vec<Span<'static>>,
    text: String,
    /// Where the marker starts in `text`.
    marker: usize,
    /// The line's number in the file, if known.
    number: Option<u64>,
    /// Whether the row is the first line of a bookmarked entry.
    bookmarked: bool,
    /// The byte ranges of `text` that the search matches, in order.
    highlights: Vec<Range<usize>>,
}

impl Row {
    /// The row of `message` after `cells`, marked ` [+hidden]` unless no lines are
    /// hidden.
    fn new(cells: Vec<Span<'static>>, message: String, hidden: usize) -> Self {
        let marker = message.len();
        let mut text = message;
        if hidden > 0 {
            text += &format!(" [+{hidden}]");
        }
        Row {
            cells,
            text,
            marker,
            number: None,
            bookmarked: false,
            highlights: Vec::new(),
        }
    }

    /// The row with the matches of `search` in `line[message]`, the bytes that the
    /// row's message shows, highlighted.
    fn highlighted(mut self, search: Option<&Search>, line: &[u8], message: Range<usize>) -> Self {
        if let Some(search) = search {
            self.highlights = highlights(search, line, message, &self.text[..self.marker]);
        }
        self
    }

    /// `part`, which starts at byte `at` of the row's text, as spans.
    fn spans<'a>(&'a self, at: usize, part: &'a str, highlight: Style) -> Vec<Span<'a>> {
        let end = at + part.len();
        let mut spans = Vec::new();
        let mut done = at;
        for found in &self.highlights {
            let (start, stop) = (found.start.max(done), found.end.min(end));
            if start >= stop {
                continue;
            }
            spans.push(Span::raw(&self.text[done..start]));
            spans.push(Span::styled(&self.text[start..stop], highlight));
            done = stop;
        }
        spans.push(Span::raw(&self.text[done..end]));
        spans
    }

    /// How many screen lines the row takes when wrapped.
    fn height(&self, columns: &Columns) -> usize {
        wrap(&self.text, columns.message).len()
    }

    /// The row's screen lines: one with the message scrolled as far as the app says
    /// and cut where the marker still fits, or, wrapped, as many as the message and
    /// the marker need, the cells only on the first. The gutter of the first has the
    /// line number, if shown, and `▶` if the row is `selected`.
    fn lines(&self, columns: &Columns, app: &App, selected: bool, theme: Theme) -> Vec<Line<'_>> {
        let highlight = theme.paint(Style::new().fg(Color::Black).bg(Color::Yellow));
        let parts: Vec<Vec<Span>> = if app.wraps() {
            let parts = wrap(&self.text, columns.message);
            // The parts follow one another without gaps.
            let starts = parts.iter().scan(0, |at, part| {
                *at += part.len();
                Some(*at - part.len())
            });
            let spans = |(at, part)| self.spans(at, part, highlight);
            starts.zip(parts.iter().copied()).map(spans).collect()
        } else {
            let (message, marker) = self.text.split_at(self.marker);
            let width = columns.message.saturating_sub(marker.width());
            let scrolled = skip_columns(message, app.scroll());
            let kept = &scrolled[..fitting_bytes(scrolled, width)];
            let mut spans = self.spans(message.len() - scrolled.len(), kept, highlight);
            spans.extend(self.spans(self.marker, marker, highlight));
            vec![spans]
        };
        let mut parts = parts.into_iter();
        let number = columns.numbers.map(|width| {
            let number = self
                .number
                .map_or_else(String::new, |number| number.to_string());
            let style = theme.paint(app.colors().line_number);
            Span::styled(format!("{number:>width$} "), style)
        });
        let marker = Span::raw(match (selected, self.bookmarked) {
            (true, _) => "▶",
            (false, true) => "◆",
            (false, false) => " ",
        });
        let cells = self.cells.iter().cloned();
        let first = number
            .into_iter()
            .chain([marker])
            .chain(cells)
            .chain(parts.next().into_iter().flatten());
        let indent = " ".repeat(columns.gutter + columns.lead);
        let rest =
            parts.map(|part| Line::from_iter(iter::once(Span::raw(indent.clone())).chain(part)));
        iter::once(Line::from_iter(first)).chain(rest).collect()
    }
}

/// The byte ranges of `shown`, the text that `line[message]` is shown as, that `search`
/// matches in the message. A match is cut at the message's ends.
fn highlights(
    search: &Search,
    line: &[u8],
    message: Range<usize>,
    shown: &str,
) -> Vec<Range<usize>> {
    // Only what the display keeps is searched, which also bounds the work per row.
    let searched = &line[..message.end.min(message.start + source::DISPLAY_CAP)];
    // Without tabs, control characters or invalid UTF-8 the text is the bytes.
    let same = shown.as_bytes() == &line[message.clone()];
    // Where the text of the message up to a byte of the line ends. The matches come in
    // order, so the text is made piece by piece, keeping its column for the tab stops.
    let (mut done, mut column, mut length) = (message.start, 0, 0);
    let mut offset = |at: usize| {
        if same {
            return at - message.start;
        }
        let piece = source::display(&line[done..at]);
        let (piece, end) = printable_from(&piece, column);
        (done, column, length) = (at, end, length + piece.len());
        length
    };
    let boundary = |at: usize| shown.is_char_boundary(at);
    search
        .matcher
        .find(searched)
        .into_iter()
        .map(|found| found.start.max(message.start)..found.end.min(message.end))
        .filter(|found| found.start < found.end)
        .map(|found| {
            let start = offset(found.start);
            start..offset(found.end)
        })
        // The text can be cut short, and a regex can match half a character.
        .filter(|found| found.end <= shown.len() && boundary(found.start) && boundary(found.end))
        .collect()
}

/// `bytes` as text that is safe to show.
pub(super) fn text(bytes: &[u8]) -> String {
    printable(&source::display(bytes)).into_owned()
}

/// `text` in a column `width` wide: padded with spaces, or cut on the right with `…`
/// where it doesn't fit.
fn cell(text: &str, width: usize) -> String {
    if text.width() <= width {
        return pad(text, width);
    }
    let kept = take_columns(text.chars(), width.saturating_sub(1));
    pad(&format!("{kept}…"), width)
}

/// [`cell`] cutting on the left instead, which keeps the end of `text`.
fn cell_keeping_end(text: &str, width: usize) -> String {
    if text.width() <= width {
        return pad(text, width);
    }
    let kept: String = take_columns(text.chars().rev(), width.saturating_sub(1));
    let kept: String = kept.chars().rev().collect();
    pad(&format!("…{kept}"), width)
}

/// How many bytes of `text` fit into `width` columns.
fn fitting_bytes(text: &str, width: usize) -> usize {
    let mut used = 0;
    for (at, c) in text.char_indices() {
        used += c.width().unwrap_or(0);
        if used > width {
            return at;
        }
    }
    text.len()
}

/// The first of `chars` that fit into `width` columns.
fn take_columns(chars: impl Iterator<Item = char>, width: usize) -> String {
    let mut used = 0;
    chars
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

/// `text` followed by the spaces that make it `width` columns wide.
fn pad(text: &str, width: usize) -> String {
    let spaces = width.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(spaces))
}

/// `text` without its first `columns` columns. A wide character cut in two goes.
fn skip_columns(text: &str, columns: usize) -> &str {
    let mut skipped = 0;
    for (at, c) in text.char_indices() {
        if skipped >= columns {
            return &text[at..];
        }
        skipped += c.width().unwrap_or(0);
    }
    ""
}

/// `text` cut into lines of at most `width` columns, at least one. A character wider
/// than `width` gets a line of its own.
pub(super) fn wrap(text: &str, width: usize) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (at, c) in text.char_indices() {
        let columns = c.width().unwrap_or(0);
        if used + columns > width && at > start {
            lines.push(&text[start..at]);
            (start, used) = (at, 0);
        }
        used += columns;
    }
    lines.push(&text[start..]);
    lines
}
