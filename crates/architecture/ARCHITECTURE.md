# harness-architecture

The architecture of the products the harness builds, written once. Every
target repository follows the same agent-native architecture: a read side of
many independent units and a write side behind one `DataGuard`. This crate is
that architecture as the harness knows it — what a task can build, the rules a
checkout must keep, the product's data model, the files that install it.

```
harness-workflows ─┐
                   ├─▶ harness-architecture ─▶ harness-core (the Disk port only)
harness-view      ─┘
```

## Why a crate of its own

- **It is the target's domain, not the harness's.** `core` is the framework
  for any agent workflow; a family that builds no software (a consulting
  report) must depend on it without inheriting Capabilities and `DataGuard`s.
  So the architecture is not in `core`.
- **It is not a workflow either.** `split` cuts tasks along it, `dev_loop`
  checks a round against it, `planner` places milestones in it, `init_repo`
  installs it, `data_snapshot` and `data_model_draft` read its data model, and
  `view` draws it. Kept in `harness-workflows`, one workflow ended up importing
  another's checks (`split` → `dev_loop::checks`), and `init_repo` included a
  file from `data_snapshot/assets/`.
- **One source.** The vocabulary, the rules and the installed documents change
  together; here they change in one place.

## The modules

```
src/vocabulary.rs   Unit, Effect, Side, Layer, Declaration: the `## Architecture` section of a task
src/tree.rs         Tree::read (the checkout's manifests, the unit crates, the registry's Cargo.toml, through Disk), violations, undocumented_concepts
src/model.rs        the agents' data/model.json merged with the DDL, field by field, with every divergence
src/compare.rs      two data models side by side: tables, fields, links and views added, removed or changed
src/graph.rs        the plan's graph (contract R): what every milestone produces and consumes, derived from the dossier; the read-ahead errors; the critical path
src/schema_sql.rs   pg_dump --schema-only read: a tokenizer and six statement shapes (views too), never a panic
src/kit.rs          the design system (W3C tokens → CSS variables), the kit of visual components (contract O), the 16 base fragments under assets/kit/, the showcase page (assets/showcase.html), the static check a fragment is reused on
src/assets.rs       the files init-repo installs: the document, CLAUDE.md rules, contracts A–R, ports.json and the sops scaffolding, gates and their local runner (gates-local.sh/.mjs), CI, scripts, the mutants configuration and rotation, the rules' kit, the tests' scratch-database guard (assets/scratch-db, `platform-scratch-db`: a database per test, dropped in `Drop` even when the test panics, named `scratch_<epoch>_<pid>_<seq>_<label>` for the watch's sweep), the units' test kit (assets/test-kit, `platform-test-kit`: Bench plants a unit's real manifest beside its aggregate over memory or a Scratch database migrated by the product's `data::migrate`; submit/applied/plant/refused, planter, uuid, writes_only_touches, a Capability's guard checks in `read` — what 80%+ of the first product's units retyped), the unit-conventions gate (assets/unit-conventions-gate.mjs)
src/mock_handlers.rs the mock server's handlers, one file per unit (mock/handlers/<id>.js): the rule every prompt states (RULE) and the parent-first migration a merge follows when the legacy files conflict as deleted (on_conflict: the split commit checked first — one parent, the split's paths only, or stop — and the add/add rule for a handler the base added before it split itself)
src/skeleton.rs     the skeleton of the unit a task is about to build (a DataCapability or a Capability, hostless aside): target (the id's folder, package, library, query file), render (assets/skeleton/<kind>/ filled with what needs no judgement — Cargo.toml, the manifest's constants and what the declaration states, the lib.rs and test frames on the test kit, the query frame named after the folder — every judged part a `SCAFFOLD:` hole), the commands that register and check it (register.sh, queries.sh, cargo check --all-targets); the dev loop's Scaffold writes it
src/generated.rs    the files of a product a script writes (GENERATED: Cargo.lock, the registry's Cargo.toml, queries/registry.json, mock/components/*.js, mock/pages.json's builtFrom digests), how a conflict in each is taken, their generators in run order (Regenerator), the digest-only resolution of mock/pages.json (digests_only, both marker styles), the repair prompt's lines (instructions)
assets/             those files, compiled in with include_str! — the harness's own scripts/{dead-code,mutants-rotation,test-debt}.sh are links to them, one script for both
tests/              test_scope_script.rs, dead_code_script.rs, mutants_rotation_script.rs: the installed scripts, run on a real folder
                    isolation_script.rs: the installed isolation.sh, on real workspaces — the healthy shape, each way a unit could reach another, a DataCapability on its target aggregate only, the registry crossed by the two servers only
                    gates_parity.rs: gates.yml's gates (each a step, found by its id) and tree::violations on the same fixture workspaces, one verdict for both
                    queries_script.rs: the installed queries.sh against a golden registry (tests/golden/) written by the GNU path — and on macOS, under PATH=/usr/bin:/bin with /bin/bash 3.2 and BSD sed, byte for byte
                    mock_server.rs: the installed server.js on node — the engine's behaviour, handlers from their own file imported once, an id defined twice an error naming both files (at startup for a declared id, breaking every call; on first use otherwise), an undeclared payload key warned in the engine's words (top-level keys, the target's key exempt, an empty payload unchecked, any version), never refused; the graph read beside the page (__product/) or at the repository's root
                    journey_kit.rs: the installed rules' kit.ts on node (types stripped, Playwright stubbed) — a running journey fails once its body ran on the engine's undeclared-key warning in a POST /commands answer, "not a rule contest" in its first line, and on nothing else, an answer after the body, from a second context, or cut by a close included; the page's in-flight counter; a pending one stays on the plain test; the engine's warning, the kit's pattern and the mock's words checked against each other; the mounting on a throwaway product's manifests — a screen or a unit its needs do not cover is mis-mounted (annotated, never thrown), what a screen's Micro-UIs declare covered, a page's read off its input red, a link the shell does not serve (a mock-up file name, a 404) red and never a mis-mount; the status reporter writing mis-mounted whatever the outcome; red when CI has no node that strips TypeScript
                    business_kit.rs: the installed business kit's mounting.rs compiled as it is — a unit reached its needs do not name (the version no part of a need, a short name no id), a variable off the input with what the manifest takes (dotted heads, a fixture id a value), only the engine's undeclared-key warning; lib.rs wires them and judges the mounting before the body's outcome
                    rules_gate_status.rs: the installed rules-gate.mjs status on a throwaway repository — a mis-mounted test said apart after the other rules, winning over its rule's red (listed under it), failing always, a conform rule too; skips name the node that answered
                    split_handlers.rs: the installed split-handlers.mjs on a throwaway product — byte-for-byte split, idempotent, deterministic, every refusal with nothing written (guarded destructuring and for-of assignments, `this`), a file without semicolons, a handler file a merge already brought, the self-check catching a helper reached through eval
                    mock_handlers_gate.rs: the gate `mock-handlers` from gates.yml on a throwaway repository — ids twice, another id's key, a new legacy key on a pull request once mock/handlers/ holds a handler file (its README and _lib/ do not count)
                    skeleton_product.rs (ignored, on demand): the test kit and both skeletons rendered in a throwaway product — rustfmt-clean, `cargo check --all-targets`, the kit's clippy and tests (its Scratch with SCRATCH_DB_URL), the frames green, the holes red, the filled manifest planted through Bench
                    unit_conventions_gate.rs: the gates `data-capability-conventions` and `capability-conventions` on a throwaway repository — each rule on an added unit, the changed-unit rules on an edited old one, an old unit's past never judged
                    capability_sql_gate.rs: the part `queries-valid--capabilities` on a real PostgreSQL (GATES_PGPASSWORD) — a changed Capability's SQL prepared against the migrated schema, an untouched one unjudged
                    gates_local.rs: the installed gates-local.mjs on node and git — every step of gates.yml read (the sync guard: the ids in order, none unreadable, no reason given on any) and every expression of it in the table, with its value under the local pull request; on throwaway products with a bare origin: the uncommitted work judged, merged with the base's tip, a conflict a finding, base-red only when the head's errors are the base's (one more is red), a sparse checkout's hidden files judged and its sparse list left as it was, two jobs side by side and a job's steps in order, an unknown step unreadable (exit 2), a job or workflow key it does not run (needs, env) or a job env it cannot give unreadable, continue-on-error and timeout-minutes run, the gates.yml of the merged tree (a step the base added), --failed (the red gates, a red closure, the merge after a conflict), a Postgres gate skipped without docker, the closure through a fake HARNESS_BIN with the facts (exit 2 when it cannot judge); red when CI has no node 22
                    assets_rustfmt.rs: every installed .rs asset passes `rustfmt --check --edition 2024` under default settings — a product's CI runs `cargo fmt --check`, and the assets are no member of this workspace, so `cargo fmt --all` here never sees them
```

## Decisions

- **No workflow, no run.** Nothing here knows a stage, a session or a label
  of the tracker — with one exception, on purpose: a `Side` is spelled with
  its label (`harness:read-side`, `harness:write-side`), because that
  spelling is part of the `## Architecture` declaration a task carries. The
  constants live here; `workflows::common::labels` re-exports them.
- **Pure but for the reading.** `Tree::read` is the only function that
  touches a checkout, and it does so through core's `Disk` port: the rules
  run on an in-memory `Tree`, a test hands them one. Its tests use a
  read-only `FakeDisk` of their own, which refuses any write.
- **Which files are generated is here, once.** `generated::GENERATED` is
  read by the harness's catch-up of a milestone branch
  (`workflows::common::branching::catch_up_locally`: take the base's side,
  run the generators) and by the repair session's prompt (`pr_fix`'s
  conflict block, `generated::instructions`): one table, so the two never
  disagree on which file a script owns. `mock/pages.json` is half a hand's:
  a conflict there is generated only when its `builtFrom` digests are all
  that differ (`digests_only`; `mock/build.sh` rewrites them). The bundles'
  HTML fragments are hand-written and out of it. `generated::staled_by`
  names the generators a merge runs even when git merges it cleanly: the
  mock-up's build when one side changed a file every bundle is built from
  (`tree::stales_every_bundle`), or when its changes are not all known.
  Both ways: asked of the side merged in, then — only when that side
  `brings_bundles` (a bundle or `mock/pages.json`) — of the receiving
  side, whose own build change stales the arriving digests. The catch-up
  then skips GitHub's merge, and the merge train refreshes a task PR
  before it lands. Pure, like the rest.
