//! Filtering the index (PRD §9.3): which entries pass every active filter, and the
//! level counters of the header (PRD §7.1).
//!
//! The filters combine with AND. An [`Engine`] evaluates them on a background
//! thread, a batch of entries at a time (a chunk per core, evaluated in parallel),
//! and publishes what it found after each batch. Setting new filters starts a new
//! generation and cancels the one running.
//! Entries appended to the index are evaluated as they come, and an entry the index
//! read again is evaluated again.
//!
//! The level, time and class checks read the record and the class column of the
//! entry's first line; only the text check reads the rest of the entry. An engine
//! can also count a search: it then passes the entries that pass the filters and
//! have a line the search matches.

use std::fmt;
use std::num::NonZero;
use std::ops::Range;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Instant;

use jiff::Timestamp;
use memchr::memmem;
use regex::bytes::CaptureLocations;

use crate::format::{
    self, EntryParser, Format, Level, Mapping, Syntax, decoded_line, json::Field, strip_ansi_into,
};
use crate::index::Record;
use crate::matcher::Matcher;
use crate::source;

mod time;

pub use time::{Base, TimeFilter, TimeSpec};

#[cfg(test)]
mod tests;

/// The active filters. The default filters nothing out.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub levels: Levels,
    pub classes: Classes,
    /// Only entries in this time range, while set. Entries without a timestamp are
    /// hidden while it is set.
    pub time: Option<TimeRange>,
    /// Only entries whose message matches, while set: the message column of the
    /// first line and every continuation line, with ANSI escape codes stripped. An
    /// entry without columns (the headless entry, any entry under `Raw`) is matched
    /// over all its lines.
    pub text: Option<Matcher>,
}

/// Which levels are shown. All of them by default. An entry without a level is
/// shown only while every level is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Levels {
    hidden: [bool; 6],
}

impl Levels {
    /// Only `levels` shown, every other level hidden; none shown for an empty slice.
    pub fn only(levels: &[Level]) -> Levels {
        let mut only = Levels {
            hidden: [true; Level::ALL.len()],
        };
        for &level in levels {
            only.set(level, true);
        }
        only
    }

    /// Whether `level` is shown.
    pub fn is_on(&self, level: Level) -> bool {
        !self.hidden[level as usize]
    }

    /// Hides `level` if it is shown, shows it if it is hidden.
    pub fn toggle(&mut self, level: Level) {
        self.set(level, !self.is_on(level));
    }

    /// Shows `level` if `on`, else hides it.
    pub fn set(&mut self, level: Level, on: bool) {
        self.hidden[level as usize] = !on;
    }

    fn shows(&self, level: Option<Level>) -> bool {
        match level {
            Some(level) => self.is_on(level),
            None => !self.hidden.contains(&true),
        }
    }
}

/// The class filter: rules on the full class name of an entry's first line, matched
/// case-sensitively. An entry passes if no `exclude` rule matches its class and,
/// unless `include` is empty, at least one `include` rule does. An entry without a
/// class passes only while `include` is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classes {
    pub include: Vec<Rule>,
    pub exclude: Vec<Rule>,
}

/// One rule of the class filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Written `com.myco.*`: the class `com.myco` and every class starting with
    /// `com.myco.`, but not `com.mycompany`.
    Prefix(String),
    /// Written `OrderService`: every class containing it.
    Substring(String),
}

impl Classes {
    /// The rules `rules` name, one per item: `com.myco.*` is a prefix rule,
    /// `OrderService` a substring rule, and `!` in front of either makes it an
    /// exclude. Whitespace around an item and after its `!` is ignored, empty items
    /// are skipped and a rule given twice counts once. An item that names no class
    /// (`!`, `*`, `.*`), or has a `*` anywhere but in a trailing `.*`, is an error that
    /// names it.
    pub fn parse<'a>(rules: impl IntoIterator<Item = &'a str>) -> Result<Classes, String> {
        let mut classes = Classes::default();
        for item in rules
            .into_iter()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let (rules, written) = match item.strip_prefix('!') {
                Some(written) => (&mut classes.exclude, written.trim_start()),
                None => (&mut classes.include, item),
            };
            let rule = Rule::parse(written).map_err(|why| format!("logger rule '{item}' {why}"))?;
            if !rules.contains(&rule) {
                rules.push(rule);
            }
        }
        Ok(classes)
    }

    /// The rules as [`parse`](Self::parse) reads them, one per item: the includes,
    /// then the excludes, each in order.
    pub fn rules(&self) -> Vec<String> {
        let include = self.include.iter().map(Rule::to_string);
        let exclude = self.exclude.iter().map(|rule| format!("!{rule}"));
        include.chain(exclude).collect()
    }

    /// Whether an entry whose class is `class`, `None` for an entry without one,
    /// passes the rules.
    pub fn passes(&self, class: Option<&[u8]>) -> bool {
        let matched = |rule: &Rule| class.is_some_and(|class| rule.matches(class));
        !self.exclude.iter().any(matched)
            && (self.include.is_empty() || self.include.iter().any(matched))
    }

    fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }
}

