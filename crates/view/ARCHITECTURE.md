# `harness-view` — architecture

The factory view: a local web server that draws the harness as an isometric
plant, read from the traces the harness leaves behind. A **second outer ring**
beside `harness-launcher` — the only other crate allowed to build a concrete
adapter — and it **writes nothing a run reads**, with one exception: a
prompt edited here is a draft the watch applies itself (*The robots'
prompts*). It does act on the world in six ways, every one a human's
gesture and none a run's input — the list is *What the view writes*. The
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
| **B** inside | six rooms: lines, office, store, infirmary, control, construction — the rooms of a **development factory**; another workflow family would have other rooms | the blueprint; the same picture |
| **C** a room | the lines station by station, the product on the belt, an employee beside each station in progress — several when a parallel watch runs the line on several lanes; a crew that shares a station fans out and opens a pane to pick one | `run.log`, `prompts.md`, `stream.jsonl` of every fresh run; `watch.log` |
| **D** a pane | logs live, a station's cost, how long a run, a machine or a trigger took — or has been running, ticking — an issue body, the dashboards, the versions | `/api/runs/…` tails; `/api/issues/…`; the picture |

An **employee** is a run believed to be at work: the watch dispatched its
workflow and has not reported back, or its folder was written to in the last
two minutes. The harness logs no "run over" line, so this is a judgement, and
it lives in one function (`assemble::is_live`) with its two thresholds.

## What the view writes

The rule is **nothing a run reads**, and the one exception is the prompt
draft (`.llocal/prompts.drafts.json`), which only a start of the watch
takes. Everything else the view writes is the human's own gesture, done
through a port so a test can refuse it:

| what | where it lands | port | the gesture |
| --- | --- | --- | --- |
| a prompt draft, its shelf, a trial's report | `.llocal/prompts.drafts.json`, `.llocal/prompts.shelf.json`, `.llocal/trials/` | `Prompts`, `Trials` | save in the editor; dry-run |
| the steward's and the doctor's keystrokes | an interactive `claude` in the checkout — a shell, which may do anything the human types | `TerminalFactory` | typing in the pane |
| the watch started, drained or killed | the `harness watch` process | `Plant` | the status button |
| caches and old traces removed from `.llocal/` | the yard, by static rules, never a clone, a ledger or the event store | `Yard` | the janitor's buttons, or the threshold the human set |
| a mock-up edited, then a `mockup/*` branch pushed | a `git worktree` of the mirror under `.llocal/view-mockups/`; the target's remote on save | `Mockups` | a link or simulate session; *save the mock-up* |
| an issue opened, a pin and its conversation kept | the target's board (`harness:roadmap`); `.llocal/harness.db`, table `notes` | `Product::open_issue`, `NoteStore` | *Open the issue*; a pin dropped, a brainstorm |
| the factory register, and the view started again in another factory | `~/.harness/factories.db` (`--harness-home`), outside every checkout; the view's own `exec` | `harness-core::ports::store::factories::Factories` (adapter in `harness-adapters`), `adapters::relaunch` | the header's factory menu |
| a new factory: its GitHub repository, its folder, `init-repo` run there, the record of its creation | GitHub (`gh repo create`); a detached `git worktree` of the harness under `~/.harness/factories/<id>`; its `.env.local`; the register's `creations` table | `Foundry` (`adapters::foundry`), `Factories::keep_creation` | *Create the factory*, *Connect* on the creation page |
| a factory taken down: its watch stopped, its GitHub repository deleted (destroy only), its folder removed (or, for a checkout the view did not make, its `.llocal/` and `.env.local`), its place in the register forgotten | `kill`; GitHub (`gh repo delete`); `git worktree remove` of `~/.harness/factories/<id>`; the register's `factories` and `creations` rows | `Plant`, `Teardown` (`adapters::foundry`), `Factories::forget` | *Delink* or *Destroy* in the janitor's pane (the factory this view is on), or on a row of the header's factory menu |
| what the view's own `claude -p` calls cost (the design system's, a factory's creation, a micro-frontend's, a pin's) | `.llocal/harness.db`, table `usage`, beside the event store | `Usage` (`adapters::sqlite_usage`), read by every `ClaudeOneshot` the composition root builds | every such call; never a dev-loop run, which keeps its own ledger |
| a port a session asked for, approved or refused; a port moved to another provider | the target's board: the approval task closed with a comment; on a refusal, the waiting task commented and labelled `harness:needs-decision` first; a `harness:roadmap` + `harness:ready` item for the change | `Product::close_issue`, `Product::to_decide`, `Product::open_issue` (through the `DataDesk`) | *Approve*, *Refuse*, *Change provider* in the Ports pane (the capabilities list's *⇄ ports*) |
| the proof of concept drawn, then published; its roadmap task; the watch started | the integration branch's mock-up workspace (a `claude -p` that writes, `Oneshot::work_in`), then the branch itself (or `mockup/proof-of-concept`); the board; the `harness watch` process | `Oneshot`, `Mockups`, `KitDesk::publish`, `DataDesk::open_issue`, `Plant` | *Draw the mock-ups*, *Accept* |

A pin's session and a mock-up session are Claude Code sessions that edit
the product's checkout — in a worktree the view made, never the harness's
lanes — in the launcher's permission mode. Nothing they do is committed by
the view; a human saves or discards. The `asking` pins have no reader yet
(`docs/PARKED.md`).

## Several factories

A **factory** is a folder with its own `.llocal/` and its own target
repository. The register (`Factories`, SQLite at `~/.harness/factories.db`,
or `--harness-home`) lists the ones this machine knows; the view enrolls the
folder it starts in, by root, so the factory it was always run in is there
from the first start (none under `--demo`).

**One view serves one factory; a switch starts it again in another.** The
header's factory name opens a menu (`static/factories.js`): a search field,
a list showing five factories at a time, and *+ new factory* below it.
Choosing a factory is `POST /api/factories/{id}/open`: the handler puts the
factory in `AppState::switching` and answers `202`; `main` stops serving
(the page's open streams get 1.5 s, they never end on their own), shuts the
desks down and `exec`s the same binary in the factory's folder with its
repository and branch (`domain::switch::args_for`, `adapters::relaunch`).
Same pid, same port, so the page's event stream comes back with a new
`build` and the page reloads itself. The watches the view started live in
process groups of their own and keep running; a view finds the watch of its
factory by working directory (`WatchProcess::works_here`). The cost is a few
seconds and the desks' sessions, which close.

A switch also loads the target factory's `.env.local` into the new image's
environment (`domain::switch::env_of`: set values only, never
`HARNESS_VIEW_PORT` — the page waits on the port it talks to), so a
factory's own watch command and harness binary win over the one it leaves.

### Taking a factory down

Two gestures, in the **janitor's pane** (*This factory*: the factory the view
is on) and on each row of the header's menu (any factory), `removal.rs`:

| | the watch | the repository | the folder | the register |
| --- | --- | --- | --- | --- |
| **delink** (`POST /api/factories/<id>/delink`) | asked to stop, soft: the running tasks finish, none starts | stays on GitHub; connect it again by URL | removed, or emptied of `.llocal/` and `.env.local` | forgotten |
| **destroy** (`POST /api/factories/<id>/destroy`) | killed | **deleted** (`gh repo delete --yes`) | as a delink | forgotten |

Both bodies carry `{"confirm": "<owner/name>"}`: the human types the
repository's name, as GitHub asks (the factory's own name when it has no
repository). The button stays shut until it matches, and the server compares
it again — a request without the exact name changes nothing (`400`).

Rules that keep it from taking more than it should:

- **The right to delete is asked first** (destroy, `Teardown::delete_right`:
  the `X-OAuth-Scopes` header of `gh api -i user`, read-only). A token that
  lists its scopes without `delete_repo` stops everything before the watch is
  touched (`403`): the refusal carries the two commands to run by hand
  (`gh auth refresh -h github.com -s delete_repo`, then
  `gh repo delete <slug> --yes`). A token that lists nothing (fine-grained)
  is tried, and a late refusal says how to grant the scope.
- **Only a folder the view made goes whole**: a direct child of
  `~/.harness/factories/` (`Removal::is_ours`, which the list answers as
  `owned`). A factory enrolled from a checkout somebody keeps — the harness
  itself, say — loses its register entry, its watch, its harness data
  (`.llocal/` alone) and (destroy) its repository, never its checkout and
  never its `.env.local`: those settings belong to the checkout, not to the
  factory, and a `watch` or a view of someone else's may be reading them.
- **The factory this view is on can be taken down**, and its folder is the
  view's working directory. The view moves to another factory (the first
  other one whose folder exists — `409` when there is none, nothing touched)
  through the switch (`202`, `"reopening": "<id>"`); the folder left travels to
  the new process in `HARNESS_VIEW_LEAVING` (`remove:<path>` or
  `wipe:<path>`, `adapters::relaunch`), which deals with it before serving
  (`Removal::finish`) and never touches its own.
