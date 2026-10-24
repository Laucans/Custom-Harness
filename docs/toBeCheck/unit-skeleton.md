# Unit skeleton, test kit and convention gates (SK v1)

## Context

Sessions building a DataCapability or a Capability spent ~40 calls (opus) /
~31 (sonnet) before their first edit looking for a neighbour to copy, and
copied its drift (`names()`, `MANIFEST`, a second filter in Rust, a camelCase
query file, missing guard tests). The owner approved the critique's simpler
alternative (`harness-dnd2-run/night/sk-critique.md`) plus convention gates.

## What shipped (uncommitted on `main` when written — see `git log` for the commit)

- `crates/platform/test-kit` asset (`platform-test-kit`, rule 21): `Bench`
  (manifest beside aggregate, memory or `Scratch`), `Scratch` (ScratchDb +
  `data::migrate` + `seed` + `rows`), `submit`/`applied`/`plant`/`refused`,
  `planter`, `uuid`, `file`, `writes_only_touches`, `read::*` guard checks.
  In `FILES`, the closure's `ALWAYS`, the mutants exclude list.
- `harness_architecture::skeleton` + `assets/skeleton/{data-capability,capability}`.
- `dev_loop::action::scaffold::Scaffold` before `Narrow` in `code`
  (`--no-scaffold` / `NO_SCAFFOLD` off); `Ask`'s `--- SKELETON ---` block;
  `ArchitectureHolds` refuses a left `SCAFFOLD:` hole.
- Gates: `data-capability-conventions`, `capability-conventions`
  (`scripts/unit-conventions-gate.mjs`), `queries-valid--capabilities`,
  `scratch-databases-shared` extended (own connection in `tests/`). 32 gates.
- Rules block v8 (rules 21, 22), cards, `pr_review` brief hints.

## Rehearsed (scratch clone of the product at 19dfbe5, kit installed)

- `live.setMicrophone` (cross-system aggregate `Session`, projects): 6 files,
  24 holes, ~16 s warm; fmt, clippy -D warnings, isolation, register --check,
  conventions gate green; the manifest filled → planted over memory; the
  PostgreSQL frame migrates a Scratch and leaves no database.
- `session.getSessionJournal`: 4 files, 11 holes, ~3 s; queries --check,
  isolation, conventions gate, its SQL prepared.
- Existing folder (`table.setTokenVisibility`), dirty tree: skipped, said.
- A failing `cargo check` (forced): folder removed, `Cargo.lock` and the
  registry restored, tree clean.

## Changed-rule survey (product `main_agent` at a55bd4d, 2026-10-24)

A simulated PR touching every unit (each `src/lib.rs` and every
`crates/*/queries/*.graphql`), judged by the gate as committed here:

- 50 DataCapabilities: every convention holds (holes included).
- 21 Capabilities: one finding, a real defect —
  `campaign.listEntitiesToCreate` (#519): `list-entities-to-create.graphql`
  selects `sessionPosition`, its `pub const SQL` has no `session_position`
  (`parity`). Any PR touching that Capability or its query file goes red
  until the product adds `s.position AS session_position` to the SQL (or
  drops the field from the query file). Not fixed from the harness side.
- `scratch-databases-shared` (code forms, comments ignored), run as if
  every product test file were added: the 6 memory-only
  `crates/live/data-capabilities/*/tests/engine.rs` pass; 75 of 204 files
  open their own connection (pre-kit helpers, hand-rolled `struct Scratch`)
  and would be refused if added today — only added files are judged.

## To verify in a live run

- [ ] Before the upgrade lands in ForgeDesMondes: `campaign.listEntitiesToCreate`
      fixed in the product (`s.position AS session_position`) or filed as a
      `harness:roadmap` issue; re-run the survey above on the product head:
      expect 0 findings on both kinds.

- [ ] Init-repo / watch-start upgrade installs `crates/platform/test-kit/`,
      `scripts/unit-conventions-gate.mjs`, gates.yml (32 gates) and rules v8
      in ForgeDesMondes; the product's own CI green on `main_agent` after it.
- [ ] The product's `.cargo/mutants.toml` (diverged copy) gets
      `crates/platform/test-kit/**` excluded — by hand if the upgrade keeps it.
- [ ] First scaffolded DataCapability and Capability on a real lane: journal
      says `skeleton: <id> written uncommitted … N hole(s)`; never
      `taken back` (any rollback is a template bug: fix the template).
- [ ] A skeleton never overwrites: a resumed or repaired task (branch exists),
      an existing folder, a dirty lane → `skeleton: none — …`.
- [ ] Narrow after Scaffold: the closure holds the unit, its aggregate (other
      system included), `crates/platform/test-kit`, `scratch-db`, `crates/data`.
- [ ] The session stays on the skeleton (its branch carries the files), fills
      every hole; the round's post-gate finds none left, and a repair round
      that leaves one is refused by the `holes` rule of the product's CI.
- [ ] Exploration calls before the first edit drop on sonnet sessions
      (`model_ab.py` first-edit index): target ≤ 15 (baseline sonnet 31.5),
      context at first edit ≤ 90k (baseline ~150k).
- [ ] The convention gates stay green on PRs that edit untouched old units
      (no new red on `main_agent` PRs not adding a unit).
- [ ] `queries-valid--capabilities` green on a PR changing a Capability
      (17/18 legacy SQL prepare; `list-scenes` reads `dataguard_rows`, skipped).
- [ ] `pr_review` brief cites the known-deviation hints when relevant, never
      as a blocking verdict on its own.