impl Rule {
    /// The rule `written` names, without the `!` of an exclude; the error says why
    /// it names none.
    fn parse(written: &str) -> Result<Rule, &'static str> {
        let (name, rule): (_, fn(String) -> Rule) = match written.strip_suffix(".*") {
            Some(package) => (package, Rule::Prefix),
            None => (written, Rule::Substring),
        };
        if name.chars().all(|c| c == '*') {
            Err("names no logger")
        } else if name.contains('*') {
            Err("has a '*' that is not a trailing '.*'")
        } else {
            Ok(rule(name.to_string()))
        }
    }

    fn matches(&self, class: &[u8]) -> bool {
        match self {
            Rule::Prefix(package) => class
                .strip_prefix(package.as_bytes())
                .is_some_and(|rest| rest.is_empty() || rest[0] == b'.'),
            Rule::Substring(part) => memmem::find(class, part.as_bytes()).is_some(),
        }
    }
}

/// As [`Classes::parse`] reads it, without the `!` of an exclude.
impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rule::Prefix(package) => write!(f, "{package}.*"),
            Rule::Substring(part) => f.write_str(part),
        }
    }
}

/// From `start` (inclusive) to `end` (exclusive), or open-ended without `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub start: Timestamp,
    pub end: Option<Timestamp>,
}

impl TimeRange {
    fn contains(&self, timestamp: Option<Timestamp>) -> bool {
        timestamp.is_some_and(|at| at >= self.start && self.end.is_none_or(|end| at < end))
    }
}

/// A file and its index, shared between the viewer, which keeps them up to date,
/// and the engine, which reads them a chunk at a time.
#[derive(Debug)]
pub struct Indexed<B> {
    /// The records of the file's entries, as the index gives them.
    pub records: Vec<Record>,
    /// The file the records index.
    pub bytes: Arc<B>,
}

/// How an [`Engine`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Entries per chunk and thread, at least 1. The engine evaluates a batch of
    /// one chunk per thread at once; results are published and cancellation is
    /// noticed between batches.
    pub chunk: usize,
}

/// What the current generation found so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filtered {
    /// How many entries, from the first on, are evaluated.
    pub evaluated: usize,
    /// The evaluated entries that pass every filter, as indices into the records,
    /// in ascending order.
    pub entries: Vec<u32>,
    /// Per level, indexed by `Level as usize`: the evaluated entries of that level
    /// that pass every filter but the level filter.
    pub counts: [u32; 6],
}

/// What [`evaluate`] found in a run of entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Evaluated {
    /// The entries of the run that pass every filter, and the search if there is
    /// one, as indices into the records, in ascending order.
    pub entries: Vec<u32>,
    /// Per entry of the run, in order: its level if it passes every filter but the
    /// level filter, which is what it adds to [`Filtered::counts`].
    pub counted: Vec<Option<Level>>,
}

/// Evaluates `filters`, and `search` if given, over the entries `entries` of
/// `indexed`, read as `reader` says, as an [`Engine`] does: the run is split into parts
/// evaluated on up to `threads` threads at once (0 counts as 1). The result is the
/// same for any number of threads.
fn evaluate<B: AsRef<[u8]> + Sync>(
    indexed: &Indexed<B>,
    reader: Reader,
    filters: &Filters,
    search: Option<&Matcher>,
    entries: Range<usize>,
    threads: usize,
) -> Evaluated {
    let check = Check { filters, search };
    // Borrowed apart because `Indexed` as a whole would need `B: Send` to cross threads.
    let (records, bytes) = (&indexed.records[..], indexed.bytes.as_ref().as_ref());
    let run = |part: Range<usize>| {
        let mut scratch = Scratch::new(reader);
        check.run(records, bytes, part, &mut scratch)
    };
    let part_len = entries.len().div_ceil(threads.max(1)).max(1);
    let parts: Vec<Range<usize>> = (entries.start..entries.end)
        .step_by(part_len)
        .map(|start| start..entries.end.min(start + part_len))
        .collect();
    let Some((last, rest)) = parts.split_last() else {
        return Evaluated::default();
    };
    thread::scope(|scope| {
        let handles: Vec<_> = rest
            .iter()
            .map(|part| scope.spawn(|| run(part.clone())))
            .collect();
        // The calling thread takes a part too, instead of only waiting.
        let mut found = Vec::with_capacity(parts.len());
        let tail = run(last.clone());
        for handle in handles {
            // A panic in a part is a bug in the filters; pass it on.
            found.push(
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            );
        }
        found.push(tail);
        found
            .into_iter()
            .fold(Evaluated::default(), |mut all, part| {
                all.entries.extend(part.entries);
                all.counted.extend(part.counted);
                all
            })
    })
}

