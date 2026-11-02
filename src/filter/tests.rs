use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use proptest::prelude::*;

use super::*;
use crate::format::{Anchor, Format, Level, Mapping};
use crate::index::{self, Chunk, Settings};
use crate::matcher::{self, Matcher};
use crate::test_support::built_in;

/// How long evaluating a test log of a few entries may take before the test fails.
/// It takes milliseconds; the deadline is only there so that an engine that never
/// finishes fails the test quickly.
const DEADLINE: Duration = Duration::from_secs(2);

const LEVELS: [Level; 6] = [
    Level::Trace,
    Level::Debug,
    Level::Info,
    Level::Warn,
    Level::Error,
    Level::Fatal,
];

fn settings(preset: &'static Format) -> Settings {
    let zone = TimeZone::UTC;
    let mtime: Timestamp = "2026-10-08T12:00:00Z".parse().unwrap();
    Settings {
        format: preset,
        anchor: Anchor::Modified(zone.to_datetime(mtime)),
        zone,
        mapping: Mapping::default(),
    }
}

/// `log` and its index, shared as the viewer shares them with the engine.
fn indexed(preset: &'static Format, log: &[u8]) -> Arc<RwLock<Indexed<Vec<u8>>>> {
    let settings = settings(preset);
    let records = Chunk::index(log, 0, 0..log.len(), &settings).finish(&settings);
    Arc::new(RwLock::new(Indexed {
        records,
        bytes: Arc::new(log.to_vec()),
    }))
}

