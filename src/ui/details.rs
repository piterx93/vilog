//! The details pane: the selected entry in full, wrapped at the pane's width.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};

use super::structured;
use super::table::{text, wrap};
use crate::app::{App, Focus};
use crate::format::strip_ansi;
use crate::source;

pub(super) fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let border = if app.focus() == Focus::Details {
        BorderType::Thick
    } else {
        BorderType::Plain
    };
    let block = Block::bordered()
        .border_type(border)
        .title(" Details ")
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let wrapped: Vec<String> = entry_lines(app)
        .iter()
        .flat_map(|line| wrap(line, usize::from(inner.width)))
        .map(str::to_owned)
        .collect();
    app.fit_details(usize::from(inner.height), wrapped.len());
    let lines: Vec<Line> = wrapped
        .into_iter()
        .skip(app.details_scroll())
        .take(usize::from(inner.height))
        .map(Line::from)
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}

/// How many bytes of an entry the pane reads, so that a huge entry costs no more per
/// frame than a large one.
pub(super) const DETAILS_CAP: usize = 1 << 20;

/// The lines of the selected entry made safe to show. Under a preset the first line
/// becomes the full logger and the thread, then the message.
fn entry_lines(app: &App) -> Vec<String> {
    let Some(entry) = app.details_entry() else {
        return Vec::new();
    };
    let mut shown = entry_head(app, &entry[..entry.len().min(DETAILS_CAP)]);
    if entry.len() > DETAILS_CAP {
        shown.push("…".to_owned());
    }
    shown
}

fn entry_head(app: &App, entry: &[u8]) -> Vec<String> {
    let mut lines = source::lines(entry);
    let Some(parser) = app.settings().format.entry_parser() else {
        return lines.map(text).collect();
    };
    let mut shown = Vec::new();
    if let Some(first) = lines.next() {
        let first = strip_ansi(first);
        let settings = app.settings();
        let structured = settings.format.kind.syntax().and_then(|syntax| {
            let columns = settings.mapping.text(syntax, &first)?;
            Some((syntax, columns))
        });
        if let Some((syntax, columns)) = structured {
            shown.extend(structured::details(syntax, &first, &columns));
        } else {
            match parser.parse(&first) {
                Some(head) => {
                    let logger = head.logger.map(|logger| text(&first[logger]));
                    let thread = head.thread.map(|thread| text(&first[thread]));
                    shown.extend(source_line(logger, thread));
                    shown.push(text(&first[head.message]));
                }
                None => shown.push(text(&first)),
            }
        }
    }
    shown.extend(lines.map(|line| text(&strip_ansi(line))));
    shown
}

/// The line above the message: the logger and the thread in parentheses, whichever
/// the entry has.
pub(super) fn source_line(logger: Option<String>, thread: Option<String>) -> Option<String> {
    match (logger, thread) {
        (Some(logger), Some(thread)) => Some(format!("{logger} ({thread})")),
        (Some(only), None) => Some(only),
        (None, Some(thread)) => Some(format!("({thread})")),
        (None, None) => None,
    }
}