/// How a file's entries are read: its format and, for JSON lines and logfmt, the
/// keys its columns are in.
#[derive(Debug, Clone, Copy)]
pub struct Reader {
    format: &'static Format,
    mapping: Mapping,
}

impl Reader {
    pub fn new(format: &'static Format, mapping: Mapping) -> Reader {
        Reader { format, mapping }
    }
}

/// Evaluates the filters over an index on a background thread. Dropping it stops
/// the thread.
#[derive(Debug)]
pub struct Engine {
    jobs: Sender<Job>,
    updates: Receiver<Update>,
    /// Bumped whenever the results start over, so that updates of an earlier job,
    /// still in the channel, are told apart and dropped.
    generation: u64,
    filters: Arc<Filters>,
    search: Option<Arc<Matcher>>,
    filtered: Filtered,
    /// Per evaluated entry, the level it adds to `filtered.counts`, so that entries
    /// read again can be taken out of them.
    counted: Vec<Option<Level>>,
}

impl Engine {
    /// Starts evaluating `filters` over the entries of `indexed`, read as `reader`
    /// says.
    pub fn spawn<B>(
        indexed: Arc<RwLock<Indexed<B>>>,
        reader: Reader,
        filters: Filters,
        options: Options,
    ) -> Engine
    where
        B: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self::spawn_with(indexed, reader, filters, None, options)
    }

    /// Like [`spawn`](Self::spawn), but an entry passes only if one of its lines, with
    /// ANSI escape codes stripped, also matches `search`.
    pub fn spawn_search<B>(
        indexed: Arc<RwLock<Indexed<B>>>,
        reader: Reader,
        filters: Filters,
        search: Matcher,
        options: Options,
    ) -> Engine
    where
        B: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self::spawn_with(indexed, reader, filters, Some(Arc::new(search)), options)
    }

    fn spawn_with<B>(
        indexed: Arc<RwLock<Indexed<B>>>,
        reader: Reader,
        filters: Filters,
        search: Option<Arc<Matcher>>,
        options: Options,
    ) -> Engine
    where
        B: AsRef<[u8]> + Send + Sync + 'static,
    {
        let threads = thread::available_parallelism().map_or(1, NonZero::get);
        let (jobs, job_receiver) = mpsc::channel();
        let (update_sender, updates) = mpsc::channel();
        let worker = Worker {
            indexed,
            reader,
            threads,
            chunk: options.chunk.max(1),
            jobs: job_receiver,
            updates: update_sender,
        };
        thread::spawn(move || worker.run());
        let mut engine = Engine {
            jobs,
            updates,
            generation: 0,
            filters: Arc::new(filters),
            search,
            filtered: Filtered::default(),
            counted: Vec::new(),
        };
        engine.start();
        engine
    }

    /// Starts a new generation with `filters`. The results start over from no
    /// entry evaluated, and nothing the previous generation finds is published.
    pub fn set_filters(&mut self, filters: Filters) {
        self.filters = Arc::new(filters);
        self.truncate(0);
        self.start();
    }

    /// Evaluates `filters` from entry `from` on, keeping the results for the entries
    /// before it as the filters before found them: for a change of filters that can
    /// only drop entries from `from` on, such as a time window whose start moves
    /// later. Results the earlier filters publish from then on are not taken in.
    pub fn set_filters_from(&mut self, filters: Filters, from: usize) {
        self.filters = Arc::new(filters);
        self.truncate(from);
        self.start();
    }

    /// Tells the engine that the records from `from` on were read again or are new,
    /// after the viewer changed them. From this call on, the results cover at most
    /// the first `from` entries until the others are evaluated again.
    pub fn reread(&mut self, from: usize) {
        self.truncate(from);
        self.start();
    }

    /// The results of the current generation so far, with whatever the thread
    /// published since the last call taken in. Once `evaluated` reaches the number
    /// of records, they are complete until the records or the filters change.
    pub fn poll(&mut self) -> &Filtered {
        while let Ok(update) = self.updates.try_recv() {
            if update.generation == self.generation {
                self.take_in(update);
            }
        }
        &self.filtered
    }

    /// [`poll`](Self::poll), but waiting for the thread's updates until at least
    /// `evaluated` entries are evaluated or `deadline` passes, whichever comes first.
    pub fn poll_until(&mut self, evaluated: usize, deadline: Instant) -> &Filtered {
        self.poll();
        while self.filtered.evaluated < evaluated {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(update) = self.updates.recv_timeout(left) else {
                break;
            };
            if update.generation == self.generation {
                self.take_in(update);
            }
        }
        &self.filtered
    }

    /// The results as the last [`poll`](Self::poll) left them, cut short by any
    /// [`reread`](Self::reread) or [`set_filters`](Self::set_filters) since.
    pub fn filtered(&self) -> &Filtered {
        &self.filtered
    }

    /// Hands the thread a new job from where the results end, which cancels the one
    /// it is on.
    fn start(&mut self) {
        self.generation += 1;
        // The thread only ends once `self.jobs` is dropped, so sending can't fail.
        let _ = self.jobs.send(Job {
            generation: self.generation,
            filters: Arc::clone(&self.filters),
            search: self.search.clone(),
            from: self.filtered.evaluated,
        });
    }

    fn truncate(&mut self, evaluated: usize) {
        if evaluated >= self.filtered.evaluated {
            return;
        }
        for level in self.counted.drain(evaluated..).flatten() {
            self.filtered.counts[level as usize] -= 1;
        }
        let entries = &mut self.filtered.entries;
        entries.truncate(entries.partition_point(|&entry| (entry as usize) < evaluated));
        self.filtered.evaluated = evaluated;
    }

    fn take_in(&mut self, update: Update) {
        let found = update.found;
        for level in found.counted.iter().flatten() {
            self.filtered.counts[*level as usize] += 1;
        }
        self.filtered.evaluated += found.counted.len();
        self.filtered.entries.extend(found.entries);
        self.counted.extend(found.counted);
    }
}