- **A payload is judged against its manifest the way the engine judges
  it, and only there.** The DataGuard engine accepts a payload key its
  manifest does not declare with the warning `payload key(s) <keys> not
  declared by <dataCapability>` in `Submitted.warnings` (`crates/dataguard`,
  `undeclared_keys`: top-level keys, a dotted declaration `entity.id`
  declaring `entity`, the target's key always declared, a manifest that
  declares no payload checking nothing, the version ignored). Two nets read
  that one rule, no grammar of their own (H3 v1): the rules' journey kit
  (`assets/rules/journey/kit.ts`) fails a running journey, once its body
  ran, when a `POST /commands` the **real** server answered carries the
  warning — the journeys never run on the mock — with "fix the payload, this is not a rule contest" in the line the
  rules' gate shows; the mock server mirrors the check exactly, as a loud
  warning (the answer's `warnings`, `console.error`, `server.undeclared`),
  never a refusal: the mock is never stricter than the engine (contract H,
  `DATA-INTEGRITY.md`). The mock reads the manifests indexed by their
  `dataCapability` field — never by a folder's name — from
  `__product/data-capabilities.json` beside the page's folder, which the
  factory view serves read-only; `plan/graph.json` the same way, else at the
  repository's root two folders up. The kit answers each page's
  `POST /commands` itself (`route.fetch`, then `route.fulfill`), in the
  default context and in every one `browser.newContext()` opens, so the
  answer is read before the page sees it; an init script counts each
  page's commands in flight, and a page or a context the test closes, and
  the kit's own teardown (it takes `page`, so it ends while the page is
  open), wait for them — a command the body did not await, or one a close
  would have cut, is read all the same. `tests/journey_kit.rs` pins the
  engine's words, the kit's pattern and the mock's template together; it
  runs the kit on node (22.6 or later, pinned in CI) and is red, not
  skipped, when `CI` is set and no node strips TypeScript.
  Generated command types and a CI typecheck of every Micro-UI are a later
  lot.
