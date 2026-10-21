# The write side by unit: registry, `register.sh`, server as a library

## Context

`docs/DATAGUARD-CUT.md`, phases A5a–A5c. A5a ships the installed write side:
the generic server library, the registry crate whose `Cargo.toml`
`scripts/register.sh` writes, the product's `crates/server` binary, and
`init-repo` running the script once. Green tests are a throwaway product
built from the assets (`crates/architecture/tests/dataguard_server.rs`,
`--ignored`), the engine by `path`: no real product has run it yet, and the
tag the skeleton names is not cut. A5b (gates) and A5c (rule cards,
catch-up regeneration) add their own points below.

## What shipped (A5a)

- The A5a commit (see `docs/DATAGUARD-CUT.md`, row A5a): `assets/dataguard-server/`
  (`Cargo.toml` at `tag = "dataguard-v2"`, `src/lib.rs`), `assets/dataguard-registry/`
  (`build.rs`, `src/lib.rs`), `assets/register.sh`, `assets/server/`,
  `Plugins::needs`, `init-repo`'s `Process` port.

## To verify

- [ ] The tag `dataguard-v2` is cut on a green `main` and pushed, on the
      owner's word, before any product installs these assets: the skeleton
      names it already (`assets/dataguard-server/Cargo.toml`).
- [ ] `harness init-repo` on a product without a registry runs
      `bash scripts/register.sh` in the install clone, and its one commit
      carries `crates/dataguard/registry/Cargo.toml` beside the assets; the
      report says `registered the units: …`.
- [ ] On ForgeDesMondes (M1): `crates/server/Cargo.toml` is reported
      `diverged, the product's own now — kept`, and nothing of its
      `crates/server/` is overwritten.