- **The register is forgotten last.** A step that fails (a folder that will
  not go, a repository GitHub refuses) leaves the factory listed, so the
  gesture can be repeated; a repository GitHub says is gone already does not
  block the rest.

The watch stopped is the one working in *that* factory's folder, found the
way the status button finds its own (`WatchProcess` on that `Workspace`).
What the Docker services of a product hold (`docs/PERSISTENCE.md`) is not
touched.

### The factory-creation page

A factory with no GitHub repository (`project.slug` empty), one whose
creation is under way, or *+ new factory*, shows the **factory-creation
page** (`#new-factory`, `static/creation.js`): a page, not a pane — the
scene is hidden, paused and, when the page opens that way, never loaded.
It has two sections:

- **Connect an existing repository** — a URL; the view makes the factory's
  folder, runs `init-repo` there, enrolls it, and opens its plant.
- **Initialize a factory** — ten steps under a breadcrumb
  (`#new-factory/<step>`), each gated (`domain::creation`, `Creation::may_enter`:
  a step opens once every step before it has what it needs). Each crumb
  wears its step's mark (`Creation::marks`, served as `marks`): green done,
  yellow stale — made from something upstream that moved, carried down the
  scope → mock-ups → rules → data chain — red blocked — a gate before where
  the human went (`Creation::visited`) fails — grey locked:

  1. **repository** — name, owner (`gh api user`, `user/orgs`), visibility:
     `gh repo create --add-readme`, the folder, `harness init-repo`, the
     register, a creation record; then the view switches to the new factory,
     where steps 2 to 10 run — a factory like any, so the easels serve it.
     *Start* on the landing goes to `#new-factory/found`, the same form read
     as a factory with nothing done yet: the record on the desk is the
     factory open here, never the one being founded, so a factory whose own
     creation is half-done can found another one — its steps stay locked,
     its record untouched, and the view moves on once `init-repo` is done.
     `#new-factory/repository` stays the record's own step (the repository
     created, *Continue*);
  2. **kind** — *vertical architecture development factory*, the one there
     is (*hexagonal* shown, not selectable);
  3. **brief** — what the application does, first told at large
     (`Creation::description`), then split by the AI into four parts
     (`BRIEF_PARTS`: the problem, the users, their journeys, the data and
     integrations; `POST /api/creation/decompose`, `CreationDesk::decompose`)
     the human rereads and retouches — or fills alone (*Fill them myself*).
     The split is a button, or happens on its own once the description has
     been left for another part of the page for 10 s, before the first split
     only and never while the page lost the focus to another tab or window.
     The record keeps one text, a `###` heading per answered part. Then,
     under them the design system's easel on its **material** face
     (`material: true`): the sources the design system is generated from —
     inject a tokens file, or the logo, the moodboard and the design brief
     — and the kit's catalogue, the components ticked. Not its sheet, not
     the simulated page, not the canvas, and nothing published from here:
     what the material draws is read and validated at step 6, which mounts
     the easel whole. *Continue* saves and starts
     the **preparation** (`POST /api/creation/prepare`, `Work::Prepare`):
     the tokens drawn from that material when there is some and none yet
     (`KitDesk::generate`, the ticked kit components queued behind them),
     while the human answers the grilling, followed on the crumbs
     (*generating*, *to validate*). No scope is drawn here: it waits for
     the grilling's decisions (step 5);
  4. **grilling** — rounds of questions (`grill_prompt`: the grilling skill's
     frontier, each question with its recommended answer, JSON), answered,
     retouched or deleted. Each question carries a `topic`, the part of the
     product it decides (empty for the product as a whole): the page shows
     one card per topic, *General* first. Still one call per round — the
     prompt lists the topics in use, asks to reuse them (a new one only for
     a part no topic covers), and forbids one decision under two topics.
     The last round's open questions are yellow, an answered one greys as
     soon as its field has text; a new round scrolls to the first open
     question, and each topic's header stays in sight while its questions
     scroll; `add_round` folds a topic differing only by
     case onto the one already given, and the roadmap issue's decisions
     follow the same topics; *continue grilling* asks the next round once
     every open one is answered; *use the N recommendations* fills the open
     ones; *Continue* with open ones offers the same. The first round asks
     itself; the scope opens once every question is answered, or when the
     AI had nothing to ask or every question was deleted (`grilled`);
  5. **scope** — the proof of concept as the AI proposes it, in fixed
     sections (`SCOPE_SECTIONS`: Goal, Users & roles, Screens, Journeys,
     Data, Integrations, Left out, Proof, and Notes, the free one, whose
     sub-sections are `### ` headings): the AI answers one JSON key per
     section (`scope_prompt`), the record keeps one Markdown text, each
     section under its `## ` heading, the empty ones left out
     (`parse_scope`). The page shows a field per section with its `?` (what
     it is for and which step reads it) and splits the text back
     (`splitScope`: text before any heading and a heading no section knows
     go to Notes). **Screens** is required — the drawing reads it
     (`drawing::screens`): the AI's scope refuses none, and the design
     step's gate stays shut without one. First proposed on entering the
     step, once the grilling is over
     (`propose_scope`), edited freely, proposed again on demand. The record keeps what it was
     proposed from (`scope_from`, `Creation::fingerprint`: the kind, the
     brief, the decisions): decisions moved since make it **stale** (*to
     redo* on the crumb) — proposed again by itself when the human did not
     retouch it (`scope_edited`), on their *Propose again* otherwise.
     The scope settled (the record past it), the proof of concept is
     **drawn ahead** while the human settles the design system
     (`POST /api/creation/ahead`, `draw_ahead`, started by the page once no
     job runs): one job draws the mock-ups (`Work::Mockups`), then makes
     them of micro-frontends and reads the requirements
     (`Work::Componentize`), leaving the human's step where it is — the
     mock-up step and the review find them ready (*drawing*, *ready* on
     the mock-ups crumb, *micro-frontends* on the review's). The tokens follow
     as variables; the kit's components are those generated by then. The
     record keeps the scope it was drawn from (`mocked_scope`): a scope
     moved since makes it **stale** (*to redraw*), redrawn on the human's
     *Draw again* only — never by itself, it would wipe their rework;
  6. **design system** — the design system's easel mounted in the step itself,
     never in a pane over the page (moodboard, logo, tokens, kit) — already
     generated by the preparation when the brief gave it material; generating or injecting tokens already
     queues the ticked kit components' generation by itself, the first
     candidate included (`design_generate`, `design_tokens`,
     `design_validate` — the same routes whichever pane opens the easel
     from, the standalone one too), and the step closes when none waits
     (`Facts::kit_waiting`) — the mock-ups are drawn on the kit, not on
     tokens alone;
  7. **mock-ups** — one Claude Code session per screen of the scope
     (`drawing::screens`: the bullets under `## Screens`), of the drawing's
     model (`--creation-draw-model`, Opus by default; the cut into
     micro-frontends uses it too), each at work in the integration
     branch's mock-up workspace (`Oneshot::work_in`: read and write, nothing
     that runs) on its own page `pages/<slug>.html` and its fixtures, on the
     kit and the tokens (`reference_prompt`, `screen_prompt`, the mock-up's
     own rules first) — fast, its markup inline: the proposal loop is what
     is optimised here. The first screen is drawn **alone**, the
     **reference** whose look every other follows; the others are then drawn
     `--creation-sessions` (`CREATION_SESSIONS`, 4) at a time
     (`in_parallel`), each reading the reference page first. Only the desk
     writes `mock/pages.json` and `mock/index.html` (`with_pages`,
     `index_html`, `list_pages`), once the sessions returned — no concurrent
     write — and a page no session wrote fails the step, named. A page of an
     earlier drawing no screen asks for any more is removed. The design
     system's **interface examples** of the whole app and of its pages are
     lent beside the pages first (`examples_lent`: copied under
     `design/examples/`, taken back with the rest by `kept_on_design`, never
     reported), named in the prompt (`examples_part`), and each line the
     session ends on — `example <file>: <what it took>` — is a step of the
     job; the cut into micro-frontends reads them the same way. While they
     run, a concurrent poller (`poll_pages`, every `DRAW_POLL`, 2s) reports
     each page once its file lands — the manifest being the desk's, written
     last — naming it and the count out of the screens the scope lists. The page's
     left column is the growing checklist; the right shows the mock-up
     itself, navigable in its current state, through the same
     `/mock/{branch}/{*path}` route the review step's easel already reads
     pages through — reloaded at its own current path, not forced back to
     the index, whenever a new step lands. The preview has a viewport
     picker (`viewport.js`, shared with the mock-up's easel: a phone, a
     tablet, a laptop, a custom width, or fit), a handle on its right edge
     and a full-screen switch (Esc leaves). **Drawn, the mock-up's easel
     takes the step** (`embedFor`): the human reworks it there — the
     designer's hands (*edit*), request pins applied to the draft or
     discussed, pages added — before the review; *draw again* asks first
     (the pages go, the pins stay), and moving on **flushes** the draft's
     steps into the workspace (`MockupPane.flush`, `materialize`) so the
     server reads what the human saw. The easel's micro-frontends list is
     hidden here (`strip: false`); elsewhere it reads in two sections — on
     this page, the rest (each saying where it is used) — and pointing at
     an entry lights its parts on the page, and the reverse;
  8. **rules** (once the review; `Step::Rules`, serde alias `review`) — first the accepted mock-up made again of
     **micro-frontends** (`POST /api/creation/componentize`,
     `Work::Componentize`, started by the step itself; `componentize_prompt`:
     a fragment, a fixture and a `pages.json` entry per Micro-UI, the pages
     left as layout and `data-component`), with the live poller watching
     for each micro-frontend (`poll_componentize`, seeded with whatever the
     branch already had — this call has no `discard()`, components persist
     across a retry, so one already there is never reported as just made),
     and the cut's gates run on what the session left (`gated`,
     `cut_gaps`): a page that injects none or one unlisted or without a
     fragment (`not_componentized`), a fragment naming no part for one of
     its needs or actions (`audit::unnamed_parts`, contract D) — the same
     the validation refuses. What they refuse goes back to the session
     (`cut_repair_prompt`), `CUT_REPAIRS` times at most, before the job
     stops on what is left; a guard's fix applied goes through them too,
     and a micro-frontend generated alone is asked again
     (`mf_gen::retag_prompt`, `RETAGS`) while a part is unnamed. Then the **rules**
     (`Requirement` in the code): read by the AI off the scope and the
     mock-up (`requirements_prompt`, `parse_requirements`,
     `POST /api/creation/requirements`), each a step of a `journey` or a
     `business` rule (`Family`) with its criterion (given…, when…,
     then…), tied to the pages and the micro-frontends that carry it —
     retouched, added, deleted, re-tied (`PUT /api/creation`,
     `Edit::requirements`; one the human touched is `human` and survives a
     new reading); one is read again alone (`rule_prompt`,
     `POST /api/creation/rules/{id}/reread`). They sit beside the mock-up's
     easel: a rule picked filters the easel's list to its micro-frontends,
     marks them on the page and shows its page (`MockupPane.focus`); a
     micro-frontend is generated again from there (`MockupPane.regenerate`).
     Validated (`Creation::validate_rules`, `POST /api/creation/rules/validate`)
     once each has its criterion and a support and no blocking finding is
     left — the fragments read as they are, so a guard's part-unnamed
     finding on a part named since is gone (`Audit::named_since`); any
     change takes it back; read off a cut that moved, they are
     stale (`rules_for`). An **Architecture** tab holds the guard
     (`domain::audit`, `POST /api/creation/audit`, run once by itself):
     names, systems' sizes and micro-frontends reaching into another
     system read off the cut; duplicates, writes passed off as reads and
     Capabilities leaning on another read by an AI, with every write's
     effect (parallel when an insert); a finding is applied by a session in
     the workspace or dismissed (`/api/creation/audit/{id}/{apply|dismiss}`),
     and only a write passed off as a read blocks. A *simple view* (a
     per-viewer preference, for a client who is no developer) hides the tab
     and says what blocks in plain words. Then **regenerate** (back to the
     brief, everything kept; the grilling keeps its questions, editable and
     deletable, and may ask more; the workspace starts again from the
     branch), or on to the data;
  9. **data** — the data model drafted by the AI off the rules, the cut and
     the mock-up's fixtures (`domain::data_draft`, `POST /api/creation/data`):
     tables (contract M) and aggregates (after contract I — criticality,
     invariants carrying the business rules, relation policies), with the
     fields each DataCapability writes; drawn in the data pane, which reads
     it from `GET /api/creation/data-model` (`source`). The structure
     changes through the AI (`/api/creation/data/change`); the human
     retouches purposes and picks criticalities and policies
     (`PUT /api/creation/data`, `DataEdit`). Four gates keep it from
     validation (`DataDraft::gaps`): a business rule carried by nothing, a
     foreign key with no policy, a write on a field the model has not, a
     field with no criticality; drafted from rules that moved, it is stale.
     Then **accept** (the volumes said first; waits for the rules and the
     data model validated);
  10. **initialize** — the **record** of the creation written in the
     repository beside the mock-up (`mock/proof-of-concept.json` and `.md`,
     `creation::record_json`, `RECORD_JSON`: the brief, the decisions with
     the questions asked, the scope, the rules, the pages with their
     objectives, the micro-frontends with their `needs` and `actions`, the
     pins no backlog issue carries, the design system, every write with its
     effect and the fields it touches, the aggregates — and the data model
     itself, `mock/data/model.json` and one
     `mock/data/aggregates/<Aggregate>.aggregate.json` per aggregate (contract
     I), for the task building each aggregate to install), then the mock-up
     published on the integration branch (else saved on
     `mockup/proof-of-concept`) and **where it went kept**
     (`Creation::mockup_at`), the design system published and carried
     (`design_at`, `design_refused` when the carry was refused — the page
     and the task say so), a `harness:roadmap` + `harness:ready` task
     carrying all of it (`roadmap_issue`, `Context`; opened once —
     initialized again, the task already open is kept), the watch started;
     *open the plant* ends the creation (`Factories::end_creation`) — the
     record lives on in the repository.

