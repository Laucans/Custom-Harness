# `harness-view` — architecture

The factory view: a local web server that draws the harness as an isometric
plant, read from the traces the harness leaves behind. A **second outer ring**
beside `harness-launcher` — the only other crate allowed to build a concrete
adapter — and it **writes nothing the harness reads** (a prompt edited
here is a draft the watch applies itself, see *The robots' prompts*). The
crate graph is in
[ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md); the Rust rules in
[../../CLAUDE.md](../../CLAUDE.md).

```
harness-view (bin)  →  harness-workflows (lib)  →  harness-core (lib)
```

It depends on `harness-workflows` for the words the plant shares with the
router — the `harness:*` labels and `milestone_branch` — so the sign and the
store cannot disagree with the router about what a label means or what a
milestone's branch is called; and for the data archive's layout
(`data_snapshot`'s index, file names and branch), so the pane reads exactly
what the watch writes.

## What it shows, and from where

| level | what | read from |
| --- | --- | --- |
| **A** the plant | one chimney per model, smoking while a session on it is open; a sign (to do / done); a door | the latest run of each line; `costs.tsv`; the GitHub board |
| **B** inside | six rooms: lines, office, store, infirmary, control, construction | the blueprint; the same picture |
| **C** a room | the lines station by station, the product on the belt, an employee beside each station in progress — several when a parallel watch runs the line on several lanes; a crew that shares a station fans out and opens a pane to pick one | `run.log`, `prompts.md`, `stream.jsonl` of every fresh run; `watch.log` |
| **D** a pane | logs live, a station's cost, how long a run, a machine or a trigger took — or has been running, ticking — an issue body, the dashboards, the versions | `/api/runs/…` tails; `/api/issues/…`; the picture |

An **employee** is a run believed to be at work: the watch dispatched its
workflow and has not reported back, or its folder was written to in the last
two minutes. The harness logs no "run over" line, so this is a judgement, and
it lives in one function (`assemble::is_live`) with its two thresholds.

## The steward, and the doctor

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

**The doctor** is a second desk, the same way: the coral, lobster-faced
physician in a white coat in the infirmary (room 4), and the `doctor`
button in the header. Their Claude Code opens on its own model and effort
(`--doctor-model`, `--doctor-effort`: Sonnet, high), briefed on the
instruments — `harness doctor --dry-run`, `errors.tsv`, the workspaces —
(`domain::doctor::briefing`), and with the check-up already asked as the
positional prompt (`domain::doctor::CHECKUP`), so the pane opens on a
diagnosis rather than an empty line. The one rule in the briefing: the
repair, `harness doctor`, discards uncommitted files, so the doctor runs it
only on the human's explicit go. Routes `/api/doctor` and `/api/doctor/term`;
`--no-doctor` leaves the infirmary empty. The infirmary's other props open
the same pane, and the chart on its wall opens the stops dashboard.

An agent whose run stopped badly or warned carries a **doctor button** in
its pane (`run.log` matches an error line, or the pane was opened on one).
A click is `POST /api/doctor/diagnose/{workflow}/{run}`: the server pastes
the tails of `run.log` and `session.log` into the doctor's terminal as a
question (`domain::doctor::diagnosis`, a bracketed paste then Enter,
summoning the doctor first if nobody sits there) and listens to the desk's
output until a reply ends with the run's mark — `DIAGNOSIS: <run> · colour`,
heard through escapes and wrapped lines by `doctor::diagnosed` — or the
program ends, or twenty minutes pass. The page polls
`GET /api/doctor/diagnoses`: the button pulses yellow while the doctor reads,
turns green on `done`, and then opens the doctor's pane, whose back arrow
returns to the agent. The question is worded so its own echo never carries
the mark. Questions are typed one at a time (`AppState::asking`), so two
clicks in a row are two questions the doctor answers in turn. A diagnosis
that ended is a fact the poller hands `harness-notify` (`Facts::diagnoses`):
an Info notification when answered, a Warning when not, each leading to the
agent's pane — so the human hears of it with the pane closed.

## The robots' prompts

A paid station — a robot, builder or inspector — shows its prompt on its
details tab and carries a **Prompt** tab. What a station sends is
`harness_workflows::prompts`' catalog: a key (`agent-loop/code`), the
stations that send it (a shared text, like the repository map or how the
tests are run, stands on several), and the default — the constant beside
the stage. The tab edits the stage's own text only; the EXECUTION CONTEXT
and SCOPE blocks stay the framework's.

**An edit is a draft, and only a start applies it.** Two stores, two
writers (`harness_core::domain::prompt_book`, `ports::store::prompts`,
JSON files in `harness-adapters`, written aside then renamed):

| store | written by | read by |
| --- | --- | --- |
| `.llocal/prompts.drafts.json` — one draft per prompt: a new text, or a revert | the view | the view; `harness watch`, once, when it starts |
| `.llocal/prompts.json` — the book: what runs, the last 5 edits, the last 3 reverted texts | `harness watch` alone, when it starts | every run, once (`dispatch::shared::prompts`) |

The view reads the book through its own `Prompts` port, which has no way
to write it. `harness watch`, starting, takes the drafts (renamed out of
the way, so one saved meanwhile waits for the next start), applies them
(`PromptBook::apply`) and journals `ChangesApplied` — an Info notification.
A revert — putting back an edit, a reverted text or the default — keeps
what it replaced among the **recently reverted** (3), the history's second
list behind a switch. Each run records in its own folder the text of every
prompt its line could send (`prompt-texts.json`).

**The wrench**, above the notification signs, turns blue while a change
waits (`GET /api/pending`). Its pane lists each change — a click opens the
machine's prompt tab — and a **restart** button (`POST /api/restart`):
a soft stop, then a start as soon as the watch has drained (polled every
3 s), or a start at once when none runs.

**A dry-run** replays a past run of the line with any version — the
editor's text, what runs, the draft, the default, an edit, a reverted
text — and shows the answer, the time and the cost. The view starts
`harness trial` (`ports::Trials`, `adapters::trial_proc`,
`HARNESS_TRIAL_COMMAND`), the text on its stdin so a draft is never
written where a run reads; the trial reads the run's `prompts.md` and
`prompt-texts.json`, reads back the values the old text was filled with
(`prompt_book::replay`), fills the version tried with them, keeps the
framework's blocks around, and asks once — `--permission-mode plan`, in a
clone of its own (`prompt-trial`), on the station's model. Its report,
`.llocal/trials/<id>/trial.json`, is what the tab lists. One at a time. A
run older than its `prompt-texts.json` is replayed against today's
default, and refused if that text changed since.

The pane warns, as the human types, of a `{placeholder}` the default
carries and the edit dropped, and of one nothing fills. Routes
`GET /api/prompts/{line}/{station}`, `POST /api/prompts/{edit,restore,withdraw}`,
`POST /api/prompts/trial`, `GET /api/prompts/trials?key=`; `domain::prompts`
puts the catalog, the book and the drafts together, and a test holds that
every robot of the blueprint shows a prompt and every prompt stands on a
station the plant draws.

## The janitor

An old man in teal coveralls leaning on a push broom, outside at the tower's
foot by the water, and the `janitor` button in the header. Their job is the
disk: `.llocal/` — the *yard*, where the clones, their build outputs and
dependencies, the runs' traces and the stores pile up. **No model is
involved**: what goes is decided by rules on a heap's name, weight and idle
time (`domain::cleanup`), the walk and the removal are the `Yard` port's
(`adapters::fs_yard`), and `janitor.rs` holds the clock and the threads.

| what | goes when |
| --- | --- |
| a clone's build output (`target`, `.next`, `__pycache__`, …) | its workspace idle 2 h |
| a clone's dependencies (`node_modules`, `.venv`, …) | idle a week — `harness doctor` reinstalls them |
| a run's folder under `logs/`, its `flow-*.jsonl` with it | 30 days old |
| `init/`, the throwaway clones of `init-repo` | idle a day |
| a stray, a folder the harness never writes | only when the human allows it, idle a day |
| a clone's code, `.git`, the ledgers, the event store, the locks | never |

A name that a project might commit (`dist`, `build`) is not on the list: a
sweep never takes what a reset would not give back. A folder under the
workspaces without `.git` is left alone, the provisioning rule. The walk
follows no symlink and a removal unlinks one; every path stays inside the
yard.

The pane shows two pies — what the yard weighs against its limit (50 GB at
first), and what fills it by category — then the settings, what a sweep
would remove and why, and what stays. Two buttons: **Diagnose** weighs again
and removes nothing; **Run the sweep** (two clicks) weighs afresh, removes,
and weighs again. The yard is weighed when the view starts and every two
hours; with auto-sweep on, a weighing past the threshold (a share of the
limit) sweeps on its own. One job at a time. Settings are kept in
`.llocal/janitor.json`, which no run reads. Routes `GET /api/janitor`,
`POST /api/janitor/{diagnose,sweep,settings}`; `--no-janitor` leaves the
yard alone.

## The data model, and the board it is drawn on

The `DATA` cylinder in the architecture office opens the product's data
model as a **UML class diagram**: one box per table, folded to its name — a
click unfolds its fields — one curve per link, and an inspector beside it
that tells a table or a field in full: type, purpose, links in and out, the
technical rules the database holds (keys, `NOT NULL`, defaults, checks,
unique indexes, enum values), the business rules the agents wrote.

Two sides, each a file in the **product's** repository:

| file | side | what it says |
| --- | --- | --- |
| `schema.sql` | the database | `pg_dump --schema-only`: the structure the migrations build |
| `model.json` | the agents | what each table and field is *for*, its business rules, links no key carries |

**Versions.** After every merge that moves `main`, `main_agent` or a
milestone branch, the watch runs `data_snapshot` (`harness-workflows`): the
head is checked out, the target's `scripts/dump-schema.sh` (or the harness's
default) builds a scratch database from its migrations and dumps it, and the
dump and `data/model.json` of that commit are archived on the target's
orphan branch **`harness/data-model`** — `index.json` at its root, one
folder per version that changed something (`<branch>/<stamp>-<sha7>/`).
The pane offers `main`, `main_agent` and `feature ▾` (the milestone
branches), then the versions of the branch shown, newest first. Without an
archive yet, it reads the live `data/schema.sql` and `data/model.json` at
the integration branch.

Everything is read through core's `GitHub::file_text` (`ports::Product`,
`adapters::product`), or from a folder with `--data-dir <product root>`, by
**the data desk** (`data_desk.rs`): one local task that owns the `?Send`
port, answers the handlers through a channel, keeps every archived version
once read (a version never changes), and reads the index and the live files
again after a while. Routes `GET /api/data-model?branch=&dir=` and
`GET /api/data-model/versions`.

