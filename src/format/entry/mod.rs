//! Grouping lines into entries: an entry is a line that starts one, followed by the
//! lines that don't (stack traces, wrapped messages, blank lines).

use std::ops::Range;
use std::sync::LazyLock;

use regex::bytes::{CaptureLocations, Regex};

use super::{EntryHead, EntryParser, Format, strip_ansi};

#[cfg(test)]
mod tests;

/// One log entry: its first line and the continuation lines after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's lines, as numbers of the lines fed to the [`EntryGrouper`], from 0.
    /// Never empty: lines `lines.start + 1..lines.end` are the continuation lines.
    pub lines: Range<usize>,
    /// The columns of the first line, with spans into that line after ANSI stripping.
    /// `None` for the headless entry made of the lines before the first entry start,
    /// and for every entry under `Raw`.
    pub head: Option<EntryHead>,
    /// Whether a continuation line is an exception line (see [`is_exception_line`]).
    /// The first line of an entry with a head starts with its timestamp and is not
    /// checked. A headless entry is the tail of one never seen (a file that
    /// starts mid-trace), so its first line is checked too. Under `Raw` no entry has
    /// an exception.
    pub has_exception: bool,
}

/// Groups lines into entries as they come, in file order. The last entry stays open
/// until a line starts the next one, since more continuation lines may follow.
#[derive(Debug)]
pub struct EntryGrouper {
    /// The parser of entry starts and the room it finds columns in, kept from line to
    /// line. `None` under `Raw`.
    parser: Option<(EntryParser, CaptureLocations)>,
    open: Option<Entry>,
}

impl EntryGrouper {
    /// A grouper for `format`'s entry-start lines. Under `raw` every line is an entry.
    pub fn new(format: &'static Format) -> Self {
        EntryGrouper {
            parser: format
                .entry_parser()
                .map(|parser| (parser, parser.locations())),
            open: None,
        }
    }

    /// Takes the next line, as in the file but without its line ending; ANSI escape
    /// codes are stripped before it is looked at. If the line starts a new entry, the
    /// entry it closes is returned.
    pub fn push(&mut self, line: &[u8]) -> Option<Entry> {
        // Under `Raw` the line's content doesn't matter, so it isn't even stripped.
        if self.parser.is_none() {
            return self.start(None, false);
        }
        self.push_stripped(&strip_ansi(line))
    }

    /// [`push`](Self::push) for a line that is already ANSI-stripped, for a caller
    /// that strips into a buffer of its own and reads the line as well.
    pub fn push_stripped(&mut self, line: &[u8]) -> Option<Entry> {
        let Some((parser, locations)) = &mut self.parser else {
            return self.start(None, false);
        };
        if let Some(head) = parser.parse_with(line, locations) {
            return self.start(Some(head), false);
        }
        match &mut self.open {
            Some(entry) => {
                entry.lines.end += 1;
                // Once flagged, the rest of a long stack trace needn't be matched.
                entry.has_exception = entry.has_exception || is_exception_line(line);
                None
            }
            None => self.start(None, is_exception_line(line)),
        }
    }

    /// The last entry, which later lines may still extend. `None` before any line.
    pub fn open_entry(&self) -> Option<&Entry> {
        self.open.as_ref()
    }

    /// Opens an entry at the next line and returns the one it closes.
    fn start(&mut self, head: Option<EntryHead>, has_exception: bool) -> Option<Entry> {
        let first = self.open.as_ref().map_or(0, |entry| entry.lines.end);
        self.open.replace(Entry {
            lines: first..first + 1,
            head,
            has_exception,
        })
    }
}