- **A rule's test is checked for its mounting at its first real run, in
  the kits, and a fault there is the test's (H5 v1).** Once the units it
  needs are built the manifests exist, so the kits judge what the test
  reached against them, with no mock and no host of their own:
  `assets/rules/business/src/mounting.rs` (std and serde_json only, wired
  into `Server::read`/`command` of the business kit's `lib.rs`) refuses,
  before sending, a unit the test's needs do not name and a read's variable
  its `capability.json` `input` does not take (a dotted input declaring its
  head), and reads the engine's undeclared-key warning back from the
  command's answer — the engine stays the one grammar of a payload. The
  journey kit (`kit.ts`) sees what the pages reach (it already answers
  every `POST /commands`; it now looks at each `POST /capabilities/<id>`
  and each screen route, then lets them go): a screen its needs do not
  name, or a unit none of its needs covers — named, or in the
  `needs`/`actions` of a Micro-UI it needs or one its screens lay out — is
  the test's; a page's read off its input is the Micro-UI's, red with "fix
  the read, this is not a rule contest", as H3's payload is. Values are
  never typed: a fixture's id where a manifest says `uuid` is accepted. A
  test's fault records the state `mis-mounted` and passes — never red,
  never a contest: its outcome proves nothing. `scripts/rules-gate.mjs
  status` says those rules apart, after the others, each with its rule's
  red tests listed under it, and fails on every one, `conform` or not; a
  screen counts as opened only by a valid name the shell answers (a link
  like `/screens/x.html`, or a 404, is the product's, red); the rules' task fixes the
  test before anything is contested (WRITE step 3, CONTEST's first step,
  FIDELITY's check 3), and a development or a repair copies the line into
  `CONTEST.md` — the one road from a development's PR to the rules' task.
  The mock rehearsal of the design (an HTTP host for the mock, a CI step)
  is not built: it waits for a measurement that contests survive this.
  The business kit's `record` writes each line in one `write_all`, so the
  suite's parallel tests no longer interleave their status lines, and it
  indexes the manifests once per process, as `kit.ts` does.
- **A unit's mock handlers are its own file.** The mock server
  (`assets/mock/src/server.js`) imports `mock/handlers/<id>.js` the first
  time `<id>` is read or commanded — never bundled, so a handler file is no
  bundle's input and adding one touches no shared file. A file defines its
  own id only (`export const reads` / `export const commands`); the legacy
  `mock/src/reads.js` and `commands.js`, which every branch appended to,
  are still loaded while they exist. An id defined twice is an error naming
  both files, never an override: at startup for every id the legacy files
  or `mock/pages.json` (`needs`, `actions`) name — the server then answers
  every call 500 with `server.problems` — and on first use for any other
  (a browser cannot list a folder). `tree::SHARED_SOURCES` and
  `build.mjs`'s `SHARED` are one list, `server.js`, `screen.js` and the
  legacy `reads.js` / `commands.js`, so a product on an older `build.mjs`
  writes the digest the harness checks; an absent legacy file is no input,
  so a split tree digests the server and the screen host only. Rewriting a
  file every bundle embeds stales them all (`tree::stales_every_bundle`):
  the install rebuilds them in its own commit. `assets/split-handlers.mjs` (installed
  as `scripts/split-handlers.mjs`, run once per tree by the migration,
  never by the harness yet) splits them byte for byte, refuses what it
  cannot carry faithfully (a computed key, a handler assigning a top-level
  `let`, node older than 22), and self-checks by running the server on both
  layouts; `assets/mock-handlers-gate.mjs` is the gate `mock-handlers`,
  which reads the files as text with the splitter's tokenizer. New products
  are seeded `mock/handlers/README.md`, no legacy file. The rule a session
  reads is `mock_handlers::RULE`, the same in the cards, the code prompt,
  `CLAUDE-rules.md` (block v7) and `pr_fix`'s conflict block; a unit's
  narrowed checkout carries its own handler files and `_lib/` only.
- **What to install is here; whether to write it is not.** `assets` lists the
  files and their content; `init_repo::data::install` decides what a run
  writes of them (nothing already there is overwritten, `CLAUDE.md` is
  appended to).
- **The kit's bases stay here.** `assets/kit/` holds sixteen neutral
  fragments no `init-repo` installs: the view's easel styles them with the
  product's tokens and writes the result on the product's `design-system`
  branch (`mock/kit/`). What a product carries is the styled kit, never the
  harness's base — and `kit::check` is the one reading of "reusable as it
  is" that the view shows and the code stage applies.
  `assets/showcase.html` is the same kind of thing: a page the harness
  codes once — a contact page of the kit's components (`kit::SHOWCASE`) —
  that the view shows to preview a product's design system at work.
- **The gates run locally from the workflow itself.**
  `assets/gates-local.mjs` (entry `gates-local.sh`, node 22, no dependency)
  reads the product's `gates.yml` at run time — never a generated copy, which
  goes stale on the product's first edit — with a parser for the YAML the
  harness writes and an exact-string table of the expressions it uses
  (`if:` and `${{ }}`); a step it cannot read is `unreadable`, never skipped
  in silence, and `tests/gates_local.rs` keeps the table in step with
  `assets/gates.yml`. It judges the pull request's tree: a snapshot of the
  work (the real index copied, so a sparse checkout hides nothing) merged
  with the base's tip, each job in a worktree of its own under
  `.git/gates-local/`, jobs in parallel, a job's Postgres service a
  container of its own removed on exit and reaped when a killed run left
  it. A red gate is `base-red` only when the base is red there with every
  error the head reports (`::error` lines), the base's verdict taken from
  its last finished `gates` run at the same commit (gh), else from a rerun
  of the jobs that build nothing. `rules-hold` is off unless `--rules`. The
  closure check is not rewritten in JS: it is the harness binary's
  (`HARNESS_BIN architecture-check`), the loop's own reading.
- **The rules exist twice, knowingly.** `tree::violations` is the loop's own
  reading; `assets/gates.yml` runs the same rules in the target's CI. A change
  to one is a change to the other, and `tests/gates_parity.rs` runs both on
  the same fixtures. `units-registered` is mirrored by its outcome, not its
  text: the loop checks that the registry's `Cargo.toml` names every unit
  crate and no other, the libraries' names, that a unit takes the
  engine from the workspace (`harness-dataguard = { workspace = true }`),
  and that `[package.metadata.cargo-machete] ignored` lists the registry's
  unit packages, sorted (the glue that uses them sits in `OUT_DIR`, which
  `cargo machete` does not read); the gate runs `scripts/register.sh --check`, which also compares the
  engine's line in the registry and in the root's `[workspace.dependencies]`
  with the server's.
  One gate has no loop reading, on purpose: `scratch-databases-shared`
  judges the files a task's pull request *adds*, from the merge base (a
  test creating its database by hand instead of taking a `ScratchDb`; a
  milestone's pull request and a push are not judged again), which a `Tree`
  of one checkout does not carry; `tests/scratch_gate.rs` runs its script on
  a throwaway repository, and `tests/scratch_db.rs` (ignored; run by the
  harness CI's `postgres` job) compiles the guard against the engine by path
  and proves its panic test. The guard is in every narrowed checkout
  (`closure::ALWAYS`: no unit reaches it in the cargo graph before its first
  test names it) and out of the product's mutants (`exclude_globs`).
- **Units live under their system.** Aggregates, `DataCapabilities` and
  Capabilities are read under `crates/<system>/{aggregates,
  data-capabilities,capabilities}/`, the DataGuard's folder included, so
  that a violation names the folder a misplaced unit belongs in — for an
  aggregate, the one system the `DataCapabilities` targeting it share.
- **The registry is the script's.** A unit task's closure carries
  `crates/dataguard/registry/` (and the server's `Cargo.toml`, which
  `scripts/register.sh` copies the engine's line from); `closure::describe`
  marks it "regenerated, never edited".
- **One engine per product, by the workspace.** The engine's line is
  written once, in the server's `Cargo.toml` (its release tag);
  `scripts/register.sh` copies it into the registry and into the root
  `Cargo.toml`'s `[workspace.dependencies]` (added under that header, or in
  a new section, when missing; replaced when it differs). A unit takes
  `harness-dataguard = { workspace = true }`, so Cargo builds one engine and
  the server's `Plugin` is the unit's. A unit with a source of its own fails
  `--check` (a warning when the script writes, so `init-repo` still installs
  on a product not migrated yet). The script edits the root only on install
  or a tag move, never when a unit is added.
- **The installed scripts are portable.** A product's script runs in CI
  (GNU, bash 5) and on the owner's Mac, where BSD `/usr/bin/sed` reads `\s`
  as a literal `s` and once wrote a wrong registry while exiting 0. So
  `queries.sh` uses POSIX classes (`[[:space:]]`), no `globstar`, and
  `shasum -a 256` when `sha256sum` is missing; `tests/queries_script.rs`
  compares its output byte for byte, never only its exit code.
- **The cards are the units' contract with a session.** `rules.rs` tells a
  session building an aggregate, a `DataCapability` or a Capability the
  conventional items the glue calls (`plugin()`, or the four items), the
  one script it runs, and the engine's line; the parity tests keep every
  gate of `gates.yml` cited by a card.
- **A unit starts from the harness's skeleton, not a neighbour.** Sessions
  that copied the first unit of their lane copied its drift (`names()`,
  `MANIFEST`, a second filter in Rust). `skeleton.rs` renders the canonical
  shape from `assets/skeleton/`, which no product installs: the templates
  are rendered, not copied, so they are not `FILES`, and
  `tests/skeleton_product.rs` compiles what they render against the engine
  of this checkout and the installed kit. Only the data layer's two kinds
  are covered; an Aggregate or a Micro-UI has too little left to scaffold in
  the first product, or no compile check to keep a template honest.
- **Conventions are gated on what a change adds or changes.** The
  convention gates judge the units a pull request adds against every rule,
  and the units it edits only against the rules every existing unit already
  keeps: an older unit's past deviation never turns an edit of it red. The
  deviations a static check cannot judge without false positives are review
  hints in `pr_review`'s brief, never gates.
