//! Guessing a file's format from its first lines.

use std::borrow::Cow;

use super::mapping::is_logfmt_log_line;
use super::{BUILT_IN, Format, Kind, is_exception_line, strip_ansi};

#[cfg(test)]
mod tests;

const SAMPLE_LINES: usize = 200;
const MAX_LINES_READ: usize = 2000;

/// Guesses the format of a file from `lines`, the file's lines in order, without line
/// endings. `None` if no format fits.
///
/// The sample is the first 200 lines that aren't blank (empty, or only spaces and tabs)
/// once ANSI escape codes are stripped; no line past it is read. Blank lines don't
/// count toward the 200, but at most 2000 lines are read in all, so a file of blank
/// lines isn't read to its end. Continuation lines in the sample, those that start
/// with a space or a tab or are an exception line (see [`is_exception_line`]), are
/// left out. Each format in [`BUILT_IN`](super::BUILT_IN) but `raw` scores the share
/// of the remaining lines that start an entry in it; for logfmt a line also needs a
/// timestamp or level key and a message key (PRD §6.3). The best score wins if it is at
/// least 60%, ties going to the format that comes first in the table. Otherwise, and
/// when no line remains, it is `None`.
pub fn detect<'a>(lines: impl IntoIterator<Item = &'a [u8]>) -> Option<&'static Format> {
    let scored = sample(lines);
    BUILT_IN
        .iter()
        .filter_map(|format| Some((format, format.entry_parser()?)))
        .map(|(format, parser)| {
            let starts_entry = |line: &&Cow<[u8]>| match format.kind {
                Kind::Logfmt => is_logfmt_log_line(line),
                _ => parser.parse(line).is_some(),
            };
            let entries = scored.iter().filter(starts_entry);
            (format, entries.count())
        })
        // `max_by_key` keeps the last of equal maximums; reversed, that is the first
        // in the table.
        .rev()
        .max_by_key(|&(_, entries)| entries)
        // 60% in integers, so that exactly 60% isn't lost to rounding.
        .filter(|&(_, entries)| !scored.is_empty() && entries * 5 >= scored.len() * 3)
        .map(|(format, _)| format)
}

/// The lines [`detect`] scores; JSON key mapping reads the same ones.
pub(super) fn sample<'a>(lines: impl IntoIterator<Item = &'a [u8]>) -> Vec<Cow<'a, [u8]>> {
    // `take` stops pulling once it has enough, so the rest of the file is never read.
    lines
        .into_iter()
        .take(MAX_LINES_READ)
        .map(strip_ansi)
        .filter(|line| !is_blank(line))
        .take(SAMPLE_LINES)
        .filter(|line| !is_continuation(line))
        .collect()
}

fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|byte| matches!(byte, b' ' | b'\t'))
}

fn is_continuation(line: &[u8]) -> bool {
    matches!(line.first(), Some(b' ' | b'\t')) || is_exception_line(line)
}
