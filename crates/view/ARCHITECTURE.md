# `harness-view` — architecture

The factory view: a local web server that draws the harness as an isometric
plant, read from the traces the harness leaves behind. A **second outer ring**
beside `harness-launcher` — the only other crate allowed to build a concrete
adapter — and it **writes nothing the harness reads**. The crate graph is in
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md); the Rust rules in
[../../CLAUDE.md](../../CLAUDE.md).

```
harness-view (bin)  →  harness-workflows (lib)  →  harness-core (lib)
```

It depends on `harness-workflows` for two words only — the `harness:*` labels
and `milestone_branch` — so the sign and the store cannot disagree with the
router about what a label means or what a milestone's branch is called.

## What it shows, and from where

| level | what | read from |
| --- | --- | --- |
| **A** the plant | one chimney per model, smoking while a session on it is open; a sign (to do / done); a door | the latest run of each line; `costs.tsv`; the GitHub board |
| **B** inside | six rooms: lines, office, store, value, control, construction | the blueprint; the same picture |
| **C** a room | the lines station by station, the product on the belt, an employee beside each station in progress — several when a parallel watch runs the line on several lanes; a crew that shares a station fans out and opens a pane to pick one | `run.log`, `prompts.md`, `stream.jsonl` of every fresh run; `watch.log` |
| **D** a pane | logs live, a station's cost, an issue body, the dashboards, the versions | `/api/runs/…` tails; `/api/issues/…`; the picture |

An **employee** is a run believed to be at work: the watch dispatched its
workflow and has not reported back, or its folder was written to in the last
two minutes. The harness logs no "run over" line, so this is a judgement, and
it lives in one function (`assemble::is_live`) with its two thresholds.

## The steward

The grinning mascot in a blue jumpsuit giving a thumbs-up, on the forecourt and in the hall.
Click them — or the `steward` button in the header — and the pane becomes a
terminal: an **interactive Claude Code**, started in the harness checkout
with the plant's standing orders appended to its system prompt
(`domain::steward::briefing`). The human types; the steward starts the
watch, stops it, reads the ledgers, taps a label. Nothing in the view does
any of that itself.

```
browser  xterm.js ──WebSocket /api/steward/term──▶  server::attend
                                                        │  keystrokes / size / restart
                                                        ▼
                                                   desk::Desk  ── one program, kept alive
                                                        │  between visits; 256 KB replay
                                                        ▼
                                                   adapters::pty  ── portable-pty, `claude`
```

The desk is the `tmux` of the page: the program outlives the pane, every
connection subscribes to the same output, a late visitor gets the screen
replayed and a resize to make the program redraw. The program starts on
the first visit and again on the first keystroke after it exits. The desk
is a fake-able seam (`ports::TerminalFactory`); the pseudo-terminal itself
is tested against a real `cat`.

The briefing names exact commands — `pgrep`, `setsid nohup ./target/release/harness watch …`,
`pkill`, `gh issue edit … --add-label` — because the two mistakes a steward
must not make are starting a second watch and stopping one mid-session
without saying so. `--permission-mode` comes from the same `PERMISSION_MODE`
the launcher passes to every session. The page is bound to `127.0.0.1` for
a reason: this terminal is a shell in the checkout.

## The modules

```
src/main.rs            the wiring: two pollers, one server, one desk, one thread
src/cli.rs             what a human types; TARGET_REPO_URL, INTEGRATION_BRANCH, PERMISSION_MODE shared with the launcher
src/desk.rs            the steward's desk: one terminal kept alive between visits, scrollback, broadcast
src/domain/            the inside — no disk, no subprocess, no clock
  blueprint.rs         the rooms, the models, one line per workflow, station by station
  traces.rs            parsers: watch.log, run.log, prompts.md headers, the stream's first event, the ledger sums
  observe.rs           what one tick reads through the Traces port
  assemble.rs          Observed + board → Snapshot (who is live, where the product is, what smokes)
  snapshot.rs          the serializable picture the page receives
  steward.rs           the steward's standing orders, and the status the page asks for
src/ports/mod.rs       Traces (the disk), Board (GitHub), TerminalFactory/TerminalIo (the steward's program)
src/adapters/
  fs_traces.rs         .llocal/logs, read through core's own ledger readers
  gh_board.rs          core's GitHub port, read the way the router reads it
  pty.rs               a pseudo-terminal running `claude` — the one module that spawns a process
src/server.rs          axum: the page, the scripts, /render/… (the wasm bundle, from disk), /api/snapshot, /api/events (SSE), /api/runs/…, /api/issues/…, /api/steward, /api/steward/term (WebSocket)
static/                index.html, style.css, app.js (data, navigation, panes, the bridge to the renderer), vendor/ (xterm.js), render/ (built, not committed)
```

The drawing itself is **another crate**, `harness-view-render`: a Bevy scene
compiled to WebAssembly that owns the canvas and reports pointer events back
to `app.js` — see [../view-render/ARCHITECTURE.md](../view-render/ARCHITECTURE.md).
`scripts/build-render.sh` puts the bundle under `static/render/`; the page
says what to run when it is missing.

The hexagon holds inside this crate too: `domain` names no adapter, every
read goes through `ports`, and the adapters are built in `main.rs` and
nowhere else (`claude_bin`, which asks every known Claude Code install for
its version so the steward runs the newest, is one of them). The fakes used
by the tests are in-memory — a shelf of files (`observe::fake::Shelf`), an
echoing terminal (`desk::fake::Echoing`) — never a mock at the call site.

## Three decisions

- **Read-only, from the files.** The view reads what `harness watch` and the
  runs already write, and never asks the harness to emit anything for it. A
  structured event port in `harness-core` would be more precise; it can be
  added later without touching this crate's inside, because `observe` already
  reads through a port. The one hand that changes the plant is the human's,
  at the steward's terminal — a Claude Code they drive, not code of the view.
- **One picture, pushed.** The pollers assemble a `Snapshot`; the server
  streams it on `/api/events` whenever it changed (compared before it is
  stamped, so a clock alone wakes nobody). The page asks for nothing else
  except a log tail and an issue body, on demand and bounded.
- **The blueprint is declared, not derived.** `harness-workflows`' stage tables
  are built against ports and boxed actions; what a stage *looks like* is a
  fact of the view. The stage names in `blueprint.rs` are the ones the runs
  log, and `assemble`'s tests pin a real `run.log` shape against them.

## Running it

```
cargo run -p harness-view -- --demo          # the latest run shown live
cargo run -p harness-view -- --no-board      # never call gh
cargo run -p harness-view -- --no-steward    # no terminal, nobody to talk to
cargo run -p harness-view -- --static-dir crates/view/static   # edit the front without a rebuild
```

`TARGET_REPO_URL`, `INTEGRATION_BRANCH` and `PERMISSION_MODE` come from the
same `.env.local` the launcher reads. The page listens on `127.0.0.1:7878`
(`HARNESS_VIEW_PORT`).

## What is deliberately not here yet

Room 2's own conversation with Claude Code about an issue (the steward's
terminal is the plant-wide one), room 4's mock-up and data sources, room 6 —
each shows its place and says so. Level D is wired for the lines (station,
employee, line), the sign, the chimneys, the issues, the store, the control
room and the steward; the other components open a placeholder that names
what will come.
