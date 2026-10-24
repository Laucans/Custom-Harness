# Rulings — what the owner already decided

Standing rulings: **do not re-ask, do not undo.** A new ruling is appended
here (date, Paris time, source) by whoever received it. Times Paris.

| Ruling | Since | Source |
|---|---|---|
| Board decisions (`harness:human`, `needs-decision`) are the attendants': act, then report in a line. Still ask before a destructive or account-level step (deleting issues or runs, mass closes, billing, repo visibility) | 10-17 | memory `feedback_factory_decisions_are_mine` |
| Any live error → understand the cause → harness fix with a test, not only the instance; every autonomous choice logged in `DECISIONS.md` | 10-18 | memory `feedback_errors_evolve_harness` |
| At most 3 building milestones (`routing::MAX_CURRENT`); the critical path is always admitted | 10-18 07:30 | `DECISIONS.md` |
| The data layer is laid first (one schema task, then aggregates in parallel); data model before the mock-up | 10-18 | memory `project_architecture_decisions_2026_10_18` |
| Lanes: `--parallel 5`, `--interval 60` (owner's numbers; the attendants lower them only per *Quota, budget, lanes*) | 10-20 12:30 | `DECISIONS.md` |
| `BUDGET_FLOOR=12` (USD per task) | 10-20 09:30 | `DECISIONS.md` |
| A PR's first two repairs run on sonnet, the next on opus | 10-20 | `3c7b995` |
| WIP limit = lane count: open task PRs ≥ lanes → no new task, merges and repairs first | 10-20 | router (`task PRs open for N lane(s)`) |
| `IGNORE_QUOTA=true` (no 90 % Claude reserve); turning it off is the owner's call | 10-21 | `DECISIONS.md` |
| Every merge is the harness's; no prompt says "merge"; sessions are denied `gh pr merge` | 10-24 15:55 | `d0a2a93` |
| Data layer on sonnet (`DATA_LAYER_MODEL=sonnet`, effort high) since 10-24 19:02 (`night/sonnet-switch-at.txt`, 17:02Z). Verdict (keep or revert) at ≥ 5 merged sonnet data-layer tasks via `tools/model_ab.py`; blind review at 3–4; no replay. Opus reference: $14.66/task all-in, 110 turns, 2.3 fixes. Other opus uses unchanged: one variable at a time | 10-24 19:02 / 21:35 | `priorities-H.md` item M |
| Harness work: 2 lanes (`main` + one worktree), 2 reviewers per lot, `cargo mutants` once at the end of the plan | 10-24 21:52 | memory `feedback_harness_dev_pipeline` |
| **The factory stays stopped during the whole harness development plan** and restarts **once**, when the whole plan is executed, tests included (full tests, clippy, mutants 0 missed, product gates) — **never per wave** or per lot batch. A plan is open while `<records>/night/dev-plan.md` exists. Outside a plan, the watcher's named restarts (*The watch died*, the GitHub quota) stand. Procedures: `playbooks.md`, *Stopping and starting the watch* | 10-24 22:37 / 23:05 | `DECISIONS.md`, memory `feedback_harness_dev_pipeline` |
| sccache approved for the builds | 10-24 22:37 | `DECISIONS.md` |
| Harness pushed only on the owner's word; `dataguard-vN` cut only when a product must move, on green CI | 10-20 | memories `project_harness_push`, `feedback_tags_when_needed` |
| Any added phase or changed plan is said in the same message | 10-21 | memory `feedback_announce_plan_changes` |
| Never `pkill -f <generic word>`: target a pid | 10-20 | memory `feedback_no_broad_pkill` |
| Persistence runs in Docker (`harness-postgres`); never a Homebrew service | — | `docs/PERSISTENCE.md` |