The record is the register's (`Factories::creation`, `keep_creation`): the
creation survives a reload, a switch and a restart. One long job at a time
(`creation_desk.rs`: found, connect, prepare, mock-ups, componentize,
initialize), followed by the page through the **pulse** — the grilling's
rounds and the scope's proposals are calls, not jobs, so they run beside
the preparation.

The page is not asked on a timer. `pulse.rs` keeps a counter the view beats
whenever what the page draws from may have moved: every write to the
product's workspace (`PulsingMockups`, wrapped around the `Mockups` port in
`main.rs`, so whoever writes — the design system's easel, the mock-up's, a
creation job — is heard without beating it themselves; reads never beat),
the creation desk's record kept and its job begun, stepped or ended, the
kit's generation jobs queued, taken and ended. `/api/events` streams it as
a `pulse` event, the current count first, so a page that reconnects reads
again. The page then reads `/api/creation` — one reading at a time, a
burst of beats read at most every 250 ms — and redraws only what the
reading changed: nothing when it is the same, the job's panel and the
preview when only the job moved. A redraw **morphs** the page into the new
markup (`vendor/morphdom.js`) rather than rewriting it: an element that did
not change stays the same node, and two are never touched — the field the
human is typing in (its text, its caret, its focus), and an element marked
`data-cr-keep` whose mark did not change (the easel mounted in the step,
by its face; the preview's frame, by its branch), whose live state is the
script's. Until a pulse is heard — a view built before it — the page asks
every two seconds instead while something moves. An acceptance another job refuses is not kept. The AI is `--creation-model` (`CREATION_MODEL`,
`claude-opus-5-5`). A factory's folder is a worktree of the harness the view
runs from, not an empty folder: the watch lends the harness's
`.claude/skills` from the checkout it runs in, and `init-repo`'s
`.env.local` gets the harness binary by absolute path
(`HARNESS_BIN`, `HARNESS_WATCH_COMMAND`, `HARNESS_TRIAL_COMMAND`). The
renderer bundle, never committed, is read from the view's own checkout when
the factory's has none.

Routes: `GET /api/factories` (`current`, `factories`),
`POST /api/factories/{id}/open`, `GET|PUT /api/creation`,
`GET /api/creation/owners`,
`POST /api/creation/{found,connect,grill,scope,prepare,mockups,componentize,requirements,regenerate,accept,finish}`.

### What the view's own AI calls cost

Every `ClaudeOneshot` the composition root builds (`creation`, `kit`, `mf`,
`notes`, `units` — one per asker, tagged at construction) runs
`--output-format stream-json --verbose` rather than plain text, reads the
turn's own `result` event for its `total_cost_usd`, `duration_ms` and token
counts, and keeps one row per call through the `Usage` port
(`adapters::sqlite_usage`, table `usage` in `.llocal/harness.db`, beside the
event store). This is never a dev-loop run's own ledger
(`costs.tsv`, read by the *Spending* screen already) — a question the view
asked on its own account to answer a human waiting on a page, not a task a
run billed. The *Spending* screen's own section, *the view's own AI calls*,
reads it summed by asker (`GET /api/usage`, `domain::usage::summarize`). A
call that answered with no parseable `result` event logs nothing; one that
did, but carried no cost, is kept and counted under *uncosted* rather than
silently read as free.

The *Spending* screen's *By milestone* reads the ledger's `milestone`
column first (`LedgerRow::milestone`, by its header name), as the run wrote
it, so a milestone closed long ago still owns what it cost. Only a row
older than the column falls back on the board (`task_milestones`, which
lists the open milestones and the last few closed), its task looked up by
issue number — the dev loop bills `61`, every other workflow `#61`. What
neither names is bucketed by kind, read from the stages of its run
(`traces::attribute_milestones`): `roadmap` (a planner run, its repository
map included), `PR` (PR review and PR fix), `hors board` (test debt, a
decision, a data diagnosis, a task under no milestone). There is no
`unknown` bucket left.

## The steward, and the doctor

The grinning mascot in a blue jumpsuit giving a thumbs-up, on the forecourt and in the hall.
Click them — or the `steward` button in the header — and the conversation
dock at the bottom left (below) becomes a terminal: an **interactive Claude
Code**, started in the harness checkout
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

A paid station — a robot, builder or inspector — carries a **Prompt(s)**
tab; its details tab shows no prompt. With several prompts the tab lists
them first, each unfolding into its editor. What a station sends is
`harness_workflows::prompts`' catalog: a key (`agent-loop/code`), the
stations that send it (a shared text, like the repository map or how the
tests are run, stands on several), and the default — the constant beside
the stage. The tab edits the stage's own text only; the EXECUTION CONTEXT
and SCOPE blocks stay the framework's.

**An edit is a draft, and only a start applies it.** Three stores, two
writers (`harness_core::domain::prompt_book`, `ports::store::prompts`,
JSON files in `harness-adapters`, written aside then renamed):

| store | written by | read by |
| --- | --- | --- |
| `.llocal/prompts.drafts.json` — one draft per prompt: a new text, or a revert | the view | the view; `harness watch`, once, when it starts |
| `.llocal/prompts.json` — the book: what runs, the last 5 edits, the last 3 reverted texts | `harness watch` alone, when it starts | every run, once (`dispatch::shared::prompts`) |
| `.llocal/prompts.shelf.json` — the drafts a human deactivated, the last 10 per prompt | the view | the view alone: a start never takes them |

The view reads the book through its own `Prompts` port, which has no way
to write it. `harness watch`, starting, takes the drafts (renamed out of
the way, so one saved meanwhile waits for the next start), applies them
(`PromptBook::apply`) and journals `ChangesApplied` — an Info notification.
A revert — putting back an edit, a reverted text or the default — keeps
what it replaced among the **recently reverted** (3), the history's second
list behind a switch.

**The history is one list, newest first**: the draft waiting for a
restart, the deactivated drafts and the edits that ran, then the default.
What runs now carries `runs now` and is read-only; any other entry opens
on an editor whose save makes it the draft. Saving over the waiting draft
replaces it; saving any other entry, reverting or reactivating keeps the
waiting draft in the history, **deactivated** (`POST
/api/prompts/{deactivate,reactivate,discard}`). A deactivated draft waits
on the shelf until it is reactivated or deleted. Each run records in its own folder the text of every
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
| a clone's code, `.git`, the ledgers, the event store, the locks, the view's mirror and mock-up workspaces, the launcher's toolchain shims (`toolchain/`, first on every watch process's `PATH`) | never |

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
branches — every `milestone/*` branch of the product, `GitHub::branches`,
an unarchived one marked "live, not archived"), then the versions of the
branch shown, newest first. A branch with no version archived shows its
live `data/schema.sql` and `data/model.json`.