/// Polls `engine` until it has evaluated all `total` entries, handing every result
/// it publishes on the way to `check`. Fails the test after [`DEADLINE`].
fn finish(engine: &mut Engine, total: usize, mut check: impl FnMut(&Filtered)) -> Filtered {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let filtered = engine.poll();
        check(filtered);
        if filtered.evaluated == total {
            return filtered.clone();
        }
        assert!(
            Instant::now() < deadline,
            "filtering took longer than {DEADLINE:?}: {filtered:?}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// How the engine reads `format`, with no JSON keys.
fn reader(format: &'static Format) -> Reader {
    Reader::new(format, Mapping::default())
}

/// What `filters` find in `log`, once all of it is evaluated.
fn filter(preset: &'static Format, log: &str, filters: Filters) -> Filtered {
    let indexed = indexed(preset, log.as_bytes());
    let total = indexed.read().unwrap().records.len();
    let mut engine = Engine::spawn(indexed, reader(preset), filters, Options { chunk: 2 });
    finish(&mut engine, total, |_| {})
}

fn text(pattern: &str) -> Option<Matcher> {
    Some(Matcher::new(pattern, matcher::Options::default()).unwrap())
}

fn counts(pairs: &[(Level, u32)]) -> [u32; 6] {
    let mut counts = [0; 6];
    for &(level, count) in pairs {
        counts[level as usize] = count;
    }
    counts
}

#[test]
fn an_empty_index_matches_nothing() {
    assert_eq!(
        filter(built_in("logback"), "", Filters::default()),
        Filtered::default()
    );
}

#[test]
fn counters_ignore_the_level_filter() {
    let log = "\
2026-10-07 12:00:00.000 [main] INFO  com.myco.Order - created
2026-10-07 12:00:01.000 [main] WARN  com.myco.Order - slow
2026-10-07 12:00:02.000 [main] ERROR com.myco.Order - failed
";
    let mut filters = Filters::default();
    for level in LEVELS {
        filters.levels.set(level, false);
    }
    assert_eq!(
        filter(built_in("logback"), log, filters),
        Filtered {
            evaluated: 3,
            entries: vec![],
            counts: counts(&[(Level::Info, 1), (Level::Warn, 1), (Level::Error, 1)]),
        }
    );
}

/// The levels `levels` shows, from least to most severe.
fn shown(levels: Levels) -> Vec<Level> {
    Level::ALL
        .into_iter()
        .filter(|&level| levels.is_on(level))
        .collect()
}

#[test]
fn toggling_a_level_hides_only_that_level() {
    let mut levels = Levels::default();
    levels.toggle(Level::Warn);
    assert_eq!(
        shown(levels),
        [
            Level::Trace,
            Level::Debug,
            Level::Info,
            Level::Error,
            Level::Fatal
        ]
    );
}

#[test]
fn toggling_a_level_twice_shows_it_again() {
    let mut levels = Levels::default();
    levels.toggle(Level::Warn);
    levels.toggle(Level::Warn);
    assert_eq!(levels, Levels::default());
}

#[test]
fn only_shows_just_the_levels_it_names() {
    let levels = Levels::only(&[Level::Warn, Level::Error]);
    assert_eq!(shown(levels), [Level::Warn, Level::Error]);
}

/// A headless line, an INFO entry and an entry whose level vilog doesn't know.
const WITHOUT_LEVELS: &str = "\
started
2026-10-07 12:00:00.000 [main] INFO  com.myco.Order - created
2026-10-07 12:00:01.000 [main] AUDIT com.myco.Order - failed
";

#[test]
fn with_every_level_off_no_entry_matches_not_even_one_without_a_level() {
    let filters = Filters {
        levels: Levels::only(&[]),
        ..Filters::default()
    };
    let found = filter(built_in("logback"), WITHOUT_LEVELS, filters);
    assert_eq!(found.entries, [] as [u32; 0]);
}

#[test]
fn entries_without_a_level_are_hidden_while_any_level_is_off() {
    let mut filters = Filters::default();
    filters.levels.toggle(Level::Trace);
    let found = filter(built_in("logback"), WITHOUT_LEVELS, filters);
    assert_eq!(found.entries, [1]);
}

#[test]
fn raw_entries_are_matched_over_the_whole_line() {
    let filters = Filters {
        text: text("beta"),
        ..Filters::default()
    };
    let found = filter(built_in("raw"), "alpha beta\ngamma\n", filters);
    assert_eq!(found.entries, [0]);
}

#[test]
fn escape_codes_are_stripped_before_matching() {
    let log = "\
2026-10-07 12:00:00.000 [main] \x1b[31mERROR\x1b[0m \x1b[36mcom.myco.Order\x1b[0m - pay\x1b[1mment\x1b[0m failed
\t\x1b[2mat com.myco.Boom\x1b[0m
2026-10-07 12:00:01.000 [main] INFO  org.hibernate.SQL - select
";
    let class = Filters {
        classes: Classes {
            include: vec![Rule::Substring("com.myco.Order".to_string())],
            exclude: vec![],
        },
        ..Filters::default()
    };
    let message = Filters {
        text: text("payment"),
        ..Filters::default()
    };
    let continuation = Filters {
        text: text("Boom"),
        ..Filters::default()
    };
    for filters in [class, message, continuation] {
        let found = filter(built_in("logback"), log, filters.clone());
        assert_eq!(found.entries, [0], "{filters:?}");
    }
}

#[test]
fn continuation_lines_appended_to_the_last_entry_are_matched() {
    let log = "2026-10-07 12:00:00.000 [main] ERROR com.myco.Order - failed\n";
    let more = "\tat com.myco.Boom\n";
    let filters = Filters {
        text: text("Boom"),
        ..Filters::default()
    };
    let indexed = indexed(built_in("logback"), log.as_bytes());
    let mut engine = Engine::spawn(
        Arc::clone(&indexed),
        reader(built_in("logback")),
        filters,
        Options { chunk: 1 },
    );
    assert_eq!(finish(&mut engine, 1, |_| {}).entries, [] as [u32; 0]);
    append(
        &indexed,
        built_in("logback"),
        format!("{log}{more}").as_bytes(),
    );
    engine.reread(0);
    assert_eq!(finish(&mut engine, 1, |_| {}).entries, [0]);
}

#[test]
fn results_of_entries_read_again_are_never_published_stale() {
    let line = |second: u8, message: &str| {
        format!("2026-10-07 12:00:0{second}.000 [main] INFO  com.myco.Order - {message}\n")
    };
    let before = [line(0, "one"), line(1, "boom"), line(2, "boom")].concat();
    let after = [line(0, "one"), line(1, "two"), line(2, "boom")].concat();
    let shared = indexed(built_in("logback"), before.as_bytes());
    let mut engine = Engine::spawn(
        Arc::clone(&shared),
        reader(built_in("logback")),
        Filters {
            text: text("boom"),
            ..Filters::default()
        },
        Options { chunk: 1 },
    );
    let found = finish(&mut engine, 3, |_| {});
    assert_eq!(found.entries, [1, 2]);
    assert_eq!(found.counts, counts(&[(Level::Info, 2)]));
    let rewritten = Arc::into_inner(indexed(built_in("logback"), after.as_bytes())).unwrap();
    *shared.write().unwrap() = rewritten.into_inner().unwrap();
    engine.reread(1);
    // From here on, the second and third entries count only as they read now.
    let found = finish(&mut engine, 3, |partial| {
        let third_in = partial.evaluated == 3;
        let expected = Filtered {
            evaluated: partial.evaluated,
            entries: if third_in { vec![2] } else { vec![] },
            counts: counts(&[(Level::Info, u32::from(third_in))]),
        };
        assert_eq!(*partial, expected);
    });
    assert_eq!(found.entries, [2]);
}

#[test]
fn new_filters_from_an_entry_on_keep_the_results_before_it() {
    let line = |second: u8, message: &str| {
        format!("2026-10-07 12:00:0{second}.000 [main] INFO  com.myco.Order - {message}\n")
    };
    let log = [
        line(0, "boom"),
        line(1, "boom"),
        line(2, "two"),
        line(3, "boom"),
    ]
    .concat();
    let mut engine = Engine::spawn(
        indexed(built_in("logback"), log.as_bytes()),
        reader(built_in("logback")),
        Filters {
            text: text("boom"),
            ..Filters::default()
        },
        Options { chunk: 1 },
    );
    assert_eq!(finish(&mut engine, 4, |_| {}).entries, [0, 1, 3]);
    engine.set_filters_from(
        Filters {
            text: text("two"),
            ..Filters::default()
        },
        1,
    );
    // The first entry stays a match; from the second on only "two" matches.
    let found = finish(&mut engine, 4, |partial| {
        let expected: &[u32] = if partial.evaluated >= 3 {
            &[0, 2]
        } else {
            &[0]
        };
        assert_eq!(partial.entries, expected, "{partial:?}");
        assert_eq!(
            partial.counts,
            counts(&[(Level::Info, expected.len() as u32)]),
            "{partial:?}"
        );
    });
    assert_eq!(found.evaluated, 4);
}

/// Brings `indexed` up to date with `log`, the whole file after more was appended,
/// as the viewer does.
fn append(indexed: &RwLock<Indexed<Vec<u8>>>, preset: &'static Format, log: &[u8]) -> usize {
    let mut indexed = indexed.write().unwrap();
    let known = indexed.records.len();
    index::append(&mut indexed.records, log, &settings(preset));
    indexed.bytes = Arc::new(log.to_vec());
    known
}

// A model of the log and the filters, and a naive filter over it.

const WORDS: [&str; 4] = ["alpha", "beta", "myco", "Order"];
const CLASSES: [&str; 3] = ["com.myco.Order", "com.myco.pay.Client", "org.hibernate.SQL"];
const RULES: [&str; 5] = ["myco", "Order", "hibernate", "pay", "Client"];

/// An entry: a first line with columns, or none for the lines before the first one,
/// and continuation lines, each holding one word.
#[derive(Debug, Clone)]
struct Entry {
    head: Option<Head>,
    lines: Vec<&'static str>,
}

#[derive(Debug, Clone)]
struct Head {
    second: u8,
    /// `None` is written as `AUDIT`, a level word vilog doesn't know.
    level: Option<Level>,
    class: &'static str,
    message: &'static str,
}

#[derive(Debug, Clone)]
struct Model {
    levels: [bool; 6],
    include: Vec<&'static str>,
    exclude: Vec<&'static str>,
    /// Seconds after 12:00:00, the end exclusive.
    time: Option<(u8, Option<u8>)>,
    text: Option<&'static str>,
}

fn at_second(second: u8) -> Timestamp {
    format!("2026-10-07T12:00:{second:02}Z").parse().unwrap()
}

impl Model {
    fn filters(&self) -> Filters {
        let mut levels = Levels::default();
        for (level, &on) in LEVELS.iter().zip(&self.levels) {
            levels.set(*level, on);
        }
        Filters {
            levels,
            classes: Classes {
                include: self.include.iter().map(|rule| substring(rule)).collect(),
                exclude: self.exclude.iter().map(|rule| substring(rule)).collect(),
            },
            time: self.time.map(|(start, end)| TimeRange {
                start: at_second(start),
                end: end.map(at_second),
            }),
            text: self.text.and_then(text),
        }
    }

    fn passes_level(&self, entry: &Entry) -> bool {
        match entry.head.as_ref().and_then(|head| head.level) {
            Some(level) => self.levels[level as usize],
            None => self.levels.iter().all(|&on| on),
        }
    }

    fn passes_others(&self, entry: &Entry) -> bool {
        let head = entry.head.as_ref();
        let class = head.map(|head| head.class);
        let class_passes = !self
            .exclude
            .iter()
            .any(|rule| class.is_some_and(|class| class.contains(rule)))
            && (self.include.is_empty()
                || self
                    .include
                    .iter()
                    .any(|rule| class.is_some_and(|class| class.contains(rule))));
        let time_passes = self.time.is_none_or(|(start, end)| {
            head.is_some_and(|head| head.second >= start && end.is_none_or(|end| head.second < end))
        });
        let text_passes = self.text.is_none_or(|word| {
            head.is_some_and(|head| head.message.contains(word))
                || entry.lines.iter().any(|line| line.contains(word))
        });
        class_passes && time_passes && text_passes
    }

    /// What a naive filter finds in the entries `run`, with a search for `search`
    /// if given.
    fn naive_run(
        &self,
        entries: &[Entry],
        run: std::ops::Range<usize>,
        search: Option<&str>,
    ) -> Evaluated {
        let mut found = Evaluated::default();
        for (i, entry) in run.clone().zip(&entries[run]) {
            let passes = self.passes_others(entry);
            let level = entry.head.as_ref().and_then(|head| head.level);
            found.counted.push(level.filter(|_| passes));
            let searched =
                search.is_none_or(|word| render(std::slice::from_ref(entry)).contains(word));
            if passes && self.passes_level(entry) && searched {
                found.entries.push(i as u32);
            }
        }
        found
    }

    fn naive(&self, entries: &[Entry]) -> Filtered {
        let run = self.naive_run(entries, 0..entries.len(), None);
        let mut counts = [0; 6];
        for level in run.counted.into_iter().flatten() {
            counts[level as usize] += 1;
        }
        Filtered {
            evaluated: entries.len(),
            entries: run.entries,
            counts,
        }
    }
}

fn render(entries: &[Entry]) -> String {
    let mut log = String::new();
    for entry in entries {
        if let Some(head) = &entry.head {
            let level = head.level.map_or("AUDIT", |level| {
                ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"][level as usize]
            });
            log.push_str(&format!(
                "2026-10-07 12:00:{:02}.000 [main] {level:<5} {} - {}\n",
                head.second, head.class, head.message
            ));
        }
        for line in &entry.lines {
            log.push_str(&format!("\tat {line}\n"));
        }
    }
    log
}

fn word() -> impl Strategy<Value = &'static str> {
    prop::sample::select(&WORDS[..])
}

fn head() -> impl Strategy<Value = Head> {
    (
        0..6u8,
        prop::option::weighted(0.8, prop::sample::select(&LEVELS[..])),
        prop::sample::select(&CLASSES[..]),
        word(),
    )
        .prop_map(|(second, level, class, message)| Head {
            second,
            level,
            class,
            message,
        })
}

/// Entries as a file has them: only the first can be without a head, and it then
/// has at least one line.
fn entries() -> impl Strategy<Value = Vec<Entry>> {
    let lines = || prop::collection::vec(word(), 0..3);
    (
        prop::option::of(prop::collection::vec(word(), 1..3)),
        prop::collection::vec((head(), lines()), 0..12),
    )
        .prop_map(|(lead, entries)| {
            let lead = lead.map(|lines| Entry { head: None, lines });
            lead.into_iter()
                .chain(entries.into_iter().map(|(head, lines)| Entry {
                    head: Some(head),
                    lines,
                }))
                .collect()
        })
}

fn model() -> impl Strategy<Value = Model> {
    let rules = || prop::collection::vec(prop::sample::select(&RULES[..]), 0..3);
    (
        prop::array::uniform6(prop::bool::weighted(0.8)),
        rules(),
        rules(),
        prop::option::of((0..7u8, prop::option::of(0..7u8))),
        prop::option::of(word()),
    )
        .prop_map(|(levels, include, exclude, time, text)| Model {
            levels,
            include,
            exclude,
            time,
            text,
        })
}

proptest! {
    // A case that never finishes costs a whole deadline, so shrinking one is cut short.
    #![proptest_config(ProptestConfig {
        max_shrink_time: 5_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn results_match_a_naive_filter_after_a_change_of_filters(
        entries in entries(),
        first in model(),
        second in model(),
        chunk in 1..5usize,
        polls in 0..4usize,
    ) {
        let indexed = indexed(built_in("logback"), render(&entries).as_bytes());
        let mut engine = Engine::spawn(
            indexed,
            reader(built_in("logback")),
            first.filters(),
            Options { chunk },
        );
        for _ in 0..polls {
            let partial = engine.poll();
            assert_eq!(*partial, first.naive(&entries[..partial.evaluated]));
        }
        engine.set_filters(second.filters());
        let found = finish(&mut engine, entries.len(), |partial| {
            assert_eq!(*partial, second.naive(&entries[..partial.evaluated]));
        });
        prop_assert_eq!(found, second.naive(&entries));
    }

    #[test]
    fn results_match_a_naive_filter_after_an_append(
        entries in entries(),
        model in model(),
        chunk in 1..5usize,
        polls in 0..4usize,
        cut in any::<prop::sample::Index>(),
    ) {
        let log = render(&entries);
        let cut = cut.index(log.len() + 1);
        let indexed = indexed(built_in("logback"), &log.as_bytes()[..cut]);
        let mut engine = Engine::spawn(
            Arc::clone(&indexed),
            reader(built_in("logback")),
            model.filters(),
            Options { chunk },
        );
        for _ in 0..polls {
            engine.poll();
        }
        let known = append(&indexed, built_in("logback"), log.as_bytes());
        engine.reread(known.saturating_sub(1));
        let found = finish(&mut engine, entries.len(), |_| {});
        prop_assert_eq!(found, model.naive(&entries));
    }
}

/// What [`evaluate`] finds in the entries `run` of the log `entries` make, on
/// `threads` threads.
fn evaluate_run(
    entries: &[Entry],
    model: &Model,
    search: Option<&str>,
    run: std::ops::Range<usize>,
    threads: usize,
) -> Evaluated {
    let indexed = indexed(built_in("logback"), render(entries).as_bytes());
    let search = search.map(|word| Matcher::new(word, matcher::Options::default()).unwrap());
    evaluate(
        &indexed.read().unwrap(),
        reader(built_in("logback")),
        &model.filters(),
        search.as_ref(),
        run,
        threads,
    )
}

proptest! {
    #[test]
    fn evaluating_a_run_on_any_number_of_threads_matches_a_naive_filter(
        entries in entries(),
        model in model(),
        search in prop::option::of(word()),
        ends in (any::<prop::sample::Index>(), any::<prop::sample::Index>()),
        threads in 0..6usize,
    ) {
        let (a, b) = (ends.0.index(entries.len() + 1), ends.1.index(entries.len() + 1));
        let run = a.min(b)..a.max(b);
        prop_assert_eq!(
            evaluate_run(&entries, &model, search, run.clone(), threads),
            model.naive_run(&entries, run, search)
        );
    }
}

/// Long enough that each of the threads has thousands of entries to evaluate.
#[test]
fn a_long_run_split_across_threads_matches_a_naive_filter() {
    let entries: Vec<Entry> = (0..20_000)
        .map(|i| Entry {
            head: Some(Head {
                second: (i % 6) as u8,
                level: LEVELS.get(i % 7).copied(),
                class: CLASSES[i % 3],
                message: WORDS[i / 3 % 4],
            }),
            lines: WORDS[..i % 5 % 3].to_vec(),
        })
        .collect();
    let model = Model {
        levels: [true, false, true, true, true, true],
        include: vec!["myco"],
        exclude: vec!["Client"],
        time: Some((1, None)),
        text: Some("beta"),
    };
    let run = 3..entries.len() - 2;
    let expected = model.naive_run(&entries, run.clone(), Some("alpha"));
    // The pattern of the entries repeats every 420, so matches are all over the run.
    assert!(expected.entries.len() > 900, "{}", expected.entries.len());
    assert_eq!(
        evaluate_run(&entries, &model, Some("alpha"), run, 4),
        expected
    );
}

#[test]
fn only_a_text_filter_a_class_filter_or_a_search_read_the_entry_bytes() {
    let job = |filters: Filters, search: Option<&str>| Job {
        generation: 0,
        filters: Arc::new(filters),
        search: search.map(|word| Arc::new(text(word).unwrap())),
        from: 0,
    };
    let index_only = Filters {
        levels: Levels::only(&[Level::Error]),
        time: Some(TimeRange {
            start: at_second(0),
            end: None,
        }),
        ..Filters::default()
    };
    let message = Filters {
        text: text("boom"),
        ..Filters::default()
    };
    let class = Filters {
        classes: classes(&["com.myco.*"]),
        ..Filters::default()
    };
    let reads = [
        job(index_only.clone(), None),
        job(index_only, Some("boom")),
        job(message, None),
        job(class, None),
    ]
    .map(|job| job.reads_entries());
    assert_eq!(reads, [false, true, true, true]);
}

// --- class rules ---

fn prefix(rule: &str) -> Rule {
    Rule::Prefix(rule.to_string())
}

fn substring(rule: &str) -> Rule {
    Rule::Substring(rule.to_string())
}

/// The class filter `rules` make; fails the test if they don't parse.
fn classes(rules: &[&str]) -> Classes {
    Classes::parse(rules.iter().copied()).unwrap()
}

/// Which of `names` pass the class filter `rules` make.
fn passing<'a>(rules: &[&str], names: &[&'a str]) -> Vec<&'a str> {
    let classes = classes(rules);
    names
        .iter()
        .copied()
        .filter(|name| classes.passes(Some(name.as_bytes())))
        .collect()
}