`harness_workflows::data_snapshot::data::schema_sql` reads the DDL (a tokenizer and five statement shapes,
never a panic: what it does not understand it skips), `…::data::model`
merges the two field by field and lists every **divergence** — a table, a
field or a key only one side has, a type or a nullability they disagree on.
Either file alone draws the diagram. `model.json`:

```json
{ "version": 1,
  "tables": [ { "name": "character", "purpose": "…", "business": ["…"],
      "fields": [ { "name": "campaign_id", "type": "uuid", "nullable": false,
                    "purpose": "…", "technical": ["…"], "business": ["…"],
                    "references": "campaign.id" } ] } ],
  "links": [ { "from": "item.code", "to": "rule_reference.code", "purpose": "…" } ] }
```

`--demo` without `--data-dir` reads the sample under `crates/view/sample/`,
its archive under `sample/archive/` laid out as the branch is.

**Divergences.** When a version of `main` or the integration branch is
archived and its two sides disagree, the watch sends Opus to say why
(`data_diagnosis`): which side is wrong for each divergence, from the
history, and what to do. Its answer is an event (`DataModelDiverged`), so an
**error** notification; opening it shows the `data_divergence` pane — the
state of affairs, the divergences, the proposed actions — and a field where
the human writes how to resolve it. **Open the issue** (`POST
/api/data-model/issue`) composes the issue (`domain::divergence`: the human's
words first, the diagnosis after) and opens it through the data desk with
`harness:roadmap` + `harness:ready`: the planner, the split and the loop
take it from there. A later version whose sides agree (`DataModelAgreed`)
clears the alert. Route `GET /api/data-model/divergence?branch=&dir=` reads
the diagnosis back from the event store.

