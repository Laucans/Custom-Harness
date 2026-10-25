# The CI made faster: mutants sharded, gates grouped

## Context
A ForgeDesMondes PR push started about 30 jobs (28 gate jobs, `rust`,
`typescript`); five lanes push at once, and the account runs 20 jobs at a
time. Over the last 33 PR runs `rust` averaged 13.1 min, 8.9 of them the
in-diff Mutants step.

## What shipped
- Products' CI (`crates/architecture/assets/ci.yml`): the in-diff mutants
  left `rust` for a job `mutants`, 2 shards (`--shard k/2`), no debug info,
  `--test-tool nextest`, its own Postgres; one artifact per shard
  (`mutants-0.out`, `mutants-1.out`).
- `assets/mutants.toml` and `.cargo/mutants.toml`: `minimum_test_timeout`
  60 → 20; `all_features = true` replaces
  `additional_cargo_test_args = ["--all-features"]` (built once, with the
  features the tests run under).
- Products' gates (`assets/gates.yml`): 28 jobs → 4 (`shell-gates`,
  `node-gates`, `rust-gates`, `rules-hold`); each gate a step whose id and
  name are the gate's, every gate runs (`!cancelled()`), a last step
  writes `::error title=<gate>::` for each failed gate.
- Harness CI: `mutants` in 2 shards, debug off, nextest; the rotation with
  debug off, nextest and `timeout-minutes: 90`.

## To check
- [x] Harness CI on the commit: both `mutants (k/2)` shards green, each
      judging about half the mutants; compare the slowest shard with the old
      single `mutants` job. (seen 2026-10-22 11:53 Paris: run 39758035980 on 74f782f — mutants (0/2) 29 mutants in 3 min, (1/2) 28 in 7 min, both green; no like-for-like single-job run to compare)
- [x] Re-install the assets on ForgeDesMondes (`harness init-repo`): the
      four gate jobs and `mutants (0)`, `mutants (1)` appear on the next PR;
      a PR push starts 8 jobs, not 30. (seen 2026-10-24 19:15 Paris: PR heads 6f5e24c/b317ec7 ran 8 jobs — rust, typescript, mutants (0), mutants (1), shell-gates, node-gates, rust-gates, rules-hold)
- [ ] On a product PR: `rust` loses the ~9 min of mutants (expect ~4 min);
      the slower mutants shard under ~5 min on a typical task diff. (contradicted 2026-10-24 22:59 Paris: 12 product PR ci runs of 24 Oct — `rust` took 5–15 min (median ~8), not ~4; the mutants shards held at 0–1 min)
- [x] A red gate in a grouped job: its step is red with the gate's id as
      its name, the other gates of the job still ran, and the job's
      annotations carry `title=<gate>`. A fix session (`pr_fix`) finds the
      gate from `gh run view --log-failed`. (seen 2026-10-24 18:07 Paris: node-gates on 5c1253c — step `pages-mocked--bundles` red, 14 other steps ran, annotation title `pages-mocked`; pr_fix 20261024-144911 named `rule-tests-untouched` from `--log-failed`)
- [x] `rules-hold` still uploads `rules-status`, and the view still reads it. (seen 2026-10-24 23:13 Paris: run 40133015694 uploaded `rules-status`; the view's read not checked)
- [ ] No product test relies on `cargo test`'s one-binary-at-a-time order:
      nextest runs binaries in parallel (ForgeDesMondes' database tests
      create one database per test, so they should hold). (contradicted 2026-10-24 22:53 Paris: run 40131733032 (feat/speech-to-text-streaming) — the nextest baseline failed `the_adapter_passes_the_contract_on_a_stub_sidecar` (stub sidecar missing under target/tmp/phonon-stubs) while `rust` (cargo test) passed on the same commit)
- [ ] No mutant reported missed only because nextest skips doctests.