#[test]
fn a_rule_ending_in_dot_star_is_a_prefix_include() {
    assert_eq!(
        classes(&["com.myco.*"]),
        Classes {
            include: vec![prefix("com.myco")],
            exclude: vec![],
        }
    );
}

#[test]
fn any_other_rule_is_a_substring_include() {
    assert_eq!(
        classes(&["OrderService"]),
        Classes {
            include: vec![substring("OrderService")],
            exclude: vec![],
        }
    );
}

#[test]
fn a_bang_makes_either_form_an_exclude() {
    assert_eq!(
        classes(&["!org.hibernate", "!com.myco.*"]),
        Classes {
            include: vec![],
            exclude: vec![substring("org.hibernate"), prefix("com.myco")],
        }
    );
}

#[test]
fn a_prefix_rule_matches_the_package_and_everything_below_it_only() {
    let names = [
        "com.myco",
        "com.myco.Order",
        "com.myco.pay.Client",
        "com.mycompany",
        "com.mycompany.Order",
        "org.com.myco.Order",
    ];
    assert_eq!(
        passing(&["com.myco.*"], &names),
        ["com.myco", "com.myco.Order", "com.myco.pay.Client"]
    );
}

#[test]
fn a_substring_rule_matches_anywhere_in_the_full_class_name() {
    let names = [
        "com.myco.OrderService",
        "OrderServiceImpl",
        "com.myco.Order",
        "c.m.OrderService",
    ];
    assert_eq!(
        passing(&["myco.OrderService"], &names),
        ["com.myco.OrderService"]
    );
}

