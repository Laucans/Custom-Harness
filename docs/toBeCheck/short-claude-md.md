# Short CLAUDE.md rules block (v3) and failing checks by name

**Status: merged on `main`, not yet verified in a real run.**

## Context

Every robot session pays the product's CLAUDE.md block as a cache write
(1-hour TTL, twice the input price). A fork from another lane pays it again:
measured on 2026-10-15, a fork in another directory re-sent a 30k-character
CLAUDE.md (7.9k tokens written), while a fork in the base's own directory
wrote 306 tokens. The block was 18.3k characters.

## What shipped

- `ea98655`: `pr_fix` keeps the failing checks of the PR it repairs as a
  `checks_failed` event in `.llocal/harness.db`. The query that groups them
  by check is in `docs/SESSION-CARRIER.md` ("Which checks fail").
- `3e4ce45`: the CLAUDE.md block goes from 18.3k to 7k characters (v3). All
  16 rules are kept, with the same numbers, in one or two lines each plus
  the gate that enforces them. Rule 16 is back inside the list. The rule
  cards (`crates/architecture/src/rules.rs`) gain what only the block said.
  The block has its own version (`CLAUDE_VERSION = 3`): a reinstall rewrites
  the CLAUDE.md block and leaves `docs/ARCHITECTURE.md` and
  `docs/DATA-INTEGRITY.md` (still v2) alone.

## To check in a real run

1. `harness init-repo` on a product with a v2 block (dnd_helper2):
   - CLAUDE.md's block becomes v3 and the project's own text around it is
     untouched;
   - `docs/ARCHITECTURE.md` and `docs/DATA-INTEGRITY.md` are reported as
     kept.
2. `cache_write` per `code` session in `costs.tsv` goes down by about 3k
   tokens. It was 120k on average over 6 sessions, so the change is within
   the noise: compare the first-turn write in `stream.jsonl` instead.
3. No more gate failures than before: `pr_fix` repairs per task, and
   `ArchitectureHolds` green on the first round.
4. Reviewers (`pr_review`), `pr_fix` and `rules` only see CLAUDE.md, not the
   cards. Check that their verdicts still cite the right rule.
5. After a few runs, the query in `docs/SESSION-CARRIER.md` lists failing
   checks by name. A gate that fails often is a rule worth more detail in
   its card.

When every point above is checked, delete this file.
