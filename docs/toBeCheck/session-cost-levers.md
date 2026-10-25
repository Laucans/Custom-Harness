# Session cost levers: tools, cache lifetime, repair model, database gate, local mutants

**Status: on `main`, deployed on ForgeDesMondes 2026-10-20, not yet measured live.**

## Context

Measured on ForgeDesMondes (2026-10-20): a code session on opus cost
4.54 $ (64 turns); its first message was 42k tokens, re-read every turn;
14 of the 19 tools it was given were never called; every cache write was
for an hour (2× input) while its turns follow seconds apart; all 65 repairs
ran on opus; 7 of 20 red `rust` checks came from tests that skipped locally
because the database was down all night, 11 from the in-diff mutants step.

## What shipped

- `3c7b995`: robots get `--tools Bash,Read,Edit,Write,Skill` (first message
  30.6k → 14.9k tokens on a probe); code and repair sessions write a
  five-minute cache (`CLAUDE_CODE_PROMPT_CACHE_TTL=5m`), forks and resumes
  keep the hour; a PR's first two repairs run on sonnet, the next on opus.
- the watch checks the product's `DATABASE_URL` each tick: down, code
  sessions and repairs wait (journal: "the database at … does not answer").
- the code prompts and the /code skill run `cargo mutants --in-diff` in the
  foreground before the PR, a test for every missed mutant.

## To verify live

- [ ] A code session's first-turn context ~26k tokens (was ~42k); its
      `stream.jsonl` usage shows `ephemeral_5m_input_tokens`, not `1h`. (contradicted 2026-10-24 22:15 Paris: the cache is 5m (`ephemeral_5m_input_tokens` ~34.5k, 1h = 0) but the first turn is ~38.8k tokens (4.3k read + 34.5k written) on 4 sonnet code sessions (#510, #512, #520, #521), not ~26k)
- [x] No session fails for a missing tool (an error naming a tool not in
      the list). (seen 2026-10-25 01:40 Paris: 10-20..10-24 streams hold 2 calls to a tool not given (Monitor); both sessions went on and ended `success`)
- [ ] Repairs: the log line `repair N on this PR — sonnet|opus`; the share
      of repairs followed by another on the same PR (opus baseline 45 %). (contradicted 2026-10-24 21:23 Paris: the line is there (`#516: repair 3 on this PR — opus`), but 91 of 133 repairs since 10-20 (68 %; sonnet 44/69 = 64 %) were followed by another on the same PR, against the 45 % baseline; #423 alone took 25)
- [x] Database down: no code session or repair starts, the journal says
      it each tick; back up: they resume. (seen 2026-10-22 07:48–11:13 Paris: database down, the warning every tick, no code session or repair started (one decide only); back up, PrFix #408 and lane 1 took #189 at 09:14Z)
- [x] Red `rust` checks from the Mutants step: well under 11 in 20. (seen 2026-10-25 01:40 Paris: ci on PRs since 10-22: 3 red runs of 80; the mutants step is now its own `mutants (0|1)` jobs)
- [x] Average code-session cost over a day against 4.54 $ (opus) and
      1.08 $ (sonnet). (seen 2026-10-24: 56 code sessions, mean 3.09 $ (last 24 h: 54 sessions, 3.48 $), between the 4.54 $ opus and 1.08 $ sonnet baselines)