**Compare ▾** holds the version shown against another — any other branch
at its newest, or an older version of the same branch — and draws what this
version changes (`harness_architecture::compare`, `GET
/api/data-model/compare?branch=&dir=&with_branch=&with_dir=`): a table or a
field here only is green, one that differs amber, one the other side only
has is ghosted in red; the inspector lists every difference, and a table or
field selected says what differs on it.

A table or a field selected tells who **uses** it on the branch shown
(`domain::data_usage`, `GET /api/data-model/used-by?branch=&dir=`): the
Capabilities whose persisted query reads it — `queries/registry.json`,
`<entity>.<field>`, a view read back to its table through the `CREATE VIEW`
the schema carries — and the `DataCapabilities` that write it, their target
aggregate read as the table of that name and their payload's fields.

Everything is read through core's `GitHub::file_text` (`ports::Product`,
`adapters::product`), or from a folder with `--data-dir <product root>`, by
**the data desk** (`data_desk.rs`): one local task that owns the `?Send`
port, answers the handlers through a channel, keeps every archived version
once read (a version never changes), and reads the index and the live files
again after a while. Routes `GET /api/data-model?branch=&dir=` and
`GET /api/data-model/versions`.

`harness_architecture::schema_sql` reads the DDL (a tokenizer and six statement shapes, views included,
never a panic: what it does not understand it skips), `harness_architecture::model`
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

**Building what is missing.** `main` and the integration branch are always
offered; a branch with no version archived shows its live files. A file not
there is a button: `schema.sql` starts `harness data-snapshot --branch <b>`
(the watch's snapshot pass, on demand), `model.json` — on a second click —
`harness data-model-draft --branch <b>` (Opus drafts the file from the dump
and the code, and opens a pull request on the branch). The view only starts
the harness binary the switch starts the watch with (`ports::Builds`,
`adapters::harness_builds`, output in `.llocal/logs/view-builds/`); the page
follows `GET /api/data-model/builds` and reloads (`?fresh=1`) when it ends.
Routes `POST /api/data-model/build`, `GET /api/data-model/builds`,
`GET /api/data-model/builds/{id}/log`; none under `--demo`. The binary is
`--harness-bin`, else a `harness` beside the view's own executable (built
with it), else the watch command's program — and it is asked first whether
it knows the command: one built before it is refused up front, with the
command that rebuilds it.

**Every action is followed** (`static/actions.js`): the button pressed gets
a log button (≡) that opens a log window — what the action said, and the
command's own output for a build, live — and an action that fails puts a red
card at the top of the page with **Ask doctor**, which types the action, its
error and its log into the doctor's terminal (`POST /api/doctor/ask`,
`domain::doctor::about_action`) and opens the doctor's pane. Wired so far:
the builds, the divergence's issue, a pin turned into a note or an asking,
the janitor's sweep and weighing, the plant's switch.

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
table; a part gone leaves its pin grey where it was. A pin belongs to **one
branch**: the data pane keeps a board per branch (`data-model--main-agent`,
`data-model--milestone-3`), switched with the branch shown (`board.setBoard`).

A pin is one of four kinds (`domain::annotations::Kind`; `Brainstorm` stays in the enum only to read the pins stored before it was dropped, shown as requests):

| kind | colour | what it is |
| --- | --- | --- |
| note | yellow | the human's words |
| request | orange | an intention on the part — applied to the page's draft by one AI call, or discussed in a session; kept for the next roadmap (how it reaches the planner is parked, `docs/PARKED.md`); `asking`, `simulate` and the old `brainstorm` read as this kind |
| link | blue | on the mock-up: leads to another page of the product, or creates one from an objective |
| design | teal | on the mock-up: a micro-frontend — made by an AI session from the spec the pin carries, or injected in the page it sits on |

A pin may be a **zone** instead of a point: the toolbar's rectangle, circle
and freehand tools draw an outline around several parts (`Note::shape`, in
board units, drawn in the `hb-zones` SVG layer that pans and zooms with the
world), annotated as one pin, moved as one.

A brainstorm or an asking starts from the note's text — it cannot be one
while the note is empty — and that text is the **first prompt** of its first
session. A pin's session is an interactive Claude Code (the steward's model, plan
mode, in the product's snapshot clone when there is one) in the
`note_session` pane, run by `note_desks.rs`. Its system prompt carries the
pin's note, its branch — and on a `milestone/<n>-…` branch, milestone #n as
the board has it: title, state, description, tasks — what the page shows of
the part it sits on (its rules, links, users) and that branch's
`data/schema.sql` as the version shown has it (`DataDesk::schema`). It opens with a fixed
`--session-id` and the pin's **conversation handed back** in its system
prompt (`domain::note_session`): whole up to 40k tokens, else a kept summary
of the older turns plus the last ~10k word for word — the summary is made
again only once the conversation grew 20k more, so a handback stays
identical for the prompt cache. Leaving the pane closes the session: its
turns are read from Claude Code's transcript (`~/.claude/projects/…/<id>.jsonl`,
`adapters::claude_cli::ClaudeTranscripts`) and appended to the conversation —
always kept whole — and a **compact** of the discussion (`claude -p`, at most
150 words: what is decided, what is open) becomes the pin's text. Turning a brainstorm into a note has Claude (`claude -p`,
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

## The mock-up, on its easel

