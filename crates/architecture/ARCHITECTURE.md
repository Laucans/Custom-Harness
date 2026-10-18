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
src/tree.rs         Tree::read (the checkout's manifests, through Disk), violations, undocumented_concepts
src/model.rs        the agents' data/model.json merged with the DDL, field by field, with every divergence
src/compare.rs      two data models side by side: tables, fields, links and views added, removed or changed
src/graph.rs        the plan's graph (contract R): what every milestone produces and consumes, derived from the dossier; the read-ahead errors; the critical path
src/schema_sql.rs   pg_dump --schema-only read: a tokenizer and six statement shapes (views too), never a panic
src/kit.rs          the design system (W3C tokens → CSS variables), the kit of visual components (contract O), the 16 base fragments under assets/kit/, the showcase page (assets/showcase.html), the static check a fragment is reused on
src/assets.rs       the files init-repo installs: the document, CLAUDE.md rules, contracts A–R, ports.json and the sops scaffolding, gates, CI, scripts, the mutants configuration and rotation, the rules' kit
assets/             those files, compiled in with include_str! — the harness's own scripts/{dead-code,mutants-rotation,test-debt}.sh are links to them, one script for both
tests/              test_scope_script.rs, dead_code_script.rs, mutants_rotation_script.rs: the installed scripts, run on a real folder
                    isolation_script.rs: the installed isolation.sh, on real workspaces — the healthy shape, and each way a slice could reach another
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
  to one is a change to the other.