#[test]
fn includes_are_ored() {
    let names = ["com.myco.Order", "org.hibernate.SQL", "org.apache.Http"];
    assert_eq!(
        passing(&["com.myco.*", "hibernate"], &names),
        ["com.myco.Order", "org.hibernate.SQL"]
    );
}

#[test]
fn excludes_win_over_includes() {
    let names = ["com.myco.Order", "com.myco.pay.Client"];
    assert_eq!(
        passing(&["com.myco.*", "!com.myco.pay.*"], &names),
        ["com.myco.Order"]
    );
}

#[test]
fn without_includes_everything_but_the_excludes_passes() {
    let names = ["com.myco.Order", "org.hibernate.SQL", "org.apache.Http"];
    assert_eq!(
        passing(&["!org.hibernate"], &names),
        ["com.myco.Order", "org.apache.Http"]
    );
}

#[test]
fn rules_are_case_sensitive() {
    let names = ["com.myco.OrderService", "com.MyCo.Order"];
    assert_eq!(
        passing(&["com.MyCo.*", "orderservice"], &names),
        ["com.MyCo.Order"]
    );
}

#[test]
fn an_entry_without_a_class_fails_any_include() {
    assert!(!classes(&["com.myco.*", "!org.hibernate"]).passes(None));
}

#[test]
fn an_entry_without_a_class_passes_excludes_alone() {
    assert!(classes(&["!org.hibernate"]).passes(None));
}

