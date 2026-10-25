# Skill evals

The harness's skills (`.claude/skills/*/SKILL.md`) are prompts. Nothing else
says whether a session still follows one after it changes. These evals do:
they score what sessions really did against deterministic checks, and compare
two windows or two versions.

They test the harness's own tools, not a product, so they live here and run
by hand, like `cargo mutants`. They are not a factory workflow.

## Two ways to get sessions

**Observe the factory: free, the default.** The factory keeps every round's
stream in `.llocal/logs`. `observe` scores them, the build half of a code
round included. Compare the rounds before a skill change with the rounds
after it:

```
node scripts/skill-evals.mjs observe ~/.harness/factories/<f> --until 2026-10-26 --label before
node scripts/skill-evals.mjs observe ~/.harness/factories/<f> --since 2026-10-26 --label after
node scripts/skill-evals.mjs report before after
```

Different tasks on each side make it noisy: conclude on many rounds, not five.
`observe` writes `.llocal/evals/runs/<label>/observed.json`; a label that
holds it is scored from it alone, so keep observed and replayed labels apart.

**Replay a case: paid, for a controlled comparison.** A case is one real
prompt of the factory frozen with the product commit it was sent on
(`evals/cases/<skill>/<name>/`). `run` replays it several times with this
working tree's skills, or a git ref's, in a clone that cannot act: tools Bash,
Read and Skill only, unknown commands denied, a `gh` stub that fails and no
GitHub token or login in its environment, and no ref or reflog entry newer
than the case. The objects of later commits stay in the clone's store: a
session that searches them by hash could still read them. A code case is told to stop after its plan, since it
cannot build, so a replay judges the plan half only.

```
node scripts/skill-evals.mjs run code --label before --skills-from HEAD
node scripts/skill-evals.mjs run code --label after
node scripts/skill-evals.mjs report before after
```

`--changed` replays only the skills this working tree modifies, `--reps` sets
the repetitions (3), `--jobs` the parallel sessions (3), `--mirror` a local
bare clone to clone from (or `SKILL_EVALS_MIRROR`). Labels and case names are
plain names, never paths.

## Adding a case

```
node scripts/skill-evals.mjs harvest ~/.harness/factories/<f>/.llocal/logs/agent-loop/<run>
node scripts/skill-evals.mjs harvest ~/.harness/factories/<f>/.llocal/logs/split/<run>
```

`--name` overrides the case's name, `--expect-stop` marks a case whose real
round rightly stopped, `--repo <owner/name>` names the product when the
factory's lanes do not. A code round is harvested with the files its merged PR
changed, as the truth its plan is scored on; those include the files later
repair rounds of the same PR touched. A round that started from an uncommitted skeleton is
refused: its files are in no commit, so it cannot be replayed. The cases carry
the product's own text (issues, rules): they are that product's, kept here as
fixtures.

## The checks

- **code**, on a replay's plan: the four pre-flight findings, the files of the
  real PR found, at least two commands that prove a step, the stop line, and
  no `AGENT_LOOP_STOP` unless the case expects one.
- **code (observed)**, on a real round: the pre-flight and the stop line
  written out, nothing staged with `git add -A`, `.` or `commit -a`, a
  verification run when the round delivered, no `gh pr merge`, and a final
  `AGENT_LOOP_OK` or `AGENT_LOOP_STOP`. Commands are read one simple command
  at a time, quoted text dropped, so a grep for `gh pr merge` is no merge.
- **split**: no command denied, the answer parses, every slice is well formed, the data layer
  names its effect and aggregate, every slice carries a spec, dependencies are
  in range and acyclic, branches are unique, and the slice count stays close
  to the factory's.

The scoring is tested by `node --test scripts/skill-evals.test.mjs`, in CI.
