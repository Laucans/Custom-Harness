# Mock handlers: one file per unit (H1, round C)

## Context

Every unit's mock handlers lived in two shared files, `mock/src/reads.js`
and `mock/src/commands.js`. Every branch appended to them, so every
catch-up conflicted there. An object literal's later key also silently won
over an earlier one, which hid a payload mismatch (Battlemap vs the
`table.*` DataCapabilities). Both files were bundled into every component,
so one handler added anywhere made all 12 bundles and their `builtFrom`
digests stale.

## What shipped (uncommitted on `main`, round H1-C)

- `assets/mock/src/server.js`: on first use of `<id>`, it imports
  `mock/handlers/<id>.js` at run time (`export const reads` /
  `export const commands`, keyed by that id only). It still loads the
  legacy `mock/src/reads.js` / `commands.js` while they exist. An id
  defined twice (its file and a legacy file, or a key of another id in a
  file) is an error naming both files, never an override. **At startup**
  the server looks up every id the legacy files define and every `needs` /
  `actions` id of `mock/pages.json`: a problem there breaks the server
  (every call answers 500 `the mock server did not start: …`,
  `server.problems`, `console.error`). An id nobody declared is checked on
  first use, answered 500 alone (a browser cannot list a folder).
- Bundle digests: `build.mjs` is **unchanged** and `tree.rs`
  `SHARED_SOURCES` keeps the same four names (`server.js`, `screen.js`,
  `reads.js`, `commands.js`). An absent file is no input, so a split tree
  digests the server and the screen host only, and a product on its old
  `build.mjs` agrees with the harness whatever it runs.
- The install (`harness init-repo`, and the upgrade at every watch start)
  runs `sh mock/build.sh` in its clone when it writes a file every bundle
  embeds or builds with (`tree::stales_every_bundle`: `mock/src/server.js`,
  `screen.js`, `mock/build.mjs`, `mock/build.sh`) and the product has a
  `builtFrom` in `mock/pages.json`. The rebuilt bundles and digests go in
  the upgrade's commit. A failing build stops the upgrade before the commit
  (the watch says it and goes on, nothing pushed).
- `scripts/split-handlers.mjs` (new asset): a byte-for-byte, deterministic
  split. It refuses on node < 22, a computed key, a spread, a getter, a
  shorthand entry, a key defined twice, a handler that assigns a top-level
  `let` (`x =`, `x +=`, `++x`, `[x] = …`, `({ x } = …)`, `for (x of …)`),
  a handler using `this`, a READS/COMMANDS reference, an old server.js that
  still imports a legacy file, or a handler file already there with other
  content. A file written without semicolons splits (ASI). A handler file
  already there with the same content is kept (a merge brought it). Before
  writing anything, its self-check runs the server on both layouts: same
  handler set, same sources, same answers.
- Gate `mock-handlers` (`scripts/mock-handlers-gate.mjs`, node-gates job).
  The PR rule "no new key in a legacy file" starts with the **first handler
  file** in `mock/handlers/` — the seeded README and `_lib/` do not count,
  so the upgrade alone turns no open PR red.