/// Whether `line` looks like part of a stack trace (PRD §6.5). Java and Node.js: a
/// frame (`\tat …`), a cause (`Caused by: …`), omitted frames (`\t... 5 more`) or an
/// exception header (`java.lang.IllegalStateException: …`). Python: `Traceback`, `File`
/// and chaining lines. Go: `panic: `, `goroutine N [`, tab-indented `.go:N` frames, the
/// function line above each frame (`main.run(0xc000012345, {0x1, 0x2})`, `panic(…)`),
/// `created by …`, `[signal SIG…: …]` and `...additional frames elided...`.
/// .NET: ` ---> ` and the `--- End of …` markers. `line` has no line ending and is
/// already ANSI-stripped.
pub fn is_exception_line(line: &[u8]) -> bool {
    // Anchored, so matching stops at the first byte no alternative can take: a long
    // line costs no more than its leading whitespace and first word. Unlike the preset
    // patterns it keeps Unicode on, so `\w` takes non-ASCII Java identifiers.
    static EXCEPTION_LINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^(?:\s+at ",
            r"|Caused by: ",
            r"|\s+\.\.\. \d+ (?:more|common frames omitted)",
            r"|[\w.$]+(?:Exception|Error)(?:: |$)",
            r"|Traceback \(most recent call last\):",
            r#"|\s+File "[^"]*", line \d+"#,
            r"|During handling of the above exception",
            r"|The above exception was the direct cause",
            r"|panic: ",
            r"|goroutine \d+ \[",
            r"|\t/[^:\s]*\.go:\d+",
            r"|\s+---> ",
            r"|\s*--- End of (?:inner exception stack trace|stack trace from previous location) ---)",
        ))
        .expect("the exception pattern is a valid regex")
    });
    EXCEPTION_LINE.is_match(line) || is_go_trace_line(line)
}

/// The Go trace lines the regex can't tell from prose. Each check starts with a byte or
/// two that most lines fail, so ordinary lines cost next to nothing.
fn is_go_trace_line(line: &[u8]) -> bool {
    if let Some(rest) = line.strip_prefix(b"created by ") {
        return is_created_by_target(rest);
    }
    if line.starts_with(b"[signal SIG") {
        return line.ends_with(b"]");
    }
    line == b"...additional frames elided..." || is_go_function_line(line)
}

/// `main.main in goroutine 1`: a function, and since Go 1.21 the goroutine that
/// started it.
fn is_created_by_target(target: &[u8]) -> bool {
    let name_len = target
        .iter()
        .position(|&b| b == b' ')
        .unwrap_or(target.len());
    let (name, rest) = target.split_at(name_len);
    let goroutine = rest.strip_prefix(b" in goroutine ");
    is_go_function_name(name)
        && (rest.is_empty()
            || goroutine.is_some_and(|id| !id.is_empty() && id.iter().all(u8::is_ascii_digit)))
}

/// `pkg.(*T).method(0xc000012345, {0x4b2f10, 0x2})`. Go prints every argument as hex
/// words, so any other argument (`order.close(42)`, a Java frame) rules the line out.
fn is_go_function_line(line: &[u8]) -> bool {
    let Some(call) = line.strip_suffix(b")") else {
        return false;
    };
    // Arguments hold no parentheses, so the last `(` opens them.
    let Some(open) = call.iter().rposition(|&b| b == b'(') else {
        return false;
    };
    let (name, arguments) = (&call[..open], &call[open + 1..]);
    is_go_function_name(name)
        && arguments
            .split(|b| matches!(b, b' ' | b',' | b'{' | b'}'))
            .all(is_go_argument)
}

fn is_go_argument(word: &[u8]) -> bool {
    let word = word.strip_suffix(b"?").unwrap_or(word);
    let hex = word.strip_prefix(b"0x");
    word.is_empty()
        || word == b"..."
        || hex.is_some_and(|digits| !digits.is_empty() && digits.iter().all(u8::is_ascii_hexdigit))
}

/// A qualified function name, with a `(*T)` receiver and `[...]` type arguments, or
/// the built-in `panic`.
fn is_go_function_name(name: &[u8]) -> bool {
    if name == b"panic" {
        return true;
    }
    let mut rest = name;
    let mut qualified = false;
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        match byte {
            b'.' => qualified = true,
            b'(' => {
                // The receiver: `(*Entry)`.
                let Some(close) = rest.iter().position(|&b| b == b')') else {
                    return false;
                };
                let receiver = &rest[..close];
                let receiver = receiver.strip_suffix(b"[...]").unwrap_or(receiver);
                let ident = receiver.strip_prefix(b"*").unwrap_or(receiver);
                if ident.is_empty()
                    || !ident
                        .iter()
                        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return false;
                }
                rest = &rest[close + 1..];
            }
            b'/' | b'_' | b'-' | b'[' | b']' | b'*' => {}
            _ if byte.is_ascii_alphanumeric() || byte >= 0x80 => {}
            _ => return false,
        }
    }
    qualified
}
