# The doctor's two attendants — watcher and fixer (not wired yet)

Status: **draft, loaded by no session.** These files describe the role the
factory's human supervisor has played by hand (a Claude Code session
watching ForgeDesMondes and evolving the harness), so the view's infirmary
(room 4, `crates/view/src/domain/doctor.rs`) can one day carry it. Nothing
reads them today; wiring them is its own lot (the fixer's, view crate).

## The split

| | Watcher | Fixer |
|---|---|---|
| Model | Sonnet, effort high | Opus, effort high |
| Runs | continuously while its switch is on | on demand: an incident the watcher hands over, or the human |
| Does | watches, diagnoses, applies the playbooks on the board (labels, a Decision rewrite, an issue in the harness's shape), restarts the watch only where a playbook names it and never during a harness development plan, keeps the records, reports | changes the harness or the product's installed assets: design → critique → implement → 2 reviews → fix, gates, docs; the (fixer) actions of the playbooks; the one restart when the whole plan passes |
| Never | edits code, builds, pushes, merges; any action that does is handed over | pushes the harness without the owner's word, works on the factory checkout by hand |
| Brief | [`watcher.CLAUDE.md`](watcher.CLAUDE.md) | [`fixer.CLAUDE.md`](fixer.CLAUDE.md) |

Both import the shared references: [`rulings.md`](rulings.md) (what the
owner already decided), [`places.md`](places.md) (every folder, log,
record, label, marker, clock) and [`playbooks.md`](playbooks.md)
(instruments, symptom → action → harness fix, traps, reference numbers).

## Design change this implies

The view's invariant today: every way it acts on the world is the human's
gesture, never a run's input — the steward and the doctor act only on what
the human types (`crates/view/ARCHITECTURE.md`, *The steward, and the
doctor*: "Nothing in the view does any of that itself"). A watcher that
relabels issues, rewrites `## Decision` and restarts the watch on its own
breaks it. It stays defensible only as **one explicit human gesture that
delegates**:

- **A switch the human flips**, per attendant, on the infirmary's wall —
  persisted (`.llocal/attendants.json`, the janitor's `janitor.json`
  pattern in `crates/view/src/adapters/fs_yard.rs`), off by default, never turned on by a
  run, a flag alone or the attendant itself.
- **Journaled**: each flip, and each write the watcher makes, is an event
  (`Logbook::event`) and a `DECISIONS.md` line, so `harness-notify` can
  surface it and the owner can audit it.
- **A kill switch**: switching off ends the desk (`Desk::shutdown`); the
  plant keeps running. A hard-coded floor the switch cannot lift: the
  never-lists of the briefs.

Wiring amends `crates/view/ARCHITECTURE.md` (*What the view writes*, *The
steward, and the doctor*) and the view paragraph of the root `CLAUDE.md`.

## Coexistence with the existing doctor

- The doctor stays what the infirmary opens and what every agent's doctor
  button asks (`POST /api/doctor/diagnose/{workflow}/{run}`,
  `POST /api/doctor/ask`, `GET /api/doctor/diagnoses`): read first, treats
  only on the human's go. The watcher answers none of those.
- With the watcher on, it reads the doctor's diagnoses
  (`/api/doctor/diagnoses`) and folds them into its incident flow; it never
  types into the doctor's desk.
- Same rule as the doctor: `harness doctor` without `--dry-run` discards
  uncommitted files — the human's go only, for every desk.

## Activation — fitting the real `Desk`

Today a `desk::Desk` starts its program **on the first browser visit**, not
at server start, and after the program exits it comes back only on the next
keystroke ("press Enter to call them back"). Every desk opens in the view's
repo root (`Pty::new(root, …)`) — here `<factory>`, since `start_view.sh`
runs the view there. So a watcher "running continuously" needs:

- **Server-side start**: when its switch is on, the server summons the
  watcher's desk at start and summons it again when its program ends —
  the gated exception to "nothing starts by itself".
- **Flags in the doctor's pattern** (`--no-doctor`, `--doctor-model` /
  `DOCTOR_MODEL`, `--doctor-effort` / `DOCTOR_EFFORT`): `--no-watcher`,
  `--no-fixer` (the desks exist but stay off until switched on);
  `--watcher-model` (`WATCHER_MODEL`, default `claude-sonnet-5-5`),
  `--watcher-effort` (`WATCHER_EFFORT`, `high`), `--fixer-model`
  (`FIXER_MODEL`, `claude-opus-5-5`), `--fixer-effort` (`FIXER_EFFORT`,
  `high`); the env names added to the list `crates/view/src/cli.rs`'s tests
  assert.
- **Each desk its own cwd**: `--records-dir <records>` (the watcher's),
  `--fixer-dir <harness>` (the fixer's; required to switch it on) — never
  the view's root, which the fixer's brief forbids working in.
- **Code** (view crate): `watcher_for` / `fixer_for` beside `doctor_for` in
  `main.rs`, each a `Desk` over `ports::TerminalFactory`; `AppState`
  fields; routes `/api/watcher/term`, `/api/fixer/term`, `/api/fixer/ask`;
  panes and the wall switches on the page.
- **Permissions**: every desk inherits `--permission-mode`
  (`PERMISSION_MODE=bypassPermissions` here). The watcher's desk adds
  `--disallowedTools Edit Write NotebookEdit`; it still has Bash, so "never
  edits code" stays mostly a prompt rule — say so, do not claim more.

## Brief delivery — files, not argv

- **Not through argv.** The view briefs its desks with
  `--append-system-prompt <text>` today, and `pgrep -f` sees argv. The run
  tools match on strings these briefs contain (`disk-cleaner.sh`:
  `cargo-mutants|cargo mutants`; `night_metrics.py`: `harness.*watch`;
  `restart_watch.sh`: `bin/harness watch --force-reset`); the steward's
  claude already makes `harness.*watch` match with the watch stopped. The
  view writes each built brief to a file and passes
  `--append-system-prompt-file <file>` (the flag exists in `claude --help`;
  check interactive mode honours it; fallback: the desk folder's own
  `CLAUDE.md`).
- **`@imports` are not expanded there.** `@rulings.md` / `@places.md` /
  `@playbooks.md` resolve only in a memory file (a `CLAUDE.md` in the cwd
  ancestry, or one it imports); appended prompt text is passed literally.
  So the brief builder (`crates/view/src/domain/attendants.rs`, new,
  `include_str!` of the five files) **inlines each `@file` line with that
  file's content** and fills the placeholders, at build time. Installing a
  brief as `CLAUDE.md` with its siblings beside it would also resolve them,
  but would make that folder the desk's cwd.

## What is factory-specific (parameterize before reuse)

- Placeholders filled at brief-build time, like `doctor::briefing(project)`:
  `<factory>`, `<harness>`, `<product>`, `<records>`.
- `<records>` is today the owner's run folder `harness-dnd2-run` — an
  ad-hoc human convention, not a harness concept. Wired, it becomes a
  configurable **records folder** (`--records-dir`, default
  `<factory>/.llocal/attendants/`) holding `DECISIONS.md`,
  `night/incidents/`, `night/priorities-H.md`, reports.
- The instruments in `<records>/tools/` hard-code ForgeDesMondes paths
  (`restart_watch.sh`, `start_view.sh`, `follow_quiet.py`,
  `disk-cleaner.sh`, …): each is parameterized or becomes a harness
  command before another factory uses the attendants.

## Hand-over (watcher → fixer)

The incident file and the priorities line are in the watcher's brief
(step 5), the restart checklist in the fixer's (*The plan and its one
restart*). Wiring adds one call: once the incident is written and the
fixer is on, the watcher asks it through `POST /api/fixer/ask
{"incident": "<path>"}` (modelled on `/api/doctor/ask`: summon, warm-up,
bracketed paste "Read <path> and treat it", Enter). That call is itself a
non-human write: it exists only while both switches are on.