The easel in the architecture office opens the **mock-up**: the product's
frontend without a backend — the static pages under `mock/` of the branch
shown, one per screen, on JSON fixtures (contract N, `mock/pages.json`) —
run in an iframe on a board. The bar offers the same branches as the data
pane (`main`, the integration branch, `feature ▾`), then the branch's
pages; the board is **one per branch and page** (`mockup--<branch>--<page>`),
so pins stay where they were left when the page changes. A transparent
shield over the iframe gives the board its pans and drops; `interact`
lifts it to click through to the page, and a page that navigates itself is
followed.

The board cannot reach inside an iframe, so the pane answers for it
(`mockup.js`): `under(x, y)` finds the part under the pointer in the page's
document — a `data-component`, an `id`, else a short tag path — and
`resolve(target)` finds it again, each as a stand-in that carries the
part's box through the iframe's place and the board's zoom. A pin's
session is told the page, its objective and the part's markup.

**Micro-frontends.** A page is static HTML plus the micro-frontends it
injects: a micro-frontend is a Micro-UI as the mock shows it — once built,
**its real code** bundled (`mock/src/<Name>.js`, `mock/build.sh`) on the
product's shell client over the mock server `harness init-repo` installs
(`mock/src/server.js`: the shell's protocol answered from `mock/fixtures/`,
taught by the product's `reads.js` and `commands.js`) with the installed
screen host (`mock/src/screen.js`: the `$ctx.<name>` context a frozen
screen composes with); before, a hand-written fragment —
`mock/components/<Name>.html`, listed under `components` of
`mock/pages.json` with its `needs`, `actions`, `props`, its `source` and
the `builtFrom` digest `mock/build.mjs` records (the harness recomputes it,
`harness_architecture::tree::digest`, and refuses a stale bundle), injected
as a `<div data-component="<Name>" data-props='…'>` that `mock/mock.js`
fills. A new page may carry a reference link — a Figma frame — that its
session draws from when a Figma tool is at hand (`Note::source`).
The easel has **two views**. In the *frontend* view, the `micro-frontends ◧`
switch outlines every one on the page in its own colour with its name
(a style the view injects in the page's document), and pins then land on
the micro-frontend rather than its parts; a strip beside the page lists the
branch's micro-frontends — the registry's and the manifests' Micro-UIs
together (`domain::mockup::units_of`), each marked mocked or built — to
**drag onto the page**: the drop is a **design** pin carrying the spec,
whose session injects it there (and makes it first when it has no
fragment). The *micro-frontends* view keeps the design system's rules
(`gallery.js`, shared with the kit): a gallery that only scrolls down, each
tile rendered alone from its fragment (`/mock/<slug>/__preview/<Name>`, a
page the view makes around one `data-component` element), the finder and
chips (real code, hand-written, not mocked, writes, on no page) above it; a
tile opens its micro-frontend **alone on the board**
(`mockup--<branch>--micro-frontends`, its own pins only), **used** on its
fixture or **inspected**, the previous and the next a click away and the
gallery found where it was left. Beside it, **its rules**, read from
`/api/capabilities`: every **actuator** — each `DataCapability` it
triggers, its effect and target, what it sends, the invariants it meets —
and every **view** — each Capability it reads, what it is handed and what
it returns — with the parts of the frame tied to each by the markup's own
words (`data-writes`, `data-reads`, contract D): pointed at in the panel,
they are outlined; picked in the frame, their unit is shown; a unit no part
names is marked *not tagged*. A **long press** on a kit component inside it
opens that component alone on the design system's easel, and the pane's
back arrow returns to the micro-frontend (`onPlace` keeps the view and the
one opened). A micro-frontend not mocked yet is **generated** on the
design system's model (`mf_desk.rs`, `domain::mf_gen`): from its tile, from
**generate N** in the bar, or from the form's *generate it*, several at
once (the kit's sessions). One generation is three steps — **plan**: a
light question reads the kit's index (`kit::index_line`, a line a
component, never its code) and answers the components it uses and those it
lacks; **kit**: the missing ones are added as custom components and
generated on the easel first (`KitDesk::custom`, `start_generation`, one
maker at a time so two plans lacking the same component make it once);
**write**: a question reads the skeletons of the components used
(`kit::skeleton`: elements and classes, no style, no script, no text), the
units' contracts and rules, the fixture when there is one, and answers the
fragment — tagged `data-writes`/`data-reads`, at rest, falling back on the
fixture when a prop is missing — and a fixture when there was none; the
fragment, the fixture and the manifest's entry (`mock/pages.json`) are
written in the branch's workspace and saved with the mock-up. The stable
parts of each prompt come first, the micro-frontend's own last. `kit.css`
is served derived from the design system's branch as it is now
(`mock_file`), so a generated micro-frontend wears the kit before any
publish. On a micro-frontend alone, request pins — dropped on it, or on
the very part picked while inspecting (`Note::part`, its path from the
fragment's root) — are **applied** by one question to its **draft**,
`mock/components/<Name>.draft.html` (`MfDesk::apply`, one rewrite of a
micro-frontend at a time, each from the last one's draft); the gallery and
the board show the draft (`__preview/<Name>?draft=1`), and **save…**
writes it in its place — every page that injects it changes — or as a
**fork**, a new micro-frontend listed beside it in `mock/pages.json`, the
origin untouched; **drop the draft** goes back. **+ micro-frontend** is a form —
name, system, description, the Capabilities it needs and the
`DataCapabilities` it triggers, chosen among the branch's, its props — that
becomes a design pin on that board and opens its session at once: the
session writes the fragment, the fixture and the registry entry. A design
pin's editor is the same form; its text is the spec written out, what the
session reads, never rewritten by a compact.

Two kinds are the mock-up's own. A **link** leads to another page — chosen
in its editor — or to a page that does not exist yet: named there, drawn by
an AI session from the objective the human writes (`+ page` in the bar does
the same from the bar). A **request** is an intention on a part — "the
customer pays late", "the total above the lines" — that the human writes
once and the AI applies: **apply to the draft** is one `claude -p` call
(`POST /api/notes/{board}/apply`, `NoteDesks::ask`, `drafts::apply_prompt`)
answering operations of the page's draft (below), one undoable step, the
pin marked *simulated* (`RequestState`; *stale* once its ops are dropped);
applied again, its former operations go and the new ones come; deleted,
they go. `simulate requests` in the edit bar applies every pending one in
one call, each request's refusal reported with its reason. **Discuss it**
opens the pin's session instead. The data pane's pins know the same
kind; `asking` and `simulate` are read as `request` (`Kind`'s aliases).
A request, a link or a design pin goes **on the backlog** from its
editor (`→ backlog`, `POST /api/notes/{board}/{id}/backlog`,
`annotations::backlog_issue`): an issue carrying `harness:roadmap` and
`harness:ready` — the same labels the data pane's divergence issue
carries, so the planner takes it from there — titled by the pin's first
line, its body the pin's text, where it sits, and what the easel adds
(the page and its objective, the part, the draft's summary); the pin
remembers the issue's URL and shows it. **Validate** on a draft drops a
request pin on the page that sums the version up — what moved, what was
added or rewritten, the forks, the requests — ready for the same button.
These sessions **edit**: they open in the launcher's permission
mode (a brainstorm stays in plan mode), in a **workspace** of the branch
the view makes on the first one — a detached `git worktree` of the mirror
under `.llocal/view-mockups/<branch slug>/` — and from then on the easel
serves the workspace, polling `/api/mockup?branch=` for a stamp that
changes with its files and reloading the page when it does. Nothing is
committed by a session. The bar then shows **edited, not saved**: `save the
mock-up` commits what changed under `mock/` on a new `mockup/<name>` branch
and pushes it (the code stage reads a `mockup/*` branch before building the
screen it names); `discard` removes the workspace, and the branch's
mock-up is shown as committed again. A product with no `mock/` yet shows an
empty canvas and the one thing that can be done on it: the first page.

**Edit mode, and the page's draft.** `edit` in the bar turns the page
into a designer in the manner of Figma (`design.js` the hands, `edit.js`
the engine, `mockup.js` lending both the page): a part hovered is
outlined with its name, clicked is selected — eight handles, its size
under it — shift-clicked joins the selection, a marquee on the empty page
takes every outermost part it encloses. A container laid out in flow is
an *auto-layout* frame: a part dragged through it shows the blue
insertion bar where it will land (before, after, or into an empty
container; a table cell drags its column), and several selected parts
land in their document order. A container switched to **free placement**
(shift+A, the panel or the menu; `layout: static` in the draft) is a plain
frame: its parts drag anywhere in it, snapping to red smart guides on the
siblings' edges and centres, resize from any handle (shift keeps the
ratio), nudge by the arrows (a pixel, ten with shift), and align or
distribute from the panel; a part dragged from flow onto a free container
lands where it is dropped. The **layers** tree at the left is the page
itself: click selects, drag reorders, the twist folds; the **properties**
at the right are the selection's identity, X/Y/W/H (in twelfths of the
row in flow), text, link target or button intent, the micro-frontend's
spec when it is one, and the requests on it. The **tools** (V move, F
section, T text, B button, L link, I image) drop a part where the page is
clicked, sized by a drag in a free container; the right-click menu and
the keys (⌘D duplicate, ⌫ delete, ⌘A select the siblings, ⇧1 fit, ⇧2
zoom to the selection, Esc) do the rest. With `micro-frontends ◧` on,
only micro-frontends move, as blocks; off, a part *inside* a real
micro-frontend edited from a page
**forks** it — a copy of the page's own, `<Name><Page>` (`variantOf`),
inlined, the page's manifest entry switched to it, a toast of seven
seconds to rename it. Every gesture is an **operation** on the page as
committed (`move`, `resize`, `remove`, `clone`, `text`, `static`, `add`,
`fork` — `by: human` or the request's id), never a rewrite of its HTML:
the page is always shown as *source + operations* (`MockEdit.engine`,
into the iframe's `srcdoc` — a step that finds every part on the page as
shown is applied to it in place, without a reload; forty steps of undo in
edit mode, ctrl+z),
so a draft survives the page changing under it — a page whose fingerprint
(`MockPage::hash`) no longer matches the draft's `base` shows the draft
**frozen**, its lost operations counted: keep it on the page as it is, or
restart from the page as it is (requests back to pending, pins kept).

The operations are the **draft**, kept by the view (`DraftStore`, the
`drafts` table of the pins' SQLite file) per board — per branch and page —
in **versions** (`domain::drafts`): one open version takes the operations
as they come (`PUT /api/drafts/{board}`, debounced); leaving the pane or
`keep` closes it, the next change opens the next (`POST …/open` with the
page's hash); the last three kept stay beside the open one, viewed read
only or **restored** as a copy in front, named if the human cares;
**validate** closes it as the next round of development — kept whatever
its age, and the page, its draft (`mock/drafts/<page>.json`: operations,
forks, requests, version) and the forked fragments written to the
workspace (`POST /api/mockup/write`) so `save the mock-up` commits them on
the `mockup/*` branch. The width the page is shown at (Mobile 390, Tablet
768, Laptop 1280, Large 1536, or dragged at its right edge) is the
version's `viewport`; the operations do not depend on it. Routes:
`GET /api/drafts` (every board's history, the pin board's *drafts* tab),
`GET|PUT /api/drafts/{board}`, `POST …/{open,keep,validate,unvalidate,reset}`,
`POST …/restore/{version}`.

Everything goes through the `Mockups` port (`adapters::mockups`): on
GitHub, `GitMockups` over the same `git_units::Mirror` the capabilities
list reads (`show`, `head_of`, a worktree per edited branch); under
`--demo`, `DirMockups` serves a copy of the sample seeded once under
`.llocal/view-demo/`, and a session edits the copy — the tracked sample
never changes, and a kit component, its reference image included, lands
in the product's repository only. Routes `GET /mock/{branch slug}/{*path}` (a page's relative links
resolve beside it), `GET /api/mockup?branch=`, `POST /api/mockup/{save,discard}`;
`POST /api/notes/{board}/{id}/session` carries the branch and the page for
a board named `mockup--…`, and the server makes the workspace before the
session opens. The product's rules on the mock-up — one page per screen,
`data-component` on every Micro-UI, the gates that refuse a screen no page
mocks — are `harness-architecture`'s (contract N, `tree::violations`).

## The design system, on its second easel

This plant is a **development factory**: it builds software, and its office
holds the tools of that trade. Other workflow families (a consulting report,
a research brief) run other kinds of factory with other rooms; this easel is
the development factory's own, nothing of it reaches `core`.

The second easel in the architecture office, beside the mock-up's, opens
the **design system** and its **kit** (`kit.js`, `kit_desk.rs`,
`domain::design`; the shapes and the rules are `harness_architecture::kit`,
contract O). Everything on it lives on one branch of the product, **`design-system`**,
born from the integration branch on the first visit (`Mockups::ensure_branch`)
and edited in a workspace of the view's own like a page's branch; the bar's
**publish the design** commits that workspace *on the branch itself* and
pushes it (`Mockups::publish`) — the design system is one truth for every
branch, never saved aside as `mockup/*`. Before the commit it derives
`mock/kit/kit.css` (`kit::stylesheet`: every generated fragment's style,
what the real frontend loads beside `tokens.css`); after it, it **carries**
`mock/design/` and `mock/kit/` onto the integration branch
(`Mockups::carry`: `git` plumbing on the mirror — `ls-tree`, `mktree`,
`commit-tree` — one commit that replaces those two folders and nothing
else, pushed, read again and spliced again when the branch moved). The
branches in progress find it on their next sync; the product's gate
`design-system-single-writer` keeps every pull request off those folders,
so no merge meets them. A carry refused leaves the publication standing,
and the toast says why. Three tabs — two of them, and the sources alone,
when the easel is mounted with `material` (the creation's brief step, where
the design system is given its material, not looked at):

- **design system** — the **tokens** (`mock/design/tokens.json`, the W3C
  design tokens format; `tokens.css` derived from it by the harness, one CSS
  variable per token, the file every page, micro-frontend and kit component
  consumes — change a token and the product restyles with no line generated
  again). **Injected** as a file, or **generated** by one `claude -p` that
  reads the **moodboard** and the **logo** kept under `mock/design/`
  (`Oneshot::ask_in`: the session runs in that folder with `Read` alone) from
  the human's brief; the sheet (palette, type, spacings, radii, three samples)
  is what the human **validates**, the tokens retouched by hand in a table.
  Before any validation the tokens are *the* candidate, with a real
  `tokens.css` of their own (`KitDesk::inject`, `::generate`: the first
  candidate writes it in place, same as validating does); after a first
  validation, tokens injected or generated again wait as a **candidate**
  beside the validated ones (`candidate.json`) until the human takes them,
  the live preview laying its variables over the validated sheet instead.
  Injecting, generating and validating each end by queueing the ticked
  bases' generation on whichever `tokens.css` is now there
  (`start_generation(vec![])`, best-effort — no Claude at hand is not
  these calls' business to report): the kit is styled from tokens that
  exist, not from the human's validation of them, so the simulated page
  means something while a candidate is still being judged — not only once
  it is taken. Validation opens a `harness:roadmap` + `harness:ready`
  issue — *install the design system in the product's frontend* — so the
  first real screen does not reinvent the variables; `design.json`
  remembers it. Once tokens exist, the sheet comes first, then
  the **simulated page** — a contact page the harness codes
  (`kit::SHOWCASE`, served at `/mock/design-system/__showcase/contact`)
  made of the kit's components only, their texts fitted to the page once
  each fragment is in, reloaded on every change of the branch, so a
  component the canvas saves shows there too — a candidate waiting
  shows its own copy of the page, the candidate's variables laid over the
  tokens, beside its sheet — and injecting or generating again is folded
  under *regenerate the design system*.
  Beside the moodboard, optional, up to six **interface examples**
  (`design.json` `examples`, images under `mock/design/examples/`, kept by
  `KitDesk::keep_asset(Asset::Example)`, described by `describe_example`,
  `POST /api/design/example`): screens whose structure — layout,
  navigation, density — the product takes after, never their colours,
  fonts or texts. Each is of the whole app, of a page (by its title) or of
  a kit component (by its name): a component's generation reads its own
  and the whole app's (`examples_for_component`, the session then run in
  `mock/` with `Read`), the mock-up's drawing the whole app's and the
  pages' (`examples_for_pages`). The tokens are never drawn from them —
  that is the moodboard's part.
- **catalogue** — the kit's components: the harness's sixteen **bases**
  (`crates/architecture/assets/kit/`, neutral fragments every application
  starts from: button, input, drop list, checkbox, radio, toggle, menu, tabs,
  card, modal, table, carousel, badge, toast, pagination, breadcrumb — twelve
  ticked by default) and the **custom** ones the human prompts (a name for
  humans — the code name is made from it, `kit::name_for`, unless a page
  already injects one — a text, a reference image, a sketch drawn on the
  pad). A kit component is a **visual
  primitive below the Micro-UI**: a fragment `mock/kit/<Name>.html` — markup,
  a `<style>` on the variables, a `<script>` only for a visual dynamic (a
  carousel turns, a menu opens) — with no data of its own and no network;
  what loads from a server is a micro-frontend. **Generate** runs up to
  `--kit-sessions` (3) one-shot sessions at once on `--kit-model`
  (`claude-sonnet-5-5`), each one question: the base fragment and the tokens
  in, the styled fragment out — applying the tokens to a base is what keeps
  it cheap. Resumable: what is generated stays, a run stopped halfway
  (quota, a refusal) takes only what is missing or failed next time
  (`kit.json` carries each component's state).
- **canvas** — two views. The **gallery** only scrolls down: a grid of
  three tiles a row (two when narrow), each a component rendered alone
  (`/mock/design-system/__kit/<Name>`, the tokens linked) at a fixed height,
  its frame loaded while it is on the screen or a screen away and let go
  further, so nothing moves as frames come in and a long kit stays light.
  The **finder** stays above it — every word typed in the name for humans
  or the code name, accents and case ignored, `/` to reach it — with the
  count and chips (standard, custom, drafts, with requests, fails the
  check); a list changed by them starts again from the top. A tile opens
  its component **alone on the board** (`kit--components`, note and request
  pins, **no backlog**: the kit is not roadmap work; the board shows that
  component's pins only, `only` in `board.js`): zoomed, **used** as a
  visitor would by default, pinnable at any time, the previous and
  the next of the list a click away, and the gallery found where it was
  left. **Inspect** picks a part of it — named by its path of children from
  the fragment's root, found again after a reload — and lists the tokens
  its rules wear, each swappable for another of its family (every colour,
  every radius, the font sizes): swapped in the frame at once, then **kept
  as the draft** (`POST /api/kit/{name}/draft`, the frame's styles written
  back into the fragment, refused when it fails `kit::check`), saved or
  dropped like an AI's. No Figma-like editor here on purpose: moving or
  recolouring by hand would write literals the kit refuses and part from
  the classes the real frontend reproduces. **Request on this part** drops
  a request pin carrying the part (`Note::part`), which the rewrite prompt
  names to the AI. The bar's **component background** paints every preview: the design
  system's `color.surface` by default, another colour the viewer picks kept
  in their browser. **Apply** on a component rewrites the fragment from its
  requests in one question, kept as the component's **draft**
  (`<Name>.draft.html`, what the card shows); **save…** then offers
  **replace** — the draft takes the component's place, and the list of
  **who uses it** (every page and micro-frontend of every active branch that
  injects it with `data-kit`, `KitDesk::usages`) is shown before the click —
  or **fork** — a new component, `variantOf` its origin, the origin
  untouched. A component no active branch uses may be **deleted**. A pin's
  **discuss it** opens its session in the design branch's workspace.

Each fragment carries the verdict of the kit's **static check**
(`kit::check`): under 60 000 characters, no `fetch`/`XMLHttpRequest`/
`WebSocket`/`EventSource`/beacon/dynamic `import`, no `<script src>`, no
`<link>`, no `@import`, no `<iframe>`, at least one `var(--…)`, and no
colour literal outside a variable's fallback. That verdict is the rule the
**code stage** reuses a component on: a frontend task reads the
`design-system` branch first, reuses a `data-kit` component as it is when
it passes, and otherwise recodes it for real and writes the corrected
fragment back on `design-system` with one line under `recoded` in
`kit.json` — the canvas then drops a **recoded** request pin on the card, so
the human sees what the real frontend changed.

On the mock-up, the design system reaches two places. In **edit mode**, a
**search field** beside the image placeholder tool lists the kit's
generated components as the human types; a pick arms the `kit` tool, and
the next click drops `<div data-kit="<Name>" data-props='{}'>` where it
lands — an ordinary part of the page from then on (moved, resized, forked
like any; its `props` edited as JSON in the properties, a `props` operation
of the draft). And the **request AI** (`drafts::apply_prompt`) is handed the
kit's components with the micro-frontends: it adds a kit component rather
than inventing markup, and for a visual component the kit lacks it answers
`skip: missing component: <Name> — <what>` instead of guessing; the page
then offers the four ways out — **build it** (the catalogue, the custom
form prefilled), **edit a component** (the canvas), **a placeholder** (an
image dropped for that request), **do without**. The pages find the kit
beside themselves on any branch: `/mock/<slug>/kit/…` and
`/mock/<slug>/design/…` fall back to the `design-system` branch, and a
branch whose `mock/mock.js` predates the kit is served the harness's own
loader, which fills `[data-kit]` like `[data-component]` and links
`design/tokens.css` once.

Routes: `GET /api/design` (the record, the tokens, the candidate, the
catalogue, the generations, the branch's state), `POST /api/design/{tokens,
generate,validate,candidate/drop,asset,asset/remove,publish,discard}`,
`POST /api/kit/{checked,custom,generate}`, `GET /api/kit/usages`,
`DELETE /api/kit/{name}`, `POST /api/kit/{name}/{prompt,apply,save,draft/drop}`.
Under `--demo`, the sample's `design-system` folder stands for the branch.

## The pin board

The cork board on the office's wall opens **every pin of every board**, by
branch — `main`, the integration branch, the milestones', and the boards no
branch matches — read from `GET /api/notes` (`NoteStore::boards`, then
each board's live pins). A board's name says where it belongs:
`data-model--<slug>` and `mockup--<slug>--<page>`, and the page matches the
slugs against the product's branches (the data pane's and the easel's own
naming). A click opens the pin where it was dropped: the data model at that
branch, or the easel at that branch and page (`pageSlug`), which centres
and opens the pin once its board is read (`board.goTo`, the `onLoaded`
hook). Its **drafts** tab lists the pages with a draft (`GET /api/drafts`):
the open version, the kept ones, the validated ones, the requests still
pending; a click opens the page in edit mode.

**The conversation dock.** A pin's session — brainstorm, request, link,
design, on the data model or the mock-up — no longer takes the pane: it
sits in a dock at the bottom left (`#talk`, `app.js`), floating over the
page, resized from its corner and remembered, one session at a time, kept
while the human walks the panes and pages. Two ways out, at its top
right: **validate & quit** closes the session with what it said joining
the pin (`POST …/session/close`, the pin's text compacted from it), **quit
without validating** stops it and drops what it said (`?keep=false`,
`NoteDesks::discard`); either way the canvas rereads its pins. The **×**
only closes the window: the program keeps running, and the pin or the button
sits down at it again.

The **steward** and the **doctor** sit in the same dock: a click on their
header button opens (or closes) theirs, the pin's ways out hidden. Every
session in the dock can be **pointed at a part of the page** with the
dock's own **📍** at its top left: the window steps
aside (hidden by visibility, so the terminal keeps its columns), the cursor
becomes a pin, the part under it is outlined, and a click hands it over (Esc
cancels). What is handed over is written by `app.js` (`contextOf`): where
the part is (the pane and its kind, or the plant's room), the view's own
code for that pane (`kit.js`, `mockup.js`…, for "show me the code of this
canvas"), what the pane's canvas says of the part (`contextAt` on the kit,
the mock-up and the data model — the same words a pin's session gets), the
frame it sits in and the file behind it (a kit component's
`mock/kit/<Name>.html` on `design-system`, a micro-frontend's fragment, a
page's branch and workspace), and the element's text and markup, the frames
looked through. It is **pasted** in the session's input — bracketed, one
block — once the program has been quiet for a moment, never sent: the
human's question comes after it.

The easel has a **full window** switch (`⛶`, Esc leaves) that puts the mock-up over
the whole page, the dock still floating above it.

## The capabilities list

The screen in the architecture office opens the **capabilities list**: every
Capability and `DataCapability` of the product, one row each, coloured by
where it stands (`domain::capabilities`):

| colour | status | when |
| --- | --- | --- |
| green | available | its manifest is on `main` |
| yellow | building | its manifest is on another branch only |
| blue | planned | an open task of an open milestone declares a `capability` or `data-capability` under `## Architecture`, and no task names an existing unit |
| red | bug | an open issue labelled `bug` names it — its id, or its name as a word of the title |

Like the data pane, a switch picks the branch shown: **all** (every branch
at once, the default), `main`, the integration branch, or a milestone's
(`feature ▾`). On one branch the list holds only the units that branch
carries, each as its manifest reads there, and nothing planned; the colour
still says where the unit stands across every branch. A detail and a fusion
test opened from it read the units as that branch has them
(`GET /api/capabilities?branch=`, `branch` in the fusion's body).

A row shows the name, the kind, the system, and the dates its manifest was
first and last committed, on any branch. The page searches by name as the
human types (only the list is redrawn, never the search box), sorts by
name, availability or date, filters by kind and by status — the legend's
chips — and pages fifteen rows at a time. A click opens the unit's detail:
its role (`description`; for a `DataCapability`, its effect and target),
its **interface contracts** — from the UI (`input` or `payload`, and each
Micro-UI that reads or triggers it with the props it hands), to the
dataview (the persisted queries a Capability `reads`) or to the
`DataGuard` (effect, aggregate, touches, mode, permissions) — what it
returns, and its **business rules**: the Concept a Capability implements,
or the aggregate's invariants a `DataCapability` declares, by id.

Everything comes from the manifests the architecture asks for, read with
`git` on **every branch** by the `Units` port (`adapters::git_units`): a
bare mirror of the product the view keeps under `.llocal/view-mirror/` —
cloned once, fetched at most once a minute, read by no run, a store the
janitor never sweeps — or the branches of `--data-dir`, or the sample's
folders under `--demo`. One `ls-tree` per branch, one `cat-file --batch`
for every manifest, one `git log` for every date. `units_desk.rs` keeps a
reading a minute, two requests sharing one read; `↻` reads afresh.

**A fusion test.** `⚭ fusion` turns the rows into picks; two picked, `test
the fusion` asks `POST /api/capabilities/fusion`. The **tooling** decides
first (`domain::fusion`): a read and a write never merge, nor two systems,
nor two Concepts, nor two aggregates or effects; a field one side lacks, a
field typed two ways, a mode, permissions, different invariants are an
**adaptable contract**; identical contracts and rules merge as they are.
Only then is the model asked (`claude -p`, `--fusion-model`, haiku by
default, no tool), with the comparison and the two contracts and nothing
else — a few hundred tokens — to word the verdict, the differences that
matter, and the merged unit. Route `GET /api/capabilities?fresh=`.

## The modules

```
src/main.rs            the wiring: two pollers, one server, one desk, one switch, one thread; the event store and harness-notify's feed, once per poll
src/cli.rs             what a human types; TARGET_REPO_URL, INTEGRATION_BRANCH, PERMISSION_MODE shared with the launcher; HARNESS_WATCH_COMMAND for the switch; HARNESS_HOME for the factory register
src/note_desks.rs      the pins' sessions at work: one Claude Code per open pin, its turns read back when it closes; a page to create edits the mock-up's workspace; `ask`, the one-shot a request is applied with
src/units_desk.rs      the capabilities list's reading, kept a minute; the fusion's one question to the model
src/data_desk.rs       the data desk: one task owning the Product port, versions cached, the handlers answered by channel
src/creation_desk.rs   a factory's creation at work: the record in the register, the AI's rounds, scope and requirements, the context gathered for the roadmap task and the record kept in the repository, the long jobs (found, connect, prepare, mock-ups, componentize, initialize)
src/removal.rs         a factory taken down: delink or destroy, the confirmation checked, the watch, the repository, the folder and the register in that order
src/pulse.rs           the pulse: a counter beaten when the product's workspace is written or a desk's job moves, streamed on /api/events; `PulsingMockups`, the port wrapped so every write beats it
src/janitor.rs         the janitor at work: weigh, sweep, the two-hourly round, one job at a time
src/desk.rs            a desk: one terminal kept alive between visits, scrollback, broadcast — the steward has one, the doctor another
src/domain/            the inside — no disk, no subprocess, no clock (`switch.rs`: the command line and the environment a view starts again with in another factory; `creation.rs`: a factory's creation, its gates, the AI's prompts and the reading of its answers)
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
  annotations.rs       the human's pins: their kinds, a zone's outline, the checks before one is kept, what ctrl+z can still bring back
  mockup.rs            the mock-up as the easel shows it: a branch as a URL slug, the pages of mock/pages.json, which paths may be served, the branch a save goes to
  note_session.rs      a pin's AI session: what it is told (a talk, a request, a page to draw), what is kept of it, what is handed back (40k whole, else summary + 10k)
  drafts.rs            a page's draft: its versions (open, kept, validated), what a new one opens on, the undo budget, the prompt a request is applied with and the operations read back
  capabilities.rs      the capabilities list: every unit on every branch, its status, contracts, callers, rules, dates
  fusion.rs            could two units be one: the deterministic comparison, and the small question the model words it from
  prompts.rs           a station's prompts for the pane: the catalog, the book and the drafts together; an edit, a revert, a withdrawal; what waits
src/ports/mod.rs       Traces (the disk), Board (GitHub), TerminalFactory/TerminalIo (a desk's program), Plant (the watch process), Limits (Claude's windows and GitHub's buckets, read now), Yard (.llocal, weighed and swept), Product (the product's files), NoteStore (the pins), Oneshot / Transcripts / TerminalMaker (a pin's session), Prompts (the book read only, the drafts), Trials (the dry-runs), Units (the product's manifests on every branch), Mockups (the mock-up's pages and files on every branch, a workspace a session edits, the branch it is saved to)
src/adapters/
  fs_traces.rs         .llocal/logs, read through core's own ledger readers
  gh_board.rs          core's GitHub port, read the way the router reads it
  pty.rs               a pseudo-terminal running `claude`
  watch_proc.rs        `ps`/`lsof` to find the watch, a detached spawn to start it, `kill` to stop it
  fs_yard.rs           .llocal walked and swept with std::fs, symlinks never followed, every path kept inside
  git_units.rs         the product's manifests, and the Mirror they are read through: a bare mirror under .llocal/view-mirror, a --data-dir checkout, or the demo's folders
  mockups.rs           the mock-up through the same mirror, a detached worktree per edited branch under .llocal/view-mockups, saved on a mockup/* branch; the demo's copy of the sample under .llocal/view-demo
  product.rs           the product's files: on GitHub at the integration branch, or in a folder
  sqlite_notes.rs      the pins and their conversations, in .llocal/harness.db
  relaunch.rs          the view `exec`ed again in another factory's folder, its `.env.local` loaded
  foundry.rs           a new factory: `gh repo create`, a worktree of the harness, `harness init-repo`
  claude_cli.rs        claude -p asked once (summary, conclusion, request), and Claude Code's transcripts read back
  prompt_files.rs      the prompt book, read only, and the drafts, read and written — through harness-adapters' stores
  trial_proc.rs        `harness trial` spawned with the text on stdin; its reports read back, a process dead before reporting told as failed
  limits_cli.rs        a minimal `claude -p` (haiku, no tool) for the subscription windows, `gh api rate_limit` for GitHub
src/server/            axum, one file per pane: `mod.rs` the state, the route table, the page's assets; `runs`, `desks`, `plant`, `janitor`, `data_model`, `mockup`, `drafts` (a page's draft and its versions), `design` (the design system's easel), `capabilities`, `notes`, `prompts` the handlers of each; `tests.rs` the router driven with fakes. The routes: the page, the scripts, /render/… (the wasm bundle, from disk), /api/snapshot, /api/events (SSE), /api/runs/… (tails, stages, graph), /api/issues/…, /api/history?from&to, /api/limits, /api/plant, POST /api/plant/{start,soft,hard}, /api/steward, /api/steward/term (WebSocket), /api/doctor, /api/doctor/term (WebSocket), /api/doctor/diagnoses, POST /api/doctor/diagnose/{workflow}/{run}, /api/janitor, POST /api/janitor/{diagnose,sweep,settings}, /api/data-model, /api/data-model/versions, /api/data-model/divergence, POST /api/data-model/issue, /mock/{slug}/{*path}, /api/mockup, POST /api/mockup/{save,discard}, /api/capabilities, POST /api/capabilities/fusion, /api/notes/{board}, /api/prompts/{line}/{station}, POST /api/prompts/{edit,restore,withdraw,trial}, /api/prompts/trials, /api/pending, POST /api/restart, /api/factories, POST /api/factories/{id}/open, /api/creation, POST /api/creation/…
static/                index.html, style.css, viewport.js (the viewport picker the mock-up's easel and the creation page's preview share), clock.js (every clock a human reads, in the harness machine's time zone — the traces stay UTC), app.js (data, navigation, panes, the three notification signs and their pane, the bridge to the renderer), board.js (a pannable, zoomable canvas with pins and zones), data-model.js (the UML diagram on it), mockup.js (the product's pages in an iframe on it, the draft, the requests, the forks), edit.js (the draft engine: a page as source + operations, undo, materialization), design.js (the designer's hands: selection, handles, guides, marquee, layers, properties, tools, menu, keys), vendor/ (xterm.js; morphdom.js, the creation page's redraws), render/ (built, not committed)
sample/                a product's data model to show under --demo: data/ (live files), archive/ (versions); units/ (manifests and mock/ pages, one folder per branch, dates.tsv)
```

**Every table sorts on a click of its header** — ascending, then
descending, a ▲ or ▼ beside it. A paged table (`pagedTable`) orders all its
rows before cutting the page; any other is ordered where it stands, and kept
so when it is redrawn (a `MutationObserver` on the pane). A cell sorts by
its `data-sort` when it carries one — a date's raw value — else by its
text, numbers as numbers, empty cells last. The order lives on the pane, so
the back arrow brings it back.

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
  except a log tail and an issue body, on demand and bounded — and the
  creation page's `/api/creation`, when the same stream's `pulse` says
  the product or a desk's job moved.
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
terminal is the plant-wide one), room 6 —
each shows its place and says so. Level D is wired for the lines (station,
employee, line), the sign, the chimneys, the issues, the store, the control
room, the steward and the doctor; the other components open a placeholder that names
what will come.