#[test]
fn whitespace_around_a_rule_is_ignored() {
    assert_eq!(
        classes(&[" com.myco.* ", "\t!org.hibernate", "! org.apache "]),
        classes(&["com.myco.*", "!org.hibernate", "!org.apache"])
    );
}

#[test]
fn empty_and_blank_items_are_skipped() {
    assert_eq!(classes(&["", "  ", "Order", ""]), classes(&["Order"]));
}

#[test]
fn a_rule_given_twice_counts_once() {
    assert_eq!(
        classes(&["Order", "Order ", "!org.*", "!org.*"]),
        Classes {
            include: vec![substring("Order")],
            exclude: vec![prefix("org")],
        }
    );
}

#[test]
fn a_rule_naming_no_logger_is_an_error_naming_it() {
    for rule in ["!", "*", ".*", "!*", "!.*"] {
        let error = Classes::parse(["Order", rule]).unwrap_err();
        assert_eq!(error, format!("logger rule '{rule}' names no logger"));
    }
}

#[test]
fn a_star_anywhere_but_in_a_trailing_dot_star_is_an_error_naming_the_rule() {
    for rule in ["Order*", "com.*.Order", "com.myco.**"] {
        let error = Classes::parse([rule]).unwrap_err();
        assert!(
            error.starts_with(&format!("logger rule '{rule}' ")),
            "{rule}: {error}"
        );
    }
}

