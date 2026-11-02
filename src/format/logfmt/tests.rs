use proptest::prelude::*;

use super::*;

/// Each field of `line` as its key, its raw value and whether that is quoted.
fn scan(line: &str) -> Vec<(&str, &str, bool)> {
    fields(line.as_bytes())
        .map(|field| {
            let key = std::str::from_utf8(field.key).expect("a key of the line");
            (key, &line[field.value], field.string)
        })
        .collect()
}

/// A line cut off inside a quoted value, as the last line of a file still being
/// written may be, keeps what it has of the value.
#[test]
fn line_ending_inside_a_quoted_value_ends_the_value_there() {
    assert_eq!(
        scan(r#"level=INFO msg="shutting dow"#),
        [("level", "INFO", false), ("msg", "shutting dow", true)]
    );
    assert_eq!(scan(r#"msg="cut after \"#), [("msg", r"cut after \", true)]);
}

/// `key=` has an empty value, and the pair after it is read as usual.
#[test]
fn key_with_nothing_after_its_equals_sign_has_an_empty_value() {
    assert_eq!(
        scan("msg= level=INFO"),
        [("msg", "", false), ("level", "INFO", false)]
    );
}

/// Inside quotes, `=` and spaces are part of the value, not the start of a pair.
#[test]
fn equals_sign_inside_a_quoted_value_is_part_of_the_value() {
    assert_eq!(
        scan(r#"query="a=b c=d" level=INFO"#),
        [("query", "a=b c=d", true), ("level", "INFO", false)]
    );
}

/// slog quotes a key that has a space, an `=` or a `"` in it.
#[test]
fn quoted_key_is_read_between_its_quotes() {
    assert_eq!(
        scan(r#""user name"=alice level=INFO"#),
        [("user name", "alice", false), ("level", "INFO", false)]
    );
}

/// One pair as it is generated: a bare key, then a value or none.
#[derive(Debug, Clone)]
enum Pair {
    /// `key=value`, the value as written.
    Bare(String, String),
    /// `key="value"`, the value escaped as Go's `strconv.Quote` escapes it.
    Quoted(String, String),
    /// `key` without `=`.
    KeyOnly(String),
}

impl Pair {
    fn written(&self) -> String {
        match self {
            Pair::Bare(key, value) => format!("{key}={value}"),
            Pair::Quoted(key, value) => format!("{key}=\"{}\"", escaped(value)),
            Pair::KeyOnly(key) => key.clone(),
        }
    }

    /// The key, the value in the line, the value decoded and whether it is quoted, as
    /// the field read back should have them.
    fn expected(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>, bool) {
        match self {
            Pair::Bare(key, value) => (
                key.clone().into(),
                value.clone().into(),
                value.clone().into(),
                false,
            ),
            Pair::Quoted(key, value) => (
                key.clone().into(),
                escaped(value).into(),
                value.clone().into(),
                true,
            ),
            Pair::KeyOnly(key) => (key.clone().into(), vec![], vec![], false),
        }
    }
}

/// `value` with `"`, `\`, line feeds and tabs escaped.
fn escaped(value: &str) -> String {
    value
        .replace('\\', r"\\")
        .replace('"', r#"\""#)
        .replace('\n', r"\n")
        .replace('\t', r"\t")
}

fn chars(from: &[char], len: std::ops::Range<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(from.to_vec()), len).prop_map(String::from_iter)
}

/// A key as written bare: no space, tab, `=` or `"`.
fn key() -> impl Strategy<Value = String> {
    chars(&['a', 'b', '_', '.', '-', ':', '\\', 'é', '😀'], 1..6)
}

/// A bare value: no space or tab, and no `"` to start with; `=` and `\` are text.
fn bare_value() -> impl Strategy<Value = String> {
    chars(&['a', '1', '=', ':', '/', '\\', '"', 'é'], 0..6)
        .prop_filter("a bare value doesn't start with a quote", |value| {
            !value.starts_with('"')
        })
}

/// Text for a quoted value, full of what needs escaping or would end a bare value.
fn quoted_value() -> impl Strategy<Value = String> {
    chars(&['a', ' ', '=', '"', '\\', '\n', '\t', 'é', '😀'], 0..8)
}

fn pair() -> impl Strategy<Value = Pair> {
    prop_oneof![
        (key(), bare_value()).prop_map(|(key, value)| Pair::Bare(key, value)),
        (key(), quoted_value()).prop_map(|(key, value)| Pair::Quoted(key, value)),
        key().prop_map(Pair::KeyOnly),
    ]
}

/// Pairs written into a line with spaces and tabs between them, and maybe before the
/// first and after the last.
fn line() -> impl Strategy<Value = (String, Vec<Pair>)> {
    let separated = prop::collection::vec((pair(), "[ \t]{1,3}"), 0..6);
    ("[ \t]{0,2}", separated, "[ \t]{0,2}").prop_map(|(lead, separated, trail)| {
        let mut line = lead;
        for (pair, separator) in &separated {
            line += &pair.written();
            line += separator;
        }
        if let Some((_, last)) = separated.last() {
            line.truncate(line.len() - last.len());
        }
        line += &trail;
        (line, separated.into_iter().map(|(pair, _)| pair).collect())
    })
}

/// Bytes that are mostly what a tokenizer can trip over.
fn damaged_line() -> impl Strategy<Value = Vec<u8>> {
    let bytes = vec![b'a', b'=', b'"', b'\\', b' ', b'\t', 0xC3, 0xFF];
    prop::collection::vec(prop::sample::select(bytes), 0..64)
}

proptest! {
    #[test]
    fn generated_pairs_come_back_as_they_went_in((line, pairs) in line()) {
        let bytes = line.as_bytes();
        let read: Vec<_> = fields(bytes)
            .map(|field| {
                let raw = bytes[field.value.clone()].to_vec();
                let decoded = field.decoded(bytes).into_owned();
                (field.key.to_vec(), raw, decoded, field.string)
            })
            .collect();
        let written: Vec<_> = pairs.iter().map(Pair::expected).collect();
        prop_assert_eq!(read, written, "{:?}", line);
    }

    /// Every field takes up at least one byte of the line, so there are never more
    /// fields than bytes.
    #[test]
    fn reading_never_panics_and_stays_inside_the_line(
        line in prop_oneof![prop::collection::vec(any::<u8>(), 0..256), damaged_line()]
    ) {
        let read: Vec<_> = fields(&line).take(line.len() + 1).collect();
        prop_assert!(read.len() <= line.len());
        for field in read {
            prop_assert!(field.value.start <= field.value.end);
            prop_assert!(field.value.end <= line.len());
        }
    }
}
