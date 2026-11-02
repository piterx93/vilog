# PRD: `vilog` — interactive TUI log viewer for application logs

> Shipped as **jlv** (Java Log Viewer) up to 1.x; renamed to **vilog** in 2.0 (§15).
> Status: v0.3 · 2026-10-17

---

## 1. Summary

`vilog` is a fast, interactive terminal log viewer — in the spirit of `lnav` / LogExpert, with the feel of `htop` / `btop` — for application logs from Java, Python, Node.js, .NET and Go services. It recognises the usual text layouts of their logging libraries as well as JSON lines and logfmt, parses each entry into structured columns, colors severities, follows files live (`tail -F` semantics), and lets the user scroll, search, and filter by level, logger, time, and text without leaving the terminal.

```
| TIMESTAMP | SEVERITY | LOGGER | MESSAGE |
```

The logger column holds whatever names the source of an entry: the class in Java, the logger name in Python, the category in .NET, the caller in Go.

## 2. Problem

- `tail -f | grep` loses context, can't scroll back comfortably, and doesn't understand multi-line stack traces.
- `less` doesn't understand log structure (levels, loggers, time), and JSON logs are unreadable in it.
- `lnav` is powerful but generic; stack-trace folding, logger-name abbreviation, logger-prefix filters and the formats of common frameworks out of the box are second-class.
- LogExpert is Windows/GUI-only and unusable over SSH.

## 3. Goals

1. Open multi-GB log files instantly and scroll smoothly.
2. Recognize the common log formats of Java, Python, Node.js, .NET and Go automatically, text and JSON alike — zero configuration for the 90% case, a regex in the config for the rest.
3. Live follow that survives log rotation.
4. Fast, composable filtering by **level**, **logger**, **time window**, and **text/regex**.
5. Work identically over SSH and locally, shipped as a single static Linux binary.

### Non-goals

- Multiple files in one view, stdin / pipe input (backlog, §15).
- Rendering message templates (Serilog `@mt`); the rendered message is shown when the log has one.
- Parsing a log line nested inside another one, e.g. the line inside Docker's `json-file` wrapper (§6.2).
- Editing logs, remote log fetching (SSH/K8s APIs), alerting, persistence/database.

## 4. Users & key scenarios

Target: developers, DevOps/SRE and support engineers working with Java, Python, Node.js, .NET and Go services. Open source project.

| # | Scenario |
|---|----------|
| S1 | SSH into a server, `vilog /opt/wildfly/standalone/log/server.log`, watch live traffic during a deploy. |
| S2 | "What happened in the last 10 minutes?" — one keypress narrows to the last 10 min, toggle only WARN+ERROR. |
| S3 | Incident at 14:05 yesterday: open the rotated `.gz`, filter `14:00-14:15`, jump between exceptions. |
| S4 | Too much noise: hide `org.hibernate`, `org.jboss`, show only `com.mycompany.*`. |
| S5 | Found the relevant error: open details, expand the stack trace, copy it to clipboard, export the filtered slice to a file for a ticket. |
| S6 | A Django app behind gunicorn throws: `vilog gunicorn-error.log`, `]` jumps from traceback to traceback. |
| S7 | A Go or Node.js service in a container logs JSON: open the container's log file, filter WARN+, `Enter` shows every field of the entry and the decoded stack trace. |
| S8 | A .NET service writes a Serilog file: show only `MyApp.Payments.*`, find the inner exception. |

## 5. Platform & distribution

- **Language/stack:** Rust + `ratatui` (`crossterm` backend).
- **Platforms:** Linux x86_64 (static musl build). That's the only target that is built and tested. Other targets (aarch64, macOS) may come later if someone needs them.
- **Local development:** gates run natively (`scripts/check`), on macOS as well. Linux-specific behaviour (inotify, musl) is tested by CI, which builds and tests the x86_64 Linux target on every push.
- **Terminals:** must work with 256 colors and a minimum 80×24 terminal; use truecolor when available; respect `NO_COLOR`.
- **Distribution:** GitHub Releases (prebuilt binaries), `.deb` / `.rpm` packages, `cargo install --git`, and the Homebrew tap `piterx93/tap`.
- **Rename from jlv:** the `vilog` deb and rpm replace and conflict with `jlv`; the tap renames the formula, so `brew upgrade` moves jlv users over; GitHub redirects the old repository URL; the old config path is still read (§11). There is no `jlv` command any more.
- **Visibility:** the repo was private until 1.0 and has been public since.
- **License:** MIT OR Apache-2.0.

### 5.1 CI/CD (GitHub Actions) — set up first, in M0

**Branching model:** `feature/*` → PR into `develop` → PR into `main`.

