# logfmt fixtures

One file per logging library, each written the way the library writes it by default (PRD 6.3). LF endings, timestamps on 2026-03-14 at +01:00. The table gives the key each column is read from.

| File | timestamp | level | logger | thread | message | exception |
|---|---|---|---|---|---|---|
| `slog.log` | `time` (milliseconds, unquoted) | `level` | — | — | `msg` | — |
| `logrus.log` | `time` (seconds, quoted) | `level` | — | — | `msg` | the Go panic after the last line (entry 8) |

`slog.log` has 9 lines, one entry each. `logrus.log` has 8 entries; the last one is followed by the 11 lines Go prints when logrus panics, which continue it.

Edge cases on purpose:

- `slog.log`: a one-word message without quotes (`msg=started`), `=` and a space inside a quoted value (`query="status=open limit=20"`), `\"` inside quotes (`err=`), plain UTF-8 (`Zamówienie 43 przyjęte`), an empty message (`msg=""`), and a last line cut off inside its quoted message, without a line ending, as a file still being written has it.
- `logrus.log`: an empty value (`coupon=`), `\"` inside the message (`Deprecated header \"X-Legacy\" used`) and inside `error=`, `=` inside a quoted value (`dsn=`), and plain UTF-8.

Level names: slog writes `DEBUG`, `INFO`, `WARN`, `ERROR`; logrus writes lower case and `warning`, `trace`, `panic`.

## Library notes

- **slog** `TextHandler` writes `time`, `level`, `msg`, then the attributes in the order they were given. The time has milliseconds and isn't quoted. A value is quoted only when it is empty or has a space, `=`, `"` or a character that isn't printable, so `addr=:8080` and `elapsed=1.4s` stay bare. Quoting is Go's `strconv.Quote`: `\"`, `\\`, `\n` and so on, non-ASCII text kept as UTF-8. An error is an ordinary attribute (`err`), not an exception.
- **logrus** `TextFormatter`, writing to a file rather than a terminal, writes `time`, `level`, `msg`, then the fields sorted by key. Its default time format has seconds and an offset, which it quotes for the `:`. It quotes any value with a character outside `[A-Za-z0-9-._/@^+]`, but leaves an empty value bare (`coupon=`), and leaves `msg` out when the message is empty. `Panic` writes the entry, then panics with it, and the Go runtime prints `panic:`, the goroutine and its frames, with tab-indented file lines.