#[test]
fn an_exclude_equal_to_an_include_hides_that_class_and_everything_else() {
    let names = ["com.myco.Order", "org.hibernate.SQL"];
    assert_eq!(
        passing(&["com.myco.*", "!com.myco.*"], &names),
        [] as [&str; 0]
    );
}

#[test]
fn class_rules_see_the_full_class_name_of_the_first_line() {
    let log = "\
2026-10-07 12:00:00.000 [main] INFO  com.myco.order.service.OrderService - created
2026-10-07 12:00:01.000 [main] INFO  com.mycompany.Billing - billed
";
    let filters = Filters {
        classes: classes(&["com.myco.*"]),
        ..Filters::default()
    };
    assert_eq!(filter(built_in("logback"), log, filters).entries, [0]);
}

const SEGMENTS: [&str; 7] = [
    "com",
    "myco",
    "mycompany",
    "Order",
    "OrderService",
    "org",
    "hibernate",
];

/// `segments` joined with dots.
fn dotted(segments: std::ops::Range<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(&SEGMENTS[..]), segments)
        .prop_map(|segments| segments.join("."))
}

fn rule() -> impl Strategy<Value = Rule> {
    prop_oneof![
        dotted(1..3).prop_map(Rule::Prefix),
        dotted(1..3).prop_map(Rule::Substring),
    ]
}

