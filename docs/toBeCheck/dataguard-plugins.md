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
      as the server: same `git` URL and tag, or `workspace = true` from a
      root `[workspace.dependencies]`. Two tags compile two engines whose
      `Plugin` types do not match. A5c's cards say which form.
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