- New products get `mock/handlers/README.md` and no legacy files.
- `harness_architecture::mock_handlers::{RULE, on_conflict}` goes into:
  the code prompt, the code skill, the unit cards, `CLAUDE-rules.md`
  (block v7), the rules task, the view's design briefing, and `pr_fix`'s
  conflict block. `on_conflict` checks the split commit first (one parent,
  only the split's paths, or stop and report) and carries the add/add rule
  below. A unit's narrowed checkout carries only its own handler files and
  `_lib/`.

- **Round D (uncommitted on `main`, after `a45a86b`): a merge rebuilds
  what it stales, both ways.** `common::catch_up::staled(receiving,
  incoming)` reads, through `GitHub::changed_files` (GitHub's compare),
  what `incoming` changed since the merge base; only when that brings
  bundles (`generated::brings_bundles`: a `mock/components/*.js` or
  `mock/pages.json`) does it read, with a second compare, what `receiving`
  changed. A file of `tree::stales_every_bundle` on either side (or a list
  GitHub cut at 300 files) names `sh mock/build.sh`
  (`generated::staled_by`). The reverse side covers a milestone whose own
  task edited `mock/src/screen.js` (no rule forbids it) while `main_agent`
  brings another milestone's components with digests of its own
  `screen.js`.
  - **Milestone catch-up**: a stale merge (or a compare that cannot be
    read) skips GitHub's merge; the local catch-up merges in the
    `catch-up` checkout and runs `sh mock/build.sh` even without a
    conflict (`branching::BaseHead`), only where the merged
    `mock/pages.json` has a `builtFrom`. What it rewrote is committed on
    top of git's merge (`chore: sh mock/build.sh after merging main_agent
    (<sha>) into <branch>, … regenerated`), then pushed. One such catch-up
    a tick (`LOCAL_PER_TICK`): the other branches say `… its local
    catch-up waits for the next tick`. A change elsewhere keeps GitHub's
    merge.
  - **Task PRs (decision)**: under "one task per Micro-UI" nearly every
    open UI task PR has a component only it has, so a GitHub refresh after
    a staling catch-up would turn each one red and cost one paid `pr_fix`
    session per PR for one `sh mock/build.sh`. Instead `refresh_task_prs`
    asks `staled(pr.head, milestone)` per PR and leaves a stale one on its
    old base (said: `task PR(s) #n take it when they land — …`). The merge
    train is the guard: `milestone_merge::run`, after the checks read
    green, asks the same question and answers `Attempt::Stale` instead of
    merging; the launcher refreshes the PR in the `catch-up` checkout
    (`dispatch::catch_up::locally`, `sh mock/build.sh`, commit, push), and
    the PR lands at a later tick once green again. A PR skipped as `Busy`
    meets the same guard, so none lands an old digest — there is no
    behind-base check in the merge route otherwise (green checks from the
    older base are all it reads). A refresh that fails or meets a residual
    conflict labels the PR `harness:pr-fix`; while it carries that label
    the train answers `Attempt::Repairing` and mounts nothing (no retry
    every tick). A
    compare that cannot be read keeps the old behaviour (GitHub refresh;
    a merge on its checks).
  - **Cost**: no `pr_fix` session for stale digests. Per catch-up, 1
    compare, 2 when `main_agent` brings bundles; per refreshed task PR, 1
    or 2 more; per merge attempt, 1 or 2 more; per stale task PR, one
    local refresh (a checkout mount, `sh mock/build.sh`, one push) and one
    extra CI run before it lands. After a change to the mock-up's build
    itself (an upgrade's `server.js`), every task PR counts as stale,
    whether it has a component or not: the first side's build change is
    taken as staling whatever the other side holds (one compare, no
    second read) — rare, and a refresh that rewrites nothing pushes git's
    merge alone.

## Rehearsal (scratch copy of the product, main_agent `e13063e`)

- Upgrade alone (new `server.js`, `build.mjs` unchanged): 12/12 `builtFrom`
  → 0/12, then `sh mock/build.sh` (what the upgrade now runs, 12 s) →
  12/12. 12 bundles and `pages.json` rewritten.
- Startup check on the real pages: no problem, 81 fetches (one probe per
  declared id). A `mock/handlers/table.startCombat.js` beside the legacy
  key: `table.startCombat is defined twice: … — keep one` at startup, every
  call 500.
- Split: 9 reads and 45 commands, giving 54 handler files plus
  `_lib/reads.js` and `_lib/commands.js`. 108 answers checked. 0.5 s. The
  gate is green before and after (54 ids). The split moves every digest
  (the legacy files leave it): `sh mock/build.sh` rewrites `pages.json`
  only, no bundle changes, 12/12. A second split: "nothing to split".
- The split commit passes `on_conflict`'s check: one parent; only
  `mock/handlers/**`, the two legacy files and `mock/pages.json`.
- Earlier round, branch rehearsal: a milestone changes `table.moveToken`
  and adds `ms.newOne` in the legacy file; main_agent changes
  `soundbox.stopAll` before and after the split. The recipe below merged
  main_agent with **no conflict**: both one-side changes kept, and the new
  handler has its own file. The critique's plain "merge the parent, split,
  merge the base" order gave add/add conflicts on both one-side changes;
  recording the split commit as merged (`-s ours`, then splitting our own
  tree) fixes it.

## Migration recipe (one-off, per product)

0. **Order.** This binary computes the same digests as the product's old
   `build.mjs`, so the watch can restart on it before anything else. Its
   upgrade lands the new `server.js` with rebuilt bundles in one commit on
   main_agent; milestone branches get it through their catch-up (bundles
   and `pages.json` digests are generated files there).
1. **main_agent.** After that upgrade (`server.js`, `split-handlers.mjs`,
   `mock-handlers-gate.mjs`, `mock/handlers/README.md` installed), on a
   branch off main_agent:
   - `node scripts/split-handlers.mjs` (node ≥ 22)
   - `sh mock/build.sh` (digests only: the legacy files leave them)
   - commit only that change, as a PR.

   Wait for green gates (`pages-mocked--bundles`, `mock-handlers`), then
   land it through the normal path, squashed into one commit `SPLIT`.
2. **Each building milestone branch**, before its next catch-up:

   ```
   TIP=$(git rev-parse HEAD)
   SPLIT=$(git log -1 --format=%H --diff-filter=D origin/main_agent -- mock/src/commands.js mock/src/reads.js)
   git rev-list --parents -n 1 "$SPLIT"         # exactly one parent
   git diff --name-only "$SPLIT^" "$SPLIT"     # only mock/handlers/**, mock/src/{reads,commands}.js, mock/components/**, mock/pages.json
   git merge "$SPLIT^"                         # old layout; regenerate generated files as usual
   git merge -s ours --no-commit "$SPLIT"      # record the split as merged...
   git checkout "$SPLIT" -- scripts/split-handlers.mjs mock/src/server.js
   node scripts/split-handlers.mjs             # ...while splitting our own tree (deterministic: the true merge)
   sh mock/build.sh
   git add -A && git commit -m "merge the handler split into <branch>"
   git merge origin/main_agent                 # conflicts only where both sides changed one handler
   sh mock/build.sh                            # build last
   ```

   `-s ours` drops whatever else `SPLIT` changed, so the two checks come
   first: stop if either fails.
3. **Open task PRs** still editing a legacy file meet a modify/delete
   conflict once their milestone migrated. `pr_fix`'s conflict block gives
   the same recipe against their milestone (`mock_handlers::on_conflict`).
   Expect **add/add** conflicts on `mock/handlers/<id>.js` for a handler
   the milestone added to its legacy file before step 2: that file is in no
   merge base. Rule: compare both sides with `<id>`'s handler in the legacy
   file at `git merge-base "$TIP" origin/<milestone>`, keep the side that
   changed it, merge by hand only when both did. Never switch to a
   `--first-parent` lookup of SPLIT: when the deletion reached the
   milestone through a clean catch-up merge, `-s ours` on that merge drops
   everything else main_agent brought.
4. **Open task PRs that add a legacy handler** after the split landed on
   their base go red on `mock-handlers` ("a new handler goes in its own
   file"): `pr_fix` moves each one into `mock/handlers/<id>.js` (the rule
   it carries).

## To check live

- [x] The upgrade at the watch's start rewrites `server.js` (and the
      scripts, `gates.yml`, the README) on main_agent **and** runs
      `sh mock/build.sh` in the install clone: the same commit carries the
      12 bundles and `pages.json`, and `pages-mocked--bundles` stays green
      on main_agent and on the next task PR. The watch log says
      "rebuilt the mock-up's bundles and their digests". (seen 2026-10-24 20:40 Paris: `upgrade: installed 8 file(s) on main_agent` and `rebuilt the mock-up's bundles and their digests: mock/build.sh, in the same commit`; 19dfbe5 carries the 14 bundles and pages.json; node-gates (pages-mocked--bundles, mock-handlers) green on main_agent and on the next task PRs #516, #526–#529)
- [x] The install clone has the node the build needs (`npx esbuild`): if
      the upgrade fails there, the watch log says
      `sh mock/build.sh failed in the install clone` and nothing is pushed. (seen 2026-10-24 20:40 Paris: `sh mock/build.sh` ran in the install clone (bundles in 19dfbe5); the failure line not exercised)
- [x] Each milestone's next catch-up after the upgrade (round D): no
      `Merge <sha> into milestone/N-…` from GitHub, but `milestone/N-…
      holds main_agent: mock/components/X.js and … regenerated` and a
      `chore: sh mock/build.sh after merging main_agent (<sha>) …` commit
      on the branch, one milestone a tick. `pages-mocked--bundles` green on
      the pushed branch, including a component only the milestone has. (seen 2026-10-24 20:50–20:54 Paris: #28, #24, #29 one a tick (`its local catch-up waits for the next tick`), `holds main_agent: mock/components/… regenerated`, #29 with ElementsPanel/TableSheet that only it has; node-gates green on 45cef01 and ee39bcc. The commit is one `chore: merge main_agent (<sha>) into <branch>, … regenerated`, not git's merge plus `chore: sh mock/build.sh after merging …`)
- [x] GitHub's compare answers for a branch name with a slash
      (`compare/milestone/N-…...<sha>`): the watch log shows no
      `what main_agent changed since it cannot be read` debug line. If it
      does, every catch-up goes local (still correct, one a tick). (seen 2026-10-25 01:40 Paris: catch-ups of 10-24 18:50–20:14Z on milestone/N-… branches, 0 `cannot be read` lines in watch.log)
- [x] A `main_agent` push that changes no file of the mock-up's build
      (a handler file, a Rust source) still catches up on GitHub: `Merge
      <sha> into milestone/N-…`, no `catch-up` checkout mounted. (seen 2026-10-24 21:21 Paris: after rules PR #518 landed, GitHub committed `Merge 19262b9 into milestone/28-…` and `… into milestone/24-…`, `holds main_agent`, no catch-up checkout mounted)
- [ ] A task PR whose component exists only on it, after such a
      catch-up: the watch says `task PR(s) #n take it when they land`, the
      PR keeps its old base and stays green; no `pr_fix` session starts.
- [x] Its landing: `watch: milestone_merge -> #<task> waits: PR #n took
      milestone/N-… in the harness's checkout, its mock-up bundles
      rebuilt`, a `chore: sh mock/build.sh after merging milestone/N-… (<sha>)
      into <task branch>` commit on the PR, CI reruns green, the next
      attempt merges it, and `pages-mocked--bundles` stays green on the
      milestone after. (seen 2026-10-24 21:37 Paris: `milestone_merge -> #486 waits: PR #516 took milestone/24-… in the harness's checkout, its mock-up bundles rebuilt`; the PR got `Merge commit '2cb67f9' into feat/scr…` (not the `chore: sh mock/build.sh after merging …` wording), CI green, merged 19:51Z by milestone_merge; node-gates green on milestone/24 after (PR #519))
- [ ] A task PR skipped as `Busy` during that catch-up (its session still
      running), green on its old base: it never lands with the old digest —
      the merge train refreshes it first, same lines as above.
- [ ] Reverse side: a milestone whose own task changed `mock/src/screen.js`,
      catching up after `main_agent` landed another milestone's components:
      local catch-up with `sh mock/build.sh`, no GitHub merge, its
      `pages-mocked--bundles` green after.
- [x] GitHub's compare with a sha as base and a branch as head
      (`compare/<sha>...milestone/N-…`, the reverse read): no `cannot be
      read` debug line in the watch log. (seen 2026-10-24 22:13 Paris: catch-ups after #519 brought #24's bundles, 0 `cannot be read` lines in watch.log)
- [x] The conformance check (`ArchitectureHolds`) reports no stale bundle
      on a milestone branch the upgrade has not reached yet (old `server.js`,
      old digests). (seen 2026-10-24 20:55 Paris: #486's round mounted milestone/24 at 18:45Z, before its catch-up (18:52Z, old server.js and digests): `ArchitectureHolds: pass`)
- [ ] Open PRs adding a handler to `mock/src/commands.js` stay green after
      the upgrade (only the README in `mock/handlers/`), and go red only
      once the split landed on their base.
- [x] Step 1 on the real main_agent: 54 handler files + 2 `_lib`, the
      self-check passes, `pages.json` digests rewritten (no bundle change),
      `pages-mocked--bundles` and `mock-handlers` green, the PR lands. (seen 2026-10-24 23:10 Paris: PR #530 (squash a55bd4d): 61 handler files + `_lib/{reads,commands}.js` (more ids than the rehearsal's 54), legacy files deleted, pages.json rewritten, no bundle changed; node-gates and every check green; merged)
- [ ] The view's easel on main_agent: every mocked page behaves as before.
      Commands apply, reads answer, an unknown DataCapability is rejected
      with "has no handler: write mock/handlers/<id>.js". The only console
      errors are 404s (one probe per declared id without a file).
- [ ] Step 2 on each building milestone: no handler conflict unless both
      sides changed one; the gate is green on its landing PR.
- [ ] A task PR opened before the migration goes through `pr_fix` with
      the recipe and comes back green; an add/add on a handler its
      milestone added before migrating is resolved by the merge-base rule.
- [ ] After a week: no catch-up residual on `mock/src/commands.js`, and no
      `pages.json` digest conflict caused by a handler.
- [x] A session building a Capability/DataCapability writes
      `mock/handlers/<id>.js` (its narrowed checkout has it) and nothing in
      `mock/src/reads.js` / `commands.js`. (seen 2026-10-24 22:33 Paris: PR #526 (`live.listSuggestions`, a session after the upgrade) adds `mock/handlers/live.listSuggestions.js`, nothing in the legacy files, node-gates green; NO_NARROW is on, so the narrowed checkout was not exercised)
