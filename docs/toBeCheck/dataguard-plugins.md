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