fn class_name() -> impl Strategy<Value = Option<String>> {
    prop::option::weighted(0.9, dotted(1..5))
}

proptest! {
    #[test]
    fn adding_an_exclude_never_adds_matches(
        include in prop::collection::vec(rule(), 0..3),
        exclude in prop::collection::vec(rule(), 0..3),
        added in rule(),
        class in class_name(),
    ) {
        let before = Classes { include, exclude };
        let mut after = before.clone();
        after.exclude.push(added);
        let class = class.as_deref().map(str::as_bytes);
        prop_assert!(!after.passes(class) || before.passes(class));
    }

    // Without any include everything passes, so the first include can only take
    // matches away.
    #[test]
    fn adding_an_include_never_removes_matches_while_there_is_no_exclude(
        include in prop::collection::vec(rule(), 1..3),
        added in rule(),
        class in class_name(),
    ) {
        let before = Classes { include, exclude: vec![] };
        let mut after = before.clone();
        after.include.push(added);
        let class = class.as_deref().map(str::as_bytes);
        prop_assert!(!before.passes(class) || after.passes(class));
    }

    #[test]
    fn parsing_and_matching_never_panic(
        items in prop::collection::vec(prop_oneof![any::<String>(), "[!*. a]{0,5}"], 0..4),
        class in prop::option::of(prop::collection::vec(any::<u8>(), 0..12)),
    ) {
        if let Ok(classes) = Classes::parse(items.iter().map(String::as_str)) {
            classes.passes(class.as_deref());
        }
    }
}