- [ ] A unit crate that takes the engine itself (an aggregate's `Judge`, a
      DataCapability's `Prepare`) resolves to the same `harness-dataguard`
      as the server. Decided in A5c: `harness-dataguard = { workspace = true }`,
      the root's `[workspace.dependencies]` line written by `register.sh`
      from the server's. Proved on the throwaway product (engine by `path`);
      on ForgeDesMondes after M1, `cargo tree -i harness-dataguard` shows one
      package, at the tag.
- [ ] The root `Cargo.toml` lists `crates/dataguard/registry`,
      `crates/server` and the three unit globs (`workspace-members-by-glob`,
      A5b).
- [ ] `cargo run -p server -- inventory` on the real product lists every
      unit and its needs; `serve` refuses to start while a need is unshared.

## What shipped (A5b)

- The A5b commit (see `docs/DATAGUARD-CUT.md`, row A5b): `gates.yml`
  (`units-registered` replaces `data-capability-registered`,
  `aggregate-placed`, `ownership` refuses a derived field touched,
  `workspace-members-by-glob` covers `crates/*/aggregates/*`),
  `isolation.sh` (kind `aggregate`, a DataCapability on its target
  aggregate's crate only, the registry depended on by the two servers only),
  `tree.rs` (aggregates under `crates/*/aggregates/*` and the mirrors),
  `closure.rs` (the registry in a unit's closure, an aggregate's own
  folder), the refinement's layout check and the split prompt.
- Green is `gates_parity.rs` and `isolation_script.rs` on fixture
  workspaces: no real product has run the new gates.

## To verify (A5b)

- [ ] ForgeDesMondes is red on purpose until M2: its twelve aggregates sit
      under `crates/dataguard/aggregates/`, so `aggregate-placed` fails on
      each (naming the system its DataCapabilities share), and
      `units-registered` fails while it has no registry. No compatibility
      path was added. Expected after M2: both green on `main_agent`, and
      `tree::violations` (the dev loop's reading) empty of either rule.
- [ ] The gate's `cargo metadata --no-deps` on the real product resolves
      without the network (the engine is a `git` dependency) and in under
      a minute on the CI runner.
- [ ] `isolation.sh` on the migrated product: every DataCapability depends
      on its target aggregate's crate and on no other, and nothing but
      `crates/dataguard/server` and `crates/server` depends on the registry.
- [ ] A DataCapability whose target aggregate is another system's: the
      gate lets it depend on that crate (a decision of A5b, not of the
      plan) — confirm the owner wants writes across systems allowed this way.
- [ ] Narrowing, if it comes back: a unit's closure carries
      `crates/dataguard/registry/`, which reaches every unit crate by path,
      so the graph pass brings every unit folder back (cargo needs them).

## What shipped (A5c)

- The A5c commit (see `docs/DATAGUARD-CUT.md`, row A5c): the rule cards
  (`rules.rs`: Capability's four items, DataCapability's `plugin()` and
  `Prepare`, Aggregate's crate with `Judge` and `Project`/`ProjectPg`, the
  engine from the workspace), `CLAUDE-rules.md` v5 (rule 18, a unit is its
  folder), `ARCHITECTURE.md` (§F, §I, *One engine*, *Generated files*, two
  gate rows), `DATA-INTEGRITY.md` (*The units' code*), `register.sh` (the
  root's engine line, `--check` refusing a unit's own engine source),
  `branching::catch_up_locally` wired by the launcher's
  `dispatch/catch_up.rs`, `pr_fix`'s conflict prompt, the view's paths.
- Green is the fakes (`branching` tests) and the throwaway product's two
  branches, whose merge conflicted on `Cargo.lock` and the registry's
  `Cargo.toml` and was resolved by the regeneration alone. No live watch
  has met such a conflict yet.

## To verify (A5c)

- [ ] `harness init-repo` (or the watch's upgrade) on ForgeDesMondes writes
      `harness-dataguard` under the root's `[workspace.dependencies]`, below
      its own entries, and appends the CLAUDE rules v5 block in place of v4.
- [ ] A live watch whose milestone branch GitHub refuses to catch up on
      `Cargo.lock` and the registry's `Cargo.toml` only: the journal says
      `… holds main_agent: Cargo.lock and crates/dataguard/registry/Cargo.toml
      regenerated, never merged by hand`, the merge commit is on the
      milestone's branch, and the held task starts at the next tick.
      Expected cost: one clone (`catch-up`, under the workspaces) the first
      time, then a fetch.
- [ ] Same, with a source file conflicting too: the watch still says
      `held: … cannot take main_agent`, the `catch-up` checkout is left
      clean (merge aborted, released), nothing pushed.
- [ ] `cargo metadata` in the `catch-up` checkout fetches the engine by git
      within its 10-minute limit on a cold cargo cache.
- [ ] A `pr_fix` session on a PR conflicting on the generated pair follows
      the prompt: `git checkout --theirs Cargo.lock`, `register.sh`,
      `cargo metadata`, no hand-edited registry line in its merge commit.
- [ ] A session building a DataCapability or an aggregate from its card
      alone writes `plugin()`, takes the engine with `{ workspace = true }`,
      and its PR touches only its folder, `Cargo.lock` and the registry's
      `Cargo.toml` (M5's proof).

## What shipped (A6)

- The A6 commit (see `docs/DATAGUARD-CUT.md`, row A6), the engine the
  skeleton names as `tag = "dataguard-v3"` (not cut yet):
  - B1: the undeclared-key warning reads the submitted payload;
  - B2: `api::Submission` reads `"target": {"id": …}` beside `targetId`;
  - D1: `Context::row` (any partition, as the queue leaves it) and
    `Context.rows` (read-only SQL, `Engine::with_rows`, wired by the
    skeleton's `plant` over PostgreSQL);
  - D2: `Prepared.then`, follow-ups in `Submitted.then`, one level only;
  - D3: `Prepared.attachment`, kept in `Entry`, answered in
    `CommandResult.attachment` once applied;
  - D5: `ProjectPg::hydrate`, asked by `PgStore::row`;
  - D6: `Clock::now_millis`, `Context.now_millis`.
- Green is the engine's tests over memory, its `#[ignore]`d tests on the
  Docker PostgreSQL (hydration, attachment across a restart), and the
  throwaway product (`target.id`, `cx.row`, `now_millis` in its unit). No
  real product has run any of it.

## To verify (A6)

- [ ] The tag `dataguard-v3` is cut on the A6 commit once its CI is green,
      and pushed on the owner's word: the skeleton names it already.
- [ ] ForgeDesMondes on `dataguard-v3` (M1): no `payload key(s) … not
      declared` warning on the ten DataCapabilities whose wire keys the
      legacy `submit()` renames; `note_import_invariants.rs:268` passes
      with its exact warnings.
- [ ] The rules suite (`tests/rules/business`, `target.id`) runs green
      against the real server, not only the mock.
- [ ] `ai.extractFromNotes` as a `Prepare` with `then` (N proposals, then
      the note): the answer's `then` lists N+1 `submitted`, the note's
      idempotency key replays the whole without resubmitting a proposal.
      `campaign.createEntitiesFromNote` needs the outcome of its Entity
      commands to choose its stamps: check whether a static `then` covers
      it or the product keeps a route (D2's fallback).
- [ ] `campaign.exportCampaign`: the archive comes back in
      `GET /commands/<id>` once applied (`result.attachment`), never before.
- [ ] A fixture campaign and media loaded by `data::insert_missing` are
      seen through `hydrate` by `grid-only-for-battlemap` and by export.
      Expected: the M2 PRs' Postgres tests on the real tables.
- [ ] The AI DataCapabilities stamp `cx.now_millis`: two in one second are
      told apart.

## What shipped (A7)

- The A7 commit (see `docs/DATAGUARD-CUT.md`, row A7), the engine the
  skeleton names as `tag = "dataguard-v4"` (not cut yet):
  - `harness_dataguard::tokio_postgres`, the engine's client re-exported
    (feature `postgres`), for an aggregate's `ProjectPg`;
  - `register.sh` writes the registry's
    `[package.metadata.cargo-machete] ignored`, every unit package, sorted;
    `tree.rs` mirrors it;
  - contract B's optional `host` (`server` | `hostless`).
- Green is the engine's tests, the gate parity tests, and the throwaway
  product (a `ProjectPg` over the re-export, `ports-gate.mjs behind`,
  `cargo machete`). No real product has run any of it.

## To verify (A7)

- [ ] The tag `dataguard-v4` is cut on the A7 commit once its CI is green,
      and pushed on the owner's word: the skeleton names it already.
- [ ] ForgeDesMondes on `dataguard-v4`: each aggregate crate with a
      `ProjectPg` drops its `tokio-postgres` line, `externals-behind-ports`
      is green on `main_agent`, and `Cargo.lock` holds one `tokio-postgres`.
- [ ] After `bash scripts/register.sh`, the product's `cargo machete` (its
      CI) reports none of the registry's units.
- [ ] A hostless Capability manifest (`"host": "hostless"`) passes
      `manifests-conform-to-contracts` in a real product's CI.

## Found by the ForgeDesMondes migration (M2-noteimport, PR #334)

- [ ] **D2 follow-ups come back through the same `Prepare`.** M2-noteimport
      had to "announce and take back" each follow-up: it records it in a
      shared `Announced`, and submits it with an empty payload that the
      `Prepare` recognises by its target. The engine should let a `Prepared`
      follow-up carry an already-prepared payload that skips `Prepare`, or
      tell a follow-up from an author's submission.
- [ ] **A `Prepare` cannot read its follow-ups' verdicts.** The legacy
      validation filtered "at risk" elements, and the plugin version cannot.
      Decide whether `then` should run before the first command's own
      queueing, or whether "at risk" stays per element.