The diagram is drawn on **the board** (`static/board.js`), a component any
pane can mount: a canvas to pan (drag) and zoom (wheel, the toolbar), and a
**pin** the human drags off the toolbar onto any part marked
`data-pin="<target>"` — a table, a field, the bare canvas. A pin follows its
part as the diagram unfolds and re-lays; a field's pin stands on its folded
table; a part gone leaves its pin grey where it was.

A pin is one of three kinds (`domain::annotations::Kind`):

| kind | colour | what it is |
| --- | --- | --- |
| note | yellow | the human's words |
| brainstorm | green | an **AI session** about the part, opened from the pin |
| asking | orange | a change to consider when the next roadmap is built — its session kept too; how it reaches the planner is parked (`docs/PARKED.md`) |

A pin's session is an interactive Claude Code (the steward's model, plan
mode, in the product's snapshot clone when there is one) in the
`note_session` pane, run by `note_desks.rs`. It opens with a fixed
`--session-id` and the pin's **conversation handed back** in its system
prompt (`domain::note_session`): whole up to 40k tokens, else a kept summary
of the older turns plus the last ~10k word for word — the summary is made
again only once the conversation grew 20k more, so a handback stays
identical for the prompt cache. Leaving the pane closes the session: its
turns are read from Claude Code's transcript (`~/.claude/projects/…/<id>.jsonl`,
`adapters::claude_cli::ClaudeTranscripts`) and appended to the conversation —
always kept whole. Turning a brainstorm into a note has Claude (`claude -p`,
`ClaudeOneshot`) write its **conclusion** into the note; into an asking, the
**request**; the conversation stays with the pin.