/// What the thread is to evaluate: the entries from `from` on.
struct Job {
    generation: u64,
    filters: Arc<Filters>,
    search: Option<Arc<Matcher>>,
    from: usize,
}

impl Job {
    fn reads_entries(&self) -> bool {
        self.search.is_some() || self.filters.text.is_some() || !self.filters.classes.is_empty()
    }
}

/// What the threads found in the batch of entries that follows the ones published
/// before in the same generation.
struct Update {
    generation: u64,
    found: Evaluated,
}

struct Worker<B> {
    indexed: Arc<RwLock<Indexed<B>>>,
    /// Entries per chunk, which one thread evaluates.
    chunk: usize,
    reader: Reader,
    /// The most threads a batch of a chunk each is evaluated on.
    threads: usize,
    jobs: Receiver<Job>,
    updates: Sender<Update>,
}

impl<B: AsRef<[u8]> + Sync> Worker<B> {
    /// Evaluates a batch, a chunk per thread, at a time until the job is done, then
    /// waits for the next.
    /// Ends when the engine is dropped, or when the viewer panicked holding the index.
    fn run(self) {
        let Ok(job) = self.jobs.recv() else {
            return;
        };
        let Some(mut job) = self.newest(job) else {
            return;
        };
        loop {
            let Ok(indexed) = self.indexed.read() else {
                return;
            };
            let total = indexed.records.len();
            if job.from >= total {
                drop(indexed);
                let Some(next) = self.jobs.recv().ok().and_then(|next| self.newest(next)) else {
                    return;
                };
                job = next;
                continue;
            }
            let batch = job.from..total.min(job.from + self.chunk * self.threads);
            // Threads cost more than they save when no entry bytes are read, or
            // when the batch is a few entries.
            let threads = if job.reads_entries() {
                self.threads.min(batch.len().div_ceil(self.chunk))
            } else {
                1
            };
            let found = evaluate(
                &indexed,
                self.reader,
                &job.filters,
                job.search.as_deref(),
                batch.clone(),
                threads,
            );
            let update = Update {
                generation: job.generation,
                found,
            };
            drop(indexed);
            if self.updates.send(update).is_err() {
                return;
            }
            job.from = batch.end;
            let Some(newest) = self.newest(job) else {
                return;
            };
            job = newest;
        }
    }

