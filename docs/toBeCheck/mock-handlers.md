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

- [ ] The upgrade at the watch's start rewrites `server.js` (and the
      scripts, `gates.yml`, the README) on main_agent **and** runs
      `sh mock/build.sh` in the install clone: the same commit carries the
      12 bundles and `pages.json`, and `pages-mocked--bundles` stays green
      on main_agent and on the next task PR. The watch log says
      "rebuilt the mock-up's bundles and their digests".
- [ ] The install clone has the node the build needs (`npx esbuild`): if
      the upgrade fails there, the watch log says
      `sh mock/build.sh failed in the install clone` and nothing is pushed.
- [ ] Each milestone's next catch-up after the upgrade: its bundles and
      `pages.json` digests regenerate when they conflict. A component only
      the milestone has (its entry merged without a conflict) keeps a digest
      of the old `server.js`: if `pages-mocked--bundles` goes red there,
      `sh mock/build.sh` on the branch (the catch-up rebuilds on a conflict
      only — a gap older than this round, any `server.js` change has it).
- [ ] The conformance check (`ArchitectureHolds`) reports no stale bundle
      on a milestone branch the upgrade has not reached yet (old `server.js`,
      old digests).
- [ ] Open PRs adding a handler to `mock/src/commands.js` stay green after
      the upgrade (only the README in `mock/handlers/`), and go red only
      once the split landed on their base.
- [ ] Step 1 on the real main_agent: 54 handler files + 2 `_lib`, the
      self-check passes, `pages.json` digests rewritten (no bundle change),
      `pages-mocked--bundles` and `mock-handlers` green, the PR lands.
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
- [ ] A session building a Capability/DataCapability writes
      `mock/handlers/<id>.js` (its narrowed checkout has it) and nothing in
      `mock/src/reads.js` / `commands.js`.