Pins and conversations live in `.llocal/harness.db`, table `notes`
(`ports::NoteStore`, `adapters::sqlite_notes`; the old
`annotations.json` is imported once). A deleted pin is only marked: ctrl+z
(or the toolbar's undo) brings back the last one, ten per board; the
eleventh is destroyed with its conversation. Routes: `GET
/api/notes/{board}`, `PUT`/`DELETE /api/notes/{board}/{id}`, `POST
…/undo`, `POST …/{id}/session` (open), `…/session/term` (WebSocket), `POST
…/session/close`, `POST …/{id}/into/{note|asking}`.

The briefing names exact commands — `pgrep`, `setsid nohup ./target/release/harness watch …`,
`pkill`, `gh issue edit … --add-label` — because the two mistakes a steward
must not make are starting a second watch and stopping one mid-session
without saying so. `--permission-mode` comes from the same `PERMISSION_MODE`
the launcher passes to every session. The page is bound to `127.0.0.1` for
a reason: this terminal is a shell in the checkout.

## The modules

```
src/main.rs            the wiring: two pollers, one server, one desk, one switch, one thread; the event store and harness-notify's feed, once per poll
src/cli.rs             what a human types; TARGET_REPO_URL, INTEGRATION_BRANCH, PERMISSION_MODE shared with the launcher; HARNESS_WATCH_COMMAND for the switch
src/note_desks.rs      the pins' sessions at work: one Claude Code per open pin, its turns read back when it closes
src/data_desk.rs       the data desk: one task owning the Product port, versions cached, the handlers answered by channel
src/janitor.rs         the janitor at work: weigh, sweep, the two-hourly round, one job at a time
src/desk.rs            a desk: one terminal kept alive between visits, scrollback, broadcast — the steward has one, the doctor another
src/domain/            the inside — no disk, no subprocess, no clock
  blueprint.rs         the rooms, the models, one line per workflow, station by station; the gates each stage really declares, two in a row shown as one arch
  gates.rs             the gate verdicts out of the event store: which gate said what in which run, the checks of its last pass, the state an arch takes
  traces.rs            parsers: watch.log, run.log, prompts.md headers, the stream's first event, the ledger sums
  journal.rs           the watch loop out of watch.log: what it triggered and how each ended, empty and failed ticks
  history.rs           the control room over a chosen period: ledger, stops and journal recomputed between two trace clocks
  limits.rs            the rate limits read on demand: GitHub's buckets, and when a Claude probe is worth paying again
  observe.rs           what one tick reads through the Traces port
  assemble.rs          Observed + board → Snapshot (who is live, where the product is, what smokes)
  snapshot.rs          the serializable picture the page receives
  steward.rs           the steward's standing orders, and the status the page asks for
  doctor.rs            the doctor's standing orders, the check-up asked when they sit down, the question about one run and how its answer is heard
  plant.rs             the switch: which watch is this checkout's, what start / soft / hard send to whom
  cleanup.rs           the janitor's rules: what in the yard is no longer useful, the pie, the limit and the threshold
  data_versions.rs     the archive's versions in the page's order: main, the integration branch, the features
  divergence.rs        a diagnosed divergence found in the events, and the issue a human opens from it
  annotations.rs       the human's pins: their kinds, the checks before one is kept, what ctrl+z can still bring back
  note_session.rs      a pin's AI session: what it is told, what is kept of it, what is handed back (40k whole, else summary + 10k)
  prompts.rs           a station's prompts for the pane: the catalog, the book and the drafts together; an edit, a revert, a withdrawal; what waits
src/ports/mod.rs       Traces (the disk), Board (GitHub), TerminalFactory/TerminalIo (a desk's program), Plant (the watch process), Limits (Claude's windows and GitHub's buckets, read now), Yard (.llocal, weighed and swept), Product (the product's files), NoteStore (the pins), Oneshot / Transcripts / TerminalMaker (a pin's session), Prompts (the book read only, the drafts), Trials (the dry-runs)
src/adapters/
  fs_traces.rs         .llocal/logs, read through core's own ledger readers
  gh_board.rs          core's GitHub port, read the way the router reads it
  pty.rs               a pseudo-terminal running `claude`
  watch_proc.rs        `ps`/`lsof` to find the watch, a detached spawn to start it, `kill` to stop it
  fs_yard.rs           .llocal walked and swept with std::fs, symlinks never followed, every path kept inside
  product.rs           the product's files: on GitHub at the integration branch, or in a folder
  sqlite_notes.rs      the pins and their conversations, in .llocal/harness.db
  claude_cli.rs        claude -p asked once (summary, conclusion, request), and Claude Code's transcripts read back
  prompt_files.rs      the prompt book, read only, and the drafts, read and written — through harness-adapters' stores
  trial_proc.rs        `harness trial` spawned with the text on stdin; its reports read back, a process dead before reporting told as failed
  limits_cli.rs        a minimal `claude -p` (haiku, no tool) for the subscription windows, `gh api rate_limit` for GitHub
src/server.rs          axum: the page, the scripts, /render/… (the wasm bundle, from disk), /api/snapshot, /api/events (SSE), /api/runs/… (tails, stages, graph), /api/issues/…, /api/history?from&to, /api/limits, /api/plant, POST /api/plant/{start,soft,hard}, /api/steward, /api/steward/term (WebSocket), /api/doctor, /api/doctor/term (WebSocket), /api/doctor/diagnoses, POST /api/doctor/diagnose/{workflow}/{run}, /api/janitor, POST /api/janitor/{diagnose,sweep,settings}, /api/data-model, /api/data-model/versions, /api/data-model/divergence, POST /api/data-model/issue, /api/notes/{board}, /api/prompts/{line}/{station}, POST /api/prompts/{edit,restore,withdraw,trial}, /api/prompts/trials, /api/pending, POST /api/restart
static/                index.html, style.css, app.js (data, navigation, panes, the three notification signs and their pane, the bridge to the renderer), board.js (a pannable, zoomable canvas with pins), data-model.js (the UML diagram on it), vendor/ (xterm.js), render/ (built, not committed)
sample/                a product's data model to show under --demo: data/ (live files), archive/ (versions)
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

- **Read-only, from the files — with one switch.** (The pins are the
  human's own notes, in a file no run reads.) The view reads what
  `harness watch` and the runs already write, and never asks the harness to
  emit anything for it. A structured event port in `harness-core` would be
  more precise; it can be added later without touching this crate's inside,
  because `observe` already reads through a port. It writes nothing a run
  reads: a prompt edited here is a draft the watch applies itself when it
  starts. Its hands on the plant are
  four, all the human's: the steward's
  terminal, the status button's switch (`Plant` port), the janitor's
  broom (`Yard` port), which removes only caches and old traces no run
  needs, on a click or under the threshold the human set, and the data
  pane's "Open the issue" (`Product::open_issue`), one issue per click. The switch does
  three things only — start the watch (`HARNESS_WATCH_COMMAND`, detached in
  a process group of its own, output to `watch.out`), soft stop (`SIGTERM`
  to the watch, which drains), hard stop (`SIGKILL` to the watch's tree, and
  to its group when that group holds nothing but the plant). It only ever
  acts on a watch whose working directory is this checkout.
- **One picture, pushed.** The pollers assemble a `Snapshot`; the server
  streams it on `/api/events` whenever it changed (compared before it is
  stamped, so a clock alone wakes nobody). The page asks for nothing else
  except a log tail and an issue body, on demand and bounded.
- **The blueprint is declared, not derived.** `harness-workflows`' stage tables
  are built against ports and boxed actions; what a stage *looks like* is a
  fact of the view. The stage names in `blueprint.rs` are the ones the runs
  log, and `assemble`'s tests pin a real `run.log` shape against them. The
  gate names are the `Gate::name`s the workflows build (`code requires`),
  and only the gates a stage really declares are drawn; a stage's `post`
  and the next stage's `pre` stand as one arch, named by its neighbours
  (`technical refinement → code`), which lists its gates when clicked.
- **A gate's state comes from the event store, not from the log.** Every
  verdict a gate gives is a `gate_checked` event under the run's source;
  `gates.rs` reads the newest ones each tick and gives each arch its state —
  passed, skipped, halted — its last pass's checks with their verdicts, and
  the runs it spoke in. A run's own graph (`/api/runs/{workflow}/{run}/graph`)
  is the same reading for one run: a circle per gate, a square per stage,
  green ✓, amber !, red ✕ — what the agent's pane draws over its logs.

## Running it

```
cargo run -p harness-view -- --demo          # the latest run shown live
cargo run -p harness-view -- --no-board      # never call gh
cargo run -p harness-view -- --no-steward    # no terminal, nobody to talk to
cargo run -p harness-view -- --no-doctor     # the infirmary has nobody in
cargo run -p harness-view -- --no-janitor    # the yard is never weighed nor swept
cargo run -p harness-view -- --static-dir crates/view/static   # edit the front without a rebuild
cargo run -p harness-view -- --data-dir ../my-product           # the data model from a local checkout
```

`TARGET_REPO_URL`, `INTEGRATION_BRANCH` and `PERMISSION_MODE` come from the
same `.env.local` the launcher reads. The page listens on `127.0.0.1:7878`
(`HARNESS_VIEW_PORT`).

## What is deliberately not here yet

Room 2's own conversation with Claude Code about an issue (the steward's
terminal is the plant-wide one), its mock-up, room 6 —
each shows its place and says so. Level D is wired for the lines (station,
employee, line), the sign, the chimneys, the issues, the store, the control
room, the steward and the doctor; the other components open a placeholder that names
what will come.