/// The entries of `log`, read in `format`, in which a search for `word` finds
/// something.
fn searched(format: &'static Format, log: &str, word: &str) -> Vec<u32> {
    let indexed = indexed(format, log.as_bytes());
    let index = indexed.read().unwrap();
    let search = Matcher::new(word, matcher::Options::default()).unwrap();
    let all = 0..index.records.len();
    evaluate(
        &index,
        reader(format),
        &Filters::default(),
        Some(&search),
        all,
        1,
    )
    .entries
}

/// Only the JSON format searches decoded (PRD §9.2): read in another, a line that is
/// a JSON object is searched as written, so `"hi"` doesn't find `\"hi\"`.
#[test]
fn search_outside_the_json_format_matches_escapes_as_written() {
    let log = r#"{"msg":"say \"hi\""}"#.to_owned() + "\n";
    let raw = built_in("raw");
    assert_eq!(searched(raw, &log, r#""hi""#), Vec::<u32>::new());
    assert_eq!(searched(raw, &log, r#"\"hi\""#), [0]);
}

/// A banner before the first object is no JSON line, so its escapes stay as written.
#[test]
fn search_in_a_json_log_reads_a_line_before_the_first_object_as_written() {
    let log = concat!(r#"say \"hi\""#, "\n", r#"{"msg":"a"}"#, "\n");
    let json = built_in("json");
    assert_eq!(searched(json, log, r#""hi""#), Vec::<u32>::new());
    assert_eq!(searched(json, log, r#"\"hi\""#), [0]);
}

/// An indented line before the first entry doesn't start one, so its quoted values
/// stay as written.
#[test]
fn search_in_a_logfmt_log_reads_a_line_before_the_first_entry_as_written() {
    let log = concat!(r#"  note="say \"hi\"""#, "\n", "msg=a\n");
    let logfmt = built_in("logfmt");
    assert_eq!(searched(logfmt, log, r#""hi""#), Vec::<u32>::new());
    assert_eq!(searched(logfmt, log, r#"\"hi\""#), [0]);
}