| Trigger | Tests | Packages | Version | Published to |
|---------|-------|----------|---------|--------------|
| Push to any other branch, PRs into `develop`/`main` | yes | no | — | — |
| Push to `develop` | yes | yes | `X.Y.Z-dev.<run>` (e.g. `0.3.0-dev.42`) | Workflow artifacts + rolling GitHub **pre-release** `dev` (replaced on every push) |
| Push to `main` | yes | yes | `X.Y.Z` from `Cargo.toml` | GitHub **Release** `vX.Y.Z` |

`vilog --version` also prints the short commit hash for CI builds, e.g. `vilog 0.3.0-dev.42 (a1b2c3d)`.

**Test job** (runs on every trigger, `ubuntu-latest`):
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --all`
- `cargo llvm-cov --fail-under-lines 80` (line coverage of the library crate)
- A final aggregate job, **`ci-required`**, fails if any check job failed. This gives branch protection one stable check name to require, even if the check jobs are later split or extended.

Mutation testing doesn't run in CI: on a hosted runner the full suite takes hours. It runs locally before a change is pushed (§14.1).

**Package job** (only `develop` and `main`; `needs: ci-required`, so it never runs on red tests):
- Target: `x86_64-unknown-linux-musl`. The built binary is smoke-tested with `--version`.
- Artifacts: `vilog-<version>-<target>.tar.gz`, `.deb` (`cargo-deb`), `.rpm` (`cargo-generate-rpm`), `SHA256SUMS`.
- The version is injected at build time via `VILOG_VERSION` / `VILOG_COMMIT`; CI never edits `Cargo.toml`. On `develop`, `Cargo.toml` holds the *next* planned version, so it must be bumped right after each release.
- deb/rpm get `X.Y.Z~dev.<run>`, so that package managers sort dev builds before the release.
- On `main`, the job fails if the tag `vX.Y.Z` already exists. This forces a version bump before each release.

**Merge blocking** (GitHub repository ruleset on `develop` and `main`). Disabled until the repo went public at 1.0, enabled since.
- Changes must go through a pull request; direct pushes are not allowed.
- The required status check **`ci-required`** must pass, and the branch must be up to date before merging.
- Force-pushes and branch deletion are blocked.
- The ruleset lives in `scripts/branch-ruleset.json` and is applied by `scripts/setup-branch-protection.sh`, so it can be reproduced.

## 6. Log formats

A format says how to find where an entry starts and which of its parts are the timestamp, level, thread, logger and message. Every part but the message is optional: a format without a thread or logger simply has no such column (§7.2). There are four kinds of format: **text** (a regex over the start of a line, §6.1), **JSON lines** (§6.2), **logfmt** (§6.3) and **raw** (every line is an entry, no columns). Built-in formats and the ones defined in the config (§11) are the same kind of data; `--format` and `P` treat them alike.

### 6.1 Text presets

| `--format` | Library / default layout | Example line |
|---|---|---|
| **Java** | | |
| `logback` | Logback / Log4j2 classic | `2026-10-07 12:00:00.123 [http-nio-8080-exec-3] INFO  com.example.OrderService - Order 42 created` |
| `logback-basic` | Logback default (BasicConfigurator) | `12:00:00.123 [main] INFO  com.example.App -- Started` |
| `spring3` | Spring Boot 3.x | `2026-10-07T12:00:00.123+02:00  INFO 12345 --- [myapp] [           main] o.s.b.SpringApplication                  : Started App in 2.1s` |
| `spring2` | Spring Boot 2.x | `2026-10-07 12:00:00.123  INFO 12345 --- [           main] o.s.b.SpringApplication                  : Started App` |
| `wildfly` | WildFly server.log | `2026-10-07 12:00:00,123 INFO  [org.jboss.as.server] (Controller Boot Thread) WFLYSRV0025: WildFly Full 33.0.0.Final started` |
| `wildfly-console` | WildFly console | `12:00:00,123 INFO  [org.jboss.as] (MSC service thread 1-2) WFLYSRV0049: ...` |
| **Python** | | |
| `python` | `logging`, `%(asctime)s - %(name)s - %(levelname)s - %(message)s` | `2026-10-07 12:00:00,123 - app.orders - WARNING - Payment retry 1/3` |
| `python-basic` | `logging.basicConfig()` default | `WARNING:app.orders:Payment retry 1/3` |
| `gunicorn` | gunicorn error log | `[2026-10-07 12:00:00 +0200] [12345] [INFO] Booting worker with pid: 12345` |
| `uvicorn` | uvicorn default | `INFO:     127.0.0.1:52100 - "GET /orders HTTP/1.1" 200 OK` |
| **.NET** | | |
| `serilog` | Serilog file sink default | `2026-10-07 12:00:00.123 +02:00 [INF] Order 42 created` |
| `serilog-console` | Serilog console default | `[12:00:00 INF] Order 42 created` |
| `nlog` | NLog default layout | `2026-10-07 12:00:00.1234\|INFO\|MyApp.OrderService\|Order 42 created` |
| `dotnet-console` | Microsoft.Extensions.Logging simple console | `info: MyApp.OrderService[0]` with the message on the next line, indented; or on one line with `SingleLine` |
| **Go** | | |
| `zap-console` | zap development config | `2026-10-07T12:00:00.123+0200	INFO	orders/service.go:42	Order 42 created` (tab-separated, caller optional) |
| `go-log` | standard `log`, `LstdFlags` | `2026/10/07 12:00:00 Order 42 created` (optional `.000000` with `Lmicroseconds`; no level) |
| **Any** | | |
| `raw` | fallback | Any line; no columns, plain viewing/search only. |

Node.js libraries (pino, winston, bunyan) log JSON by default and are covered by §6.2; plain `console.log` output is shown raw.

gunicorn's pid goes in the thread column. Formats without a timestamp (`python-basic`, `uvicorn`, `dotnet-console`) date nothing: their entries have no time, the time filter hides them and the histogram skips them.

Variants to tolerate: `.` or `,` as the fraction separator; `-` or `--` before the message; optional Spring app-name group; padded levels (`INFO `); ANSI color escapes (stripped before parsing — WildFly console logs and many dev setups contain them).

### 6.2 JSON lines

One JSON object per line, as written by logstash-logback-encoder, ECS loggers, zap, slog, pino, bunyan, winston, structlog, python-json-logger, Serilog's compact JSON (CLEF) and Microsoft's JSON console formatter. Each column comes from the first key of its candidate list that the file uses:

| Column | Candidate keys, in order |
|---|---|
| timestamp | `@timestamp`, `timestamp`, `time`, `ts`, `@t`, `Timestamp`, `asctime` |
| level | `level`, `log.level`, `levelname`, `severity`, `@l`, `LogLevel` |
| logger | `logger_name`, `logger`, `log.logger`, `name`, `caller`, `Category`, `SourceContext` |
| thread | `thread_name`, `thread`, `process.thread.name` |
| message | `message`, `msg`, `event`, `@m`, `@mt`, `Message`, `log` |
| exception | `stack_trace`, `error.stack_trace`, `stacktrace`, `exception`, `exc_info`, `@x`, `Exception`, `stack`, `err.stack` |

- A dotted candidate matches a literal dotted key (`"log.level"`, as ECS loggers write it) and the same path one object deep (`{"log":{"level":…}}`). Deeper nesting isn't searched.
- **Mapping per file:** the keys of timestamp, level, logger, thread and message are chosen once, from the detection sample (§6.4): for each column the first candidate present in at least half of the sampled JSON lines. Every line is then read with the same mapping. A file whose lines have no message key still opens as JSON; its message column shows the raw line. The exception is not part of the mapping, since exceptions are too rare to be on half of the lines: on each line the first exception candidate present (not empty, not `null`) is that line's exception.
- **Levels:** strings go through the aliases of §6.6. Numbers are read on the pino/bunyan scale (10 trace … 60 fatal). A CLEF file (one that maps `@t`) maps the level to `@l` however few lines carry it, and a line without `@l` means Information.
- **Timestamps:** strings go through §6.7; unquoted numbers are epoch time (§6.7).
- **Exceptions:** an entry has an exception when its exception field is present, not empty and not `null`.
- **Docker `json-file`:** `{"log":"…\n","stream":"stdout","time":"2026-10-07T10:00:00.123456789Z"}` is an ordinary mapping: message from `log` with the trailing newline dropped, timestamp from `time`. The inner line is not parsed again.
- A line that isn't a JSON object (doesn't start with `{` after whitespace) continues the previous entry, like a stack trace line in a text format.
- **Display:** values are shown decoded (`\n`, `\"`, `\uXXXX`, …). The row shows the first line of the message. Details show the whole message, the exception with one line per `\n`, then every other field as `key: value` in file order. Search, the text filter and highlights work on the decoded text; export and copy keep the original line.

### 6.3 logfmt

`key=value` pairs separated by spaces, values quoted when they contain spaces, `\"` inside quotes:

| Library | Example line |
|---|---|
| Go `slog` TextHandler | `time=2026-10-07T12:00:00.123+02:00 level=INFO msg="Order 42 created" order_id=42` |
| logrus text formatter | `time="2026-10-07T12:00:00+02:00" level=info msg="Order 42 created"` |

Columns come from the same candidate keys and the same per-file mapping as JSON (§6.2). For detection (§6.4), a line counts as logfmt only when it has a recognised timestamp or level key and a message key, so a `key=value` inside a Java message doesn't make a file logfmt. Once a file is read as logfmt, any line that starts with a `key=` starts an entry.

### 6.4 Auto-detection

- Sample the first ~200 non-empty lines (for `.gz`: the first decompressed lines).
- Score each format by the ratio of lines that start an entry in it. Continuation lines (indented lines, exception lines of §6.5) are left out of the sample.
- Pick the highest score ≥ 60%; otherwise fall back to **raw** and say so in the header.
- Ties go to the earlier format in this order: formats from the config (§11), text presets with a date, JSON lines, logfmt, then the presets without a date or level (`python-basic`, `uvicorn`, `dotnet-console`, `go-log`), which are the loosest.
- For JSON and logfmt the same sample also chooses the key mapping.
- User override: `--format <name>` and `P` to cycle formats at runtime.

### 6.5 Multi-line entries and exceptions

- Any line that doesn't start an entry belongs to the previous entry.
- An entry contains an **exception** if a continuation line is one of:

  | Ecosystem | Lines |
  |---|---|
  | Java | `^\s+at `, `^Caused by: `, `^\s+\.\.\. \d+ (more\|common frames omitted)`, a `^[\w.$]+(Exception\|Error)(: \|$)` header |
  | Python | `Traceback (most recent call last):`, `  File "…", line N`, `During handling of the above exception…`, `The above exception was the direct cause…` |
  | Go | `panic: `, `goroutine N [`, tab-indented frames `\t/…/file.go:N`, the function line above a frame (`main.run(0xc000012345, {…})`, `panic({…})`; arguments only hex words (optionally ending in `?`), `{…}` or `...`), `created by pkg.fn [in goroutine N]`, `[signal SIG…: …]`, `...additional frames elided...` |
  | Node.js | `^\s+at ` and the `…Error: ` header, as for Java |
  | .NET | `^\s+at `, ` ---> `, `--- End of inner exception stack trace ---`, `--- End of stack trace from previous location ---` |

  The first line of an entry with a recognised start is never checked, so a message like `Error: …` doesn't count; a headless entry (a file that starts mid-trace) has all of its lines checked. In JSON lines the exception field decides (§6.2).
- **Message on the next line:** when an entry's first line has an empty message and the entry has continuation lines, the row shows its first continuation line, trimmed. `dotnet-console` relies on this.
- In the list view, multi-line entries are **folded** by default and show a marker (e.g. `[+37]`). The user can expand or collapse one entry, or all entries.
- Keys exist to jump to the next or previous entry with an exception.

### 6.6 Severity levels & colors

Six levels. Level names are matched ignoring case and padding; the level column shows the canonical name.

| Level | Also read as | pino/bunyan | Color (default) |
|-------|--------------|-------------|-----------------|
| TRACE | `TRCE`, `VRB`, `VERBOSE`, `FINER`, `FINEST` | 10 | gray / dim |
| DEBUG | `DBG`, `DBUG`, `FINE` | 20 | cyan |
| INFO | `INF`, `INFORMATION`, `NOTICE`, `CONFIG` | 30 | green |
| WARN | `WARNING`, `WRN` | 40 | yellow |
| ERROR | `ERR`, `FAIL`, `SEVERE` | 50 | red |
| FATAL | `FTL`, `CRITICAL`, `CRIT`, `PANIC`, `EMERG`, `ALERT` | 60 | bold white on red |
| unknown | | | default fg |

The level filter (§9.3) and `-l` accept the aliases too. Colors are configurable (§11).

### 6.7 Timestamps

- **Shapes:** a date as `yyyy-MM-dd` or `yyyy/MM/dd`; `T` or a space before the time; `HH:mm:ss` with an optional fraction of 1–9 digits after `.` or `,`; an optional offset `Z`, `±hh:mm`, `±hhmm` or `±hh`, with or without a space before it. A timestamp without an offset is local time.
- **Epoch time** (unquoted numbers in JSON lines and logfmt): the number of integer digits gives the unit — up to 11 seconds (a fraction is allowed), 13 milliseconds, 16 microseconds, 19 nanoseconds.
- **Time only** (WildFly console, Logback default, Serilog console): take the date of the last entry from the file name if it contains one (`server.log.2026-10-06`, `app-2026-10-06.log.gz`), otherwise from the file's mtime, taken with its time of day so that a file flushed just after midnight isn't dated a day late. Then walk backwards. When the earlier entry's time of day is more than 2 h *after* the later one, midnight was crossed, so decrement the day; smaller steps back are out-of-order logging or a DST fall-back. When the earlier entry's time of day is more than 23 h 59 min *before* the later one, a late thread wrote the later entry just before midnight, so the earlier entry belongs to the next day.
- A shape the reader doesn't know can be given as a `timestamp_format` for a config format (§11).
- Entries without a parseable timestamp inherit the timestamp of the previous entry (used for time filters and the histogram).

## 7. UI

### 7.1 Layout

```
┌ vilog ─ server.log ─ 1.8 GB ─ WildFly ──────────── FOLLOW ● ── E:37 W:212 ──┐
│ Filters: level[W E F]  logger[com.myco.* !org.hibernate]  time[last 10m]     │
├──────────────────────────────────────────────────────────────────────────────┤
│ ▁▁▂▁▁▃▁▁▁▁▂▁▁▁▁▁▁▇▅▁▁▁▁▁▂▁▁▁▁▁▁▁▁▁▁▁▃▁▁▁ (errors+warns per minute)            │
├─────────────────────┬───────┬──────────────────────┬─────────────────────────┤
│ TIMESTAMP           │ LEVEL │ LOGGER               │ MESSAGE                 │
│ 2026-10-07 12:00:01 │ INFO  │ c.m.order.OrderSvc   │ Order 42 created        │
│ 2026-10-07 12:00:02 │ WARN  │ c.m.pay.PayClient    │ Retry 1/3 ...           │
│▶2026-10-07 12:00:03 │ ERROR │ c.m.pay.PayClient    │ Payment failed [+37]    │
├─────────────────────┴───────┴──────────────────────┴─────────────────────────┤
│ Details: com.mycompany.pay.PayClient (default task-12)                       │
│ java.net.SocketTimeoutException: Read timed out                              │
│     at java.base/java.net.SocketInputStream.read(...)                        │
├──────────────────────────────────────────────────────────────────────────────┤
│ F1Help F2Level F3Search F4Filter F5Time F6Logger F7Bkmk F8Export F10Quit     │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Header:** file name, size, detected format, follow status with the liveness dot (`FOLLOW ●`, `PAUSED ◐`, `PAUSED +N new ◐`, `READ-ONLY ○`, see §8), counters per level. The counters respect the logger, time and text filters but ignore the level filter, so hidden levels don't drop to zero.
- **Filter bar:** active filters; hidden when there are none.
- **Histogram (toggleable):** WARN/ERROR counts per time bucket. The bucket size adapts to the visible range. Selecting a bucket (mouse click or keys) sets the time filter to that bucket.
- **Table:** Timestamp | Level | Logger | Message, plus optional **Thread** (hidden by default, toggle key). Columns the format doesn't have are left out (§7.2).
- **Details panel (toggleable, Enter):** full logger name, thread, full message, fully expanded stack trace, scrollable; for JSON lines also the other fields (§6.2). Parts the entry doesn't have are left out.
- **Footer:** htop-style function-key hints.

### 7.2 Columns

- A column the format doesn't have is not drawn, and the message gets its width. Toggling the thread column on a format without threads shows a notice instead of an empty column.
- Timestamp format in the table is configurable (default `yyyy-MM-dd HH:mm:ss.SSS`, or `HH:mm:ss.SSS` when narrow).
- Logger names are abbreviated Logback-style to fit the column (`com.mycompany.pay.PayClient` → `c.m.pay.PayClient` → `c.m.p.PayClient`); Python and .NET dotted names abbreviate the same way. Names with `/` (Go callers like `internal/orders/service.go:42`) keep their end and drop leading path segments. The full name is shown in Details.
- Long messages are truncated by default with horizontal scroll (`←/→`); a toggle switches to wrapping.
- A gutter left of the table shows file line numbers in a dim colour (first line of each entry, every line of an unfolded one). Shown by default, `#` hides it.

### 7.3 Mouse

Wheel scrolls; click selects a row; double-click toggles fold; click on a histogram bucket sets the time filter. Mouse support can be disabled in config, since some SSH and tmux setups don't handle it well.

## 8. Follow mode

- Starts in follow mode at end of file (override `--no-follow`, which opens at the top).
- Behaves like `less +F`. Any move away from the bottom **pauses** follow. The header shows `FOLLOW` while following, `PAUSED` during a pause and `PAUSED +N new` once entries arrive during it (N counts entries, not lines). `F` toggles follow; `End` and `G` jump to the bottom and resume.
- **Liveness dot:** after the follow status (`FOLLOW ●`, `PAUSED ○`, `PAUSED +N new ◐`, `WAITING FOR FILE ○`, `READ-ONLY ○`) a dot says whether the file is alive, whether or not the view follows it:
  - green `●`: the file changed (grew, was truncated or replaced) in the last 10 s;
  - yellow `◐`: the last change was 10 s to 5 min ago;
  - grey `○`: no change for 5 min or more, a `.gz` file, or the file is gone.

  The time counts from the last change the watcher reported, or from opening for a file that hasn't changed since. The shape carries the state on its own, so it reads under `NO_COLOR` too. The screen redraws when the dot is due to change, without a key press.
- File watching uses `notify` (inotify/kqueue), with a polling fallback every 250 ms for NFS and similar filesystems.
- **Rotation handling** (`tail -F` semantics):
  - inode change (rename+create, or delete and recreate): finish reading the old file, then reopen the path and show the new file from its start.
  - size shrink (copytruncate): drop the rows of the old content, whose bytes are gone, and show `── file truncated ──` as the first row, followed by the new content. A reader that touches the old content past the new end reads zeros instead of crashing until the truncation is noticed.
  - if the file disappears and nothing takes its place, let go of it: unmap and close it, so that deleting it frees its disk space, and empty the view (rows, filter and search results, folds, bookmarks, details pane; a waiting export is cancelled). Show `WAITING FOR FILE` in the header and keep retrying; a file that shows up at the path again is shown from its start, as after an inode change.
- Active filters apply live to new entries.
- `.gz` files (detected by the gzip magic bytes, not the name) are read-only: never followed, `READ-ONLY` in the header. A damaged file shows what could be decompressed, with `truncated .gz` in the header. Progress is printed on stderr while decompressing, before the TUI starts.

## 9. Navigation, search & filter

### 9.1 Navigation

`j/k`, `↑/↓` move by line; `PgUp/PgDn` move by page, `Ctrl-u/d` by half a page; `g/G` and `Home/End` go to the top or bottom; `:<n>` goes to the entry containing file line n (the numbers in the gutter); `t` jumps to a time (`t 12:30`).

### 9.2 Search (highlight & jump)

- `/` searches forward and `?` searches backward; `n/N` go to the next or previous match.
- Modes: plain (default), regex, case-insensitive. Toggled in the prompt with `Ctrl-r` and `Ctrl-i` (terminals send `Ctrl-i` as `Tab`, so both work).
- All visible matches are highlighted. The status bar shows `match 3/57`.
- Search also covers folded stack-trace lines; a match inside a folded entry marks the entry.
- In JSON lines and logfmt, search runs on the decoded text (§6.2): `"quoted"` finds `\"quoted\"` in the file.

### 9.3 Filters (hide non-matching)

All active filters combine with **AND** and apply live, including in follow mode. `x` clears all filters; `Esc` closes a filter panel without applying pending edits.

| Filter | Behavior | Keys |
|--------|----------|------|
| **Level** | Independent toggles per level | `1`–`6` = TRACE…FATAL, `F2` opens panel |
| **Logger** | List of rules: `com.myco.*` (prefix include), `OrderService` (substring include), `!org.hibernate` (exclude). Includes are OR'd together; excludes always win. | `F6` panel; `c` = "only this logger" from selected row; `C` = "hide this logger" |
| **Time** | Presets: last 1/5/10/30/60 min, 24 h. Range input: `12:00-12:30`, `12:00:15-12:00:45`, `12:00-` (open-ended), full `2026-10-07 12:00..2026-10-07 13:00`. A range without a date uses the date of the last entry. | `F5` panel; quick keys for presets |
| **Text** | Plain/regex/ignore-case over message + continuation lines, decoded for JSON and logfmt | `F4` / `&` |

**Time base:** "last N minutes" is relative to the **last entry in the file** by default, so it works for old logs. A toggle (`F5` panel) switches to the **wall clock**. The active base is shown in the filter bar (`last 10m (file)` / `last 10m (now)`).

### 9.4 Bookmarks

`m` toggles a bookmark on the selected entry; `'` / `"` jump to the next or previous bookmark; bookmarks are shown with a gutter marker. Bookmarks last for the session only.

### 9.5 Export & clipboard

- `F8` / `e` exports the currently filtered entries (original lines, including stack traces, with ANSI escapes stripped) to a file. The prompt suggests `<name>.filtered.log` in the current directory. JSON lines are exported as they are in the file, not decoded.
- `y` copies the selected entry (original lines, with stack trace, ANSI escapes stripped) to the clipboard via **OSC 52**, which works over SSH. Locally there is a native clipboard fallback.

## 10. CLI

```
vilog [OPTIONS] <FILE>

  -f, --format <NAME>       Force a format: a built-in one (§6.1–6.3) or one from
                            the config (§11)
      --no-follow           Open at top, don't follow
  -l, --level <LEVELS>      Initial level filter, e.g. WARN,ERROR
  -c, --logger <RULES>      Initial logger filter, e.g. 'com.myco.*,!org.hibernate'
                            (--class is kept as an alias)
  -s, --since <SPEC>        Initial time filter, e.g. 10m or 12:00-12:30
      --config <PATH>       Config file path
      --no-mouse            Disable mouse
```

An unknown `--format` name is an error that lists the valid ones. Every `--format` value of 1.x still works.

## 11. Configuration

`$XDG_CONFIG_HOME/vilog/config.toml` (default `~/.config/vilog/config.toml`), or `--config <PATH>`. When the vilog file doesn't exist but `jlv/config.toml` does, the old file is read and a notice in the header says where to move it, until the first key.

```toml
[ui]
timestamp_format = "%Y-%m-%d %H:%M:%S%.3f"
show_thread = false
show_histogram = true
show_line_numbers = true
mouse = true
wrap = false

[colors]
trace = "darkgray"
debug = "cyan"
info  = "green"
warn  = "yellow"
error = "red"
fatal = { fg = "white", bg = "red", bold = true }
line_number = "darkgray"

[follow]
poll_interval_ms = 250

[filters]
default_class_excludes = []   # logger excludes, e.g. ["org.hibernate.SQL"]

[keys]
# override any binding, e.g.
# toggle_follow = "F"

# A text format of your own: a regex over the start of an entry with named groups
# timestamp, level, thread, logger and message, all optional. Without a message
# group the message is the rest of the line.
[formats.billing]
regex = '^(?P<timestamp>\d\d\.\d\d\.\d{4} \S+) (?P<level>\w+) (?P<logger>[\w.]+): '
timestamp_format = "%d.%m.%Y %H:%M:%S"   # only for shapes §6.7 doesn't read

# Other keys for a JSON or logfmt log.
[formats.audit]
kind = "json"
level = "sev"
message = "text"
```

Config formats come first in detection (§6.4), appear under `P` and are accepted by `--format`. An invalid regex or `timestamp_format` is a config error naming the key.

## 12. Key bindings (defaults)

| Key | Action |
|-----|--------|
| `F1` | Help overlay (`?` is backward search) |
| `j k ↑ ↓` | Move |
| `PgUp PgDn Ctrl-u Ctrl-d` | Page |
| `g G Home End` | Top / bottom |
| `F` | Toggle follow |
| `P` | Cycle format |
| `/` `?` `n` `N` | Search |
| `F2`, `1`–`6` | Level filter |
| `F4` / `&` | Text filter |
| `F5` | Time filter |
| `F6`, `c`, `C` | Logger filter / only this / hide this |
| `x` | Clear all filters |
| `Enter` | Details panel |
| `Space` / `z` / `Z` | Fold toggle / fold all / unfold all |
| `]` / `[` | Next / previous exception |
| `m` `'` `"` | Bookmarks |
| `y` | Copy entry |
| `F8` / `e` | Export filtered |
| `T` | Toggle thread column |
| `#` | Toggle line numbers |
| `H` | Toggle histogram |
| `w` | Toggle wrap |
| `← →` | Horizontal scroll |
| `q` / `F10` | Quit |

## 13. Non-functional requirements

| Area | Requirement |
|------|-------------|
| Startup | First screen (tail of file) visible in < 300 ms for a 2 GB file, text or JSON; indexing continues in the background with a progress indicator. |
| Indexing | ≥ 500 MB/s on SSD for the entry index, text and JSON alike. |
| Memory | Index ≤ ~32 bytes per entry; the file's content is never loaded into memory in full (mmap + on-demand parsing of visible rows). |
| Filtering | Full re-filter of 2 GB in < 3 s, running in the background and cancellable; results stream in progressively. |
| Rendering | Scrolling stays smooth (no frame > 16 ms) regardless of file size. |
| Follow latency | New line visible < 250 ms after write. |
| Idle | ~0% CPU when the file isn't changing. |
| Robustness | Never crashes on invalid UTF-8 (lossy decode), binary garbage, malformed JSON, extremely long lines (cap rendering, e.g. at 64 KB per line), or a file truncated while it is read. |

## 14. Architecture

```
           ┌─────────────┐   events   ┌──────────────┐
 file ───▶ │ Source      │──────────▶ │ Indexer      │  entry index (offset, len,
 (.log/.gz)│ mmap/notify │            │ (bg thread)  │  ts, level, logger span, flags)
           └─────────────┘            └──────┬───────┘
                                             │
                                      ┌──────▼───────┐
                                      │ Filter engine│  Vec<u32> of matching entries,
                                      │ (bg, cancel) │  incremental on append
                                      └──────┬───────┘
                                             │
                                      ┌──────▼───────┐
                                      │ App / UI     │  ratatui; parses only visible rows
                                      └──────────────┘
```

- **Source:** `memmap2` for plain files (remap on growth); `.gz` is decompressed to a temp file once (via `flate2`) and then treated as a plain file. A SIGBUS handler turns reads past a truncation into zeros (§8).
- **Formats:** a format is data — an id, a display name and a kind (`Regex`, `Json`, `Logfmt`, `Raw`) with what that kind needs (the regex and which parts it has; the key mapping). Built-in formats are a static table; config formats are built once at startup and live for the whole run, so parsers stay cheap to copy and need no reference counting. Dispatch is a `match` on the kind.
- **JSON and logfmt:** a small hand-written scanner finds keys and value spans without allocating and without rejecting invalid UTF-8 or a truncated line; it skips over strings with `memchr`. Spans point at the raw value in the line, so the index stores them as it does for text formats; values are decoded only when shown, searched or filtered. `serde_json` is used in tests only, as the reference the scanner is checked against.
- **Index:** a compact struct per entry, built in chunks; it grows on append, and rotation triggers a rebuild. The per-entry record stays the same size for every kind of format; a logger span longer than 64 KB is not kept.
- **Filter engine:** level/time/logger checks use index fields only, so they are fast. Text and regex checks read entry bytes. Each filter generation can be cancelled.
- **Crates:** `ratatui`, `crossterm`, `notify`, `memmap2`, `regex`, `jiff`, `flate2`, `serde` + `toml`, `clap`, `arboard` (local clipboard), `memchr`, `libc`.
- **Layout:** one crate with a library and a thin binary. `src/main.rs` only parses arguments and runs the app; everything else lives in library modules (`source`, `format` with `format::json` and `format::logfmt`, `index`, `filter`, `follow`, `app`, `ui`, `config`). `format`, `index` and `filter` must not depend on `ui` or `ratatui`.

### 14.1 Testing & quality

- **Test-first.** Every feature starts with failing tests that describe the behaviour, including edge cases. Implementation changes only non-test code; if a test turns out to be wrong, it is fixed as a separate step.
- **Unit tests** live in a `tests.rs` file next to the module (`#[cfg(test)] mod tests;`), not inline, so test and production code change in separate files.
- **Integration tests** in `tests/`: a synthetic fixture log per built-in format (including stack traces, midnight rollover and ANSI codes), checked table-driven over every built-in format, and rotation scenarios (rename, copytruncate, delete+recreate). Every fixture is detected as its own format and as no other.
- **UI** is tested by rendering into ratatui's `TestBackend` and comparing against inline `insta` snapshots written by hand as the expected screen. App state and key handling are tested without rendering.
- **Property tests** (`proptest`) for parsers, the JSON and logfmt scanners, class abbreviation and the filter engine, e.g. "never panics on arbitrary bytes", "scanner agrees with serde_json", "engine result equals a naive filter".
- **Mutation testing** (`cargo mutants`): no surviving mutants in changed code before a change is pushed; equivalent mutants are excluded explicitly with a reason. The full run is started by hand, locally.
- **Coverage:** at least 80% line coverage, enforced in CI.
- **Benchmarks:** `criterion` on a generated 2 GB synthetic log, text and JSON, run manually to verify §13; not part of CI.

## 15. Milestones

| Milestone | Scope | Release |
|-----------|-------|---------|
| **M0** Skeleton | GHA pipeline per §5.1, quality gates (§14.1), CLI parsing, raw viewer with scroll | 1.0 |
| **M1** Parsing | Presets + auto-detection, columns, level colors, multi-line folding, class abbreviation, background index | 1.0 |
| **M2** Follow | notify + polling, rotation & truncation handling, `less +F` pause/resume, `.gz` read | 1.0 |
| **M3** Search & filter | Search (plain/regex/icase), level/class/time/text filters, live filtering, filter bar, line numbers | 1.0 |
| **M4** UX | Details panel, exception jumping, bookmarks, export, clipboard (OSC 52), histogram, mouse, help overlay | 1.0 |
| **M5** Release 1.0 | Config file, key remapping, docs/README with GIFs, LICENSE files, branch protection, public repo, Homebrew tap | 1.0 |
| **M6** Rename to vilog | Crate, binary and texts; config path with the jlv fallback; release assets and packages replacing jlv; tap formula rename; README and demos | 2.0 |
| **M7** Format core | Formats as data, optional columns and the logger naming, one timestamp reader with epoch, level aliases, exception lines of every ecosystem | 2.1 |
| **M8** JSON lines | Scanner, key mapping and detection, values and index, decoded display/search/filter | 2.1 |
| **M9** logfmt | Tokenizer on the JSON mapping, detection | 2.2 |
| **M10** Text presets | Python, .NET and Go presets with their stack traces, message on the next line, detection order | 2.3 |
| **M11** Custom formats | `[formats]` in the config: regex, `timestamp_format`, JSON/logfmt key overrides | 2.4 |

2.0 is a major release because the command, the packages and the config path change name; everything after it only adds formats.

### Backlog

- Multiple files merged chronologically, with a Source column
- stdin / pipe input (`kubectl logs -f | vilog -`)
- Saved filter presets
- Persistent bookmarks
- User-chosen extra columns (MDC values such as traceId, other JSON fields)
- JSON stack traces unfolded as rows of the table instead of in Details only
- Logback pattern → parser

## 16. Open questions

None at the moment.

### Resolved

- **Sample logs:** formats are validated against synthetic fixtures built from the examples in §6 and the upstream docs of each library.
- **Date for time-only formats:** a date in the file name wins over mtime (§6.7).
- **Level counters:** current filter scope without the level filter (§7.1).
- **Source column:** arrives with multi-file views (backlog).
- **`?` key:** backward search; the help overlay is `F1` only.
- **Final name:** vilog, from 2.0 (§5, §15).
- **Numeric levels:** only on the pino/bunyan scale; Python's `levelno` uses another one (20 is INFO there, DEBUG in pino) and is not read as a level.
