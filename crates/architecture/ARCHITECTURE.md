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
src/assets.rs       the files init-repo installs: the document, CLAUDE.md rules, contracts A–R, ports.json and the sops scaffolding, gates, CI, scripts, the mutants configuration and rotation, the rules' kit, the tests' scratch-database guard (assets/scratch-db, `platform-scratch-db`: a database per test, dropped in `Drop` even when the test panics, named `scratch_<epoch>_<pid>_<seq>_<label>` for the watch's sweep)
assets/             those files, compiled in with include_str! — the harness's own scripts/{dead-code,mutants-rotation,test-debt}.sh are links to them, one script for both
tests/              test_scope_script.rs, dead_code_script.rs, mutants_rotation_script.rs: the installed scripts, run on a real folder
                    isolation_script.rs: the installed isolation.sh, on real workspaces — the healthy shape, each way a unit could reach another, a DataCapability on its target aggregate only, the registry crossed by the two servers only
                    gates_parity.rs: gates.yml's gates (each a step, found by its id) and tree::violations on the same fixture workspaces, one verdict for both
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
- **The cards are the units' contract with a session.** `rules.rs` tells a
  session building an aggregate, a `DataCapability` or a Capability the
  conventional items the glue calls (`plugin()`, or the four items), the
  one script it runs, and the engine's line; the parity tests keep every
  gate of `gates.yml` cited by a card.
