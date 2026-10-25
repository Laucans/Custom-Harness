# H3 v1: a screen sends only what its manifest declares

Built 2026-10-24 on lane B (`lane-b/h3-h5-h7`, on da531e2). Not committed
yet, not yet seen on a real product. Design and decision:
`harness-dnd2-run/night/h3-design.md`, section *Decision*.

## What shipped

- **The journey net.** The rules' kit (`tests/rules/journey/kit.ts`) fails a
  running journey, once its body ran, when a `POST /commands` the real
  server answered carries the engine's warning `payload key(s) <keys> not
  declared by <dataCapability>`. The first line of the failure reads
  `payload off the manifest — fix the payload, this is not a rule contest:
  …`. A pending test still opens no browser. The kit answers every page's
  `POST /commands` itself (`route.fetch`, read, `route.fulfill`), in the
  default context and in every `browser.newContext()` (so `browser.newPage()`
  too); an init script counts each page's commands in flight, and a page or
  context close, and the kit's teardown (it takes `page`, so it ends before
  the page closes), wait for them.
- **Review fixes (same lot).** Seen on real Playwright 1.64.1 + chromium
  1248 against a stub server answering after 0, 50 and 300 ms, 5 repeats
  each, in a scratch setup: an unawaited last command, a second context
  closed at once, `page.context().close()`, `browser.newPage()`,
  `page.close()` and an XHR then close all go red; declared keys and a
  `GET /commands` stay green; 0 mismatch in 80 runs. Before the fix a
  close right after the command was green (the request never reached the
  route). `tsc --strict --noUnusedParameters` passes on `kit.ts`. Not yet
  run on the product's real runtime.
- **CI pins node 22** (`actions/setup-node@v5` in the `ci` job):
  `journey_kit.rs` needs node 22.6+ to strip TypeScript, and is red, never
  skipped, when `CI` is set and no node does. ubuntu-latest ships node 20.x
  by default, so before the pin it skipped there.
- **The mock net.** `mock/src/server.js` runs the engine's own check
  (`undeclared_keys`): top-level keys, the target's key exempt, an empty
  payload unchecked, `@version` ignored. It answers the same warning, never
  a refusal, says it in `console.error` and keeps it in `server.undeclared`.
  A replay now answers no warning, as the engine does.
- **The view serves what the mock reads.** `GET /mock/<slug>/__product/…`
  serves `plan/graph.json`, any `crates/*/data-capabilities/*/data-capability.json`,
  and `data-capabilities.json`, which indexes every manifest by its
  `dataCapability` field. It serves nothing else, and reads from the
  branch's workspace when one exists. The mock reads `plan/graph.json` from
  there first, so `planned()` now works in the mock-up pane.
- Docs: contract H in `docs/DATA-INTEGRITY.md` (asset), the decision in
  `crates/architecture/ARCHITECTURE.md`, the route in `crates/view/ARCHITECTURE.md`.

## To check for real

- [ ] Product upgrade (the watch's start runs `init-repo` upgrade):
      `kit.ts`, `mock/src/server.js` and `docs/DATA-INTEGRITY.md` are
      *upgraded*, not *diverged* (both were byte-identical to the harness's).
      `server.js` stales every bundle, so the upgrade rebuilds them in its own
      commit, and `pages-mocked` stays green.
- [ ] Mock-up pane on `main_agent`:
      `/mock/main_agent/__product/data-capabilities.json` lists every manifest (50 at product a55bd4d)
      and an empty `problems`. `/mock/main_agent/__product/plan/graph.json`
      answers 200. `/mock/main_agent/__product/Cargo.toml` answers 404.
- [ ] Mock-up pane, a Battlemap gesture: the browser console shows no
      `payload key(s)` line. Then, on a scratch branch reverting c48632b's
      Battlemap `index.ts` with its bundle rebuilt, it shows exactly 4
      warnings, every gesture still applied:
      `payload key(s) conditions not declared by table.setTokenCondition`,
      `payload key(s) position, token not declared by table.reorderInitiative`,
      `payload key(s) token not declared by table.removeParticipant`,
      `payload key(s) roll not declared by table.addParticipant`.
- [ ] A planned DataCapability with no manifest (e.g. `live.setMicrophone`)
      is applied as `mocked-data-capability` in the pane. Before H3 the graph
      404'd there and it was rejected `unhandled-data-capability`.
- [ ] Journeys on the real runtime (`scripts/rules-runtime.sh serve`, then
      the `rules-hold--journey` gate) on `main_agent`: 0 `payload off the
      manifest` failure. At product a55bd4d every Battlemap journey (R15,
      R17, R19, R23, R24, R98, R106, R109) is PENDING, `waiting for
      screen:dm-view…`, so this proves nothing for the Battlemap yet. Each
      off-manifest failure that does show becomes one `harness:agent` fix
      task on its Micro-UI.
- [ ] Until `screen:dm-view` is built, prove the net on the real runtime
      with a scratch spec (never committed): `rule('Z', [], …)` that
      POSTs from the page, through `apps/shell`, to `/commands`:
      `ai.testConnection` with an extra key goes red with "not a rule
      contest" and the key named; the same with `@1`; declared keys only
      stay green. Add one that fires the command unawaited as its last
      gesture, and one from a `browser.newContext()` it closes at once:
      both red.
- [ ] Once `screen:dm-view` is built: the scratch branch reverting c48632b
      on the real runtime turns red every Battlemap journey that drives one
      of those 4 commands, the gate's `why` naming the warnings above (4
      warnings, reorderInitiative's with two keys), with "not a rule
      contest". R23 included: it closes its context mid-test and sends
      `table.resetBattlemap` from a second one.
- [ ] First harness CI run after the merge: the `ci` job's setup-node step
      logs node 22.x, and `journey_kit` passes (it cannot skip with `CI`
      set).
- [ ] A rules session that meets such a red journey does not contest it
      (`CONTEST.md` untouched); the development fixes the payload.
- [ ] `entity.features` (EntityEditor, `campaign.saveEntity` v2): see whether
      the real server warns. Top-level `entity` is declared, so the expected
      answer is no warning.