    /// The last of `job` and the jobs waiting behind it, as each one cancels the
    /// one before. `None` once the engine is dropped.
    fn newest(&self, mut job: Job) -> Option<Job> {
        loop {
            match self.jobs.try_recv() {
                Ok(newer) => job = newer,
                Err(TryRecvError::Empty) => return Some(job),
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

/// What the thread reuses from one entry to the next: room to strip escape codes
/// and to decode JSON strings into, and to find the columns of a first line in.
struct Scratch {
    buffer: Vec<u8>,
    decoded: Vec<u8>,
    reading: Reading,
}

/// How the first line of an entry yields its message.
enum Reading {
    Text(EntryParser, CaptureLocations),
    Structured(Syntax, Mapping),
    /// The whole line is the message.
    Raw,
}

impl Scratch {
    fn new(reader: Reader) -> Scratch {
        let reading = match (reader.format.kind.syntax(), reader.format.entry_parser()) {
            (Some(syntax), _) => Reading::Structured(syntax, reader.mapping),
            (None, Some(parser)) => Reading::Text(parser, parser.locations()),
            (None, None) => Reading::Raw,
        };
        Scratch {
            buffer: Vec::new(),
            decoded: Vec::new(),
            reading,
        }
    }
}

/// The filters at work on entries.
struct Check<'a> {
    filters: &'a Filters,
    search: Option<&'a Matcher>,
}

impl Check<'_> {
    fn run(
        &self,
        records: &[Record],
        bytes: &[u8],
        run: Range<usize>,
        scratch: &mut Scratch,
    ) -> Evaluated {
        let mut update = Evaluated {
            entries: Vec::new(),
            counted: Vec::with_capacity(run.len()),
        };
        for (index, record) in run.clone().zip(&records[run]) {
            let range = record.bytes();
            let entry = bytes
                .get(range.start as usize..range.end as usize)
                .unwrap_or_default();
            let passes = self.passes_all_but_level(record, entry, scratch);
            update.counted.push(record.level().filter(|_| passes));
            if passes && self.filters.levels.shows(record.level()) && self.finds(entry, scratch) {
                // 2^32 entries would take an index of 128 GiB at 32 bytes each.
                update.entries.push(index as u32);
            }
        }
        update
    }

    /// Whether some line of `entry` matches the search, always true without one.
    fn finds(&self, entry: &[u8], scratch: &mut Scratch) -> bool {
        let Some(search) = self.search else {
            return true;
        };
        let syntax = match scratch.reading {
            Reading::Structured(syntax, _) => Some(syntax),
            _ => None,
        };
        source::lines(entry).enumerate().any(|(i, line)| {
            let line = strip_ansi_into(line, &mut scratch.buffer);
            // Only the first line of a JSON or logfmt entry has strings to decode.
            let line = match syntax {
                Some(syntax) if i == 0 => decoded_line(syntax, line, &mut scratch.decoded),
                _ => line,
            };
            search.is_match(line)
        })
    }

    fn passes_all_but_level(&self, record: &Record, entry: &[u8], scratch: &mut Scratch) -> bool {
        let filters = self.filters;
        if !filters
            .time
            .is_none_or(|range| range.contains(record.timestamp()))
        {
            return false;
        }
        if filters.classes.is_empty() && filters.text.is_none() {
            return true;
        }
        let mut lines = source::lines(entry);
        let first = strip_ansi_into(lines.next().unwrap_or_default(), &mut scratch.buffer);
        if !filters
            .classes
            .passes(record.class().and_then(|class| first.get(class)))
        {
            return false;
        }
        let Some(text) = &filters.text else {
            return true;
        };
        let in_first = match &mut scratch.reading {
            Reading::Text(parser, locations) => {
                let head = parser.parse_with(first, locations);
                text.is_match(head.map_or(first, |head| &first[head.message]))
            }
            Reading::Structured(syntax, mapping) => {
                structured_matches(text, *syntax, mapping, first, &mut scratch.decoded)
            }
            Reading::Raw => text.is_match(first),
        };
        in_first || lines.any(|line| text.is_match(&format::strip_ansi(line)))
    }
}

/// Whether `text` matches the decoded message or exception of the JSON or logfmt line
/// `first`. A line without a message field has the whole line as its message.
fn structured_matches(
    text: &Matcher,
    syntax: Syntax,
    mapping: &Mapping,
    first: &[u8],
    decoded: &mut Vec<u8>,
) -> bool {
    let Some(columns) = mapping.text(syntax, first) else {
        return text.is_match(first);
    };
    let mut matches = |field: &Field| text.is_match(field.text(first, decoded));
    let in_message = match &columns.message {
        Some(message) => matches(message),
        None => text.is_match(first),
    };
    in_message || columns.exception.as_ref().is_some_and(matches)
}
