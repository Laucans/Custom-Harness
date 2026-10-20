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
      `stream.jsonl` usage shows `ephemeral_5m_input_tokens`, not `1h`.
- [ ] No session fails for a missing tool (an error naming a tool not in
      the list).
- [ ] Repairs: the log line `repair N on this PR — sonnet|opus`; the share
      of repairs followed by another on the same PR (opus baseline 45 %).
- [ ] Database down: no code session or repair starts, the journal says
      it each tick; back up: they resume.
- [ ] Red `rust` checks from the Mutants step: well under 11 in 20.
- [ ] Average code-session cost over a day against 4.54 $ (opus) and
      1.08 $ (sonnet).
