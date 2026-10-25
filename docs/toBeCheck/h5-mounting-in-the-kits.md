# H5 v1: a rule's test is checked for its mounting at its first real run

Built 2026-10-25 on lane B (`lane-b/h3-h5-h7`, on f58d1d6 + H3). Not
committed yet, not yet seen on a real product. Design, critique and
decision: `harness-dnd2-run/night/h5-design.md`, section *Decision*
(the critique's simpler alternative; no mock rehearsal, no new CI step).

## What shipped

- **Business kit.** `tests/rules/business/src/mounting.rs` (new asset, std
  and serde_json only), wired into `Server::read`/`command` of the kit's
  `lib.rs`:
  - a unit reached that the test's needs do not name → refused before
    sending (`reaches capability:<id>, which its needs do not name: …`);
    the version is no part of a need;
  - a read's variable its `capability.json` `input` does not take →
    refused before sending (`<id> has no input "<key>"; its capability.json
    takes <keys> (<path>)`); a dotted input declares its head; no `input`
    object, nothing judged;
  - a command's answer carrying the engine's `payload key(s) … not declared
    by …` warning;
  - any of them: the rule is recorded `mis-mounted` (with every line, joined
    by ` · `) and the test **passes**, whatever its body answered.
  - `record` now writes each status line in one `write_all`: the suite's
    parallel tests interleaved their lines before (seen in the scratch run:
    two lines merged into one unparsable line).
  - the manifests are indexed once per process (`OnceLock`), as `kit.ts`
    does: a polling test (r114, r18, r26) no longer walks the tree per read.
- **Journey kit.** `kit.ts` looks at every `POST /capabilities/<id>` and
  every screen route (RULES_SCREEN_ROUTE) its pages send, then lets them go:
  - a screen its needs do not name, or a unit none of its needs covers
    (named, or in the `needs`/`actions` of a Micro-UI it needs or one its
    screens lay out) → annotation `mis-mounted`, never thrown;
  - a page's read off its `input` → red, `variables off the manifest — fix
    the read, this is not a rule contest: …` (the Micro-UI's, like H3's
    payload);
  - a screen counts as opened only by a valid name (`^[a-z][a-z0-9-]*$`,
    contract E) the shell answers: a link like `/screens/x.html` (the
    built Micro-UIs' `dm-view.html`, `preparation-seance.html`,
    `fiche-entite.html?new=…`, `medias.html`) or one answered 404 is red,
    `a link to <url>, which the shell does not serve` — the product's;
  - `status-reporter.ts` writes `mis-mounted` whatever the outcome.
- **Gate.** `scripts/rules-gate.mjs status` says the mis-mounted rules apart,
  after every other rule, with what to do, each with its rule's red tests
  listed under it (the product's, fixed in the same pass), and fails on
  every one, `conform` or not, `--strict` or not: a mounting defect is never
  waived. `rules-status.json` carries the state `mis-mounted`; the view
  shows a red `mis-mounted` tag (its tooltip: the mis-mounted test's why,
  not a sibling's red) and counts them.
- **Prompts.** Rules WRITE: needs by `<system>.<name>` id; step 3 fixes a
  mis-mounted test before anything else. CONTEST: a mis-mounted test is
  fixed first (answer a, never c). FIDELITY check 3: ids, and no rule
  mis-mounted on `rules-hold`; WRITE says mis-mounted shows only on the
  PR's `rules-hold--status` step (nothing is served there). Dev CODE (both):
  `rm -f target/rules-status.jsonl` before the suites, `node
  scripts/rules-gate.mjs status` after them, before the stop; a mis-mounted
  rule is copied into `CONTEST.md` as it is, its red tests fixed. `pr_fix`:
  the same copy, nothing changed for it.
- **Docs.** Contract P in the installed `docs/ARCHITECTURE.md` (ids in the
  example, the mis-mounted bullet), `crates/architecture/ARCHITECTURE.md`
  (decision, tests), `crates/workflows/ARCHITECTURE.md` (the rules' task).
- **Seen in a scratch product, for real** (not CI): the kit's `lib.rs` +
  `mounting.rs` built with ureq 2.12.1 against a node stub server, 6 tests:
  1 green, 1 red, 4 mis-mounted (unnamed need, variable off input, payload
  warning, a mis-mount the body swallowed with `let _ =`), 5 runs, 30 lines,
  0 unparsable; `rules-gate.mjs status` exit 1, the 4 said apart. The
  journey kit on Playwright 1.64.1 + chromium 1248, 4 specs: mounted green,
  another screen mis-mounted (passes), a read off its input red, mis-mounted
  with a failing body still `mis-mounted`. `tsc --strict --noUnusedLocals
  --noUnusedParameters` passes on `kit.ts` and `status-reporter.ts`.

## To check for real

- [ ] Product upgrade (the watch's start runs `init-repo` upgrade):
      `scripts/rules-gate.mjs`, `tests/rules/journey/kit.ts`,
      `tests/rules/journey/status-reporter.ts` and `docs/ARCHITECTURE.md`
      are *upgraded* (gate and reporter byte-identical to the harness's at
      product a55bd4d); `tests/rules/business/src/mounting.rs` is *new*.
- [ ] `tests/rules/business/src/lib.rs` has diverged in the product (`dm()`
      caller, `then` follow-ups), so the upgrade leaves it as it is. The
      upgrade commit is the watch's own (made and pushed by `init-repo` at
      the watch's start), and a later PR touching `tests/rules/` is refused
      by `rule-tests-untouched` unless labelled `harness:rules`. Route: a
      supervisor commit straight to `main_agent` right after the upgrade
      push, or a PR labelled `harness:rules`.
      The hook: `mod mounting;`, the two `Server` fields and `mounted`, the
      checks at the top of `read` and `command` and after `accepted`, the
      `mis_mounted` check before `match outcome` in `rule`, `record`'s
      `write_all`, and the `OnceLock` index. Verified on a scratch copy:
      `patch` applies 8 of 9 hunks; place the one rejected by hand —
      `self.mounted(mounting::off_payload(&accepted))?;` after `accepted`,
      before the product's `self.settle(...)`. Then it compiles, 30 tests;
      against a stub server 22 red (stub answers), 8 pending, 0
      mis-mounted. `common::read_reopened` builds its own request and stays
      unchecked: note it, do not widen the hook.
- [ ] R6's undeclared need (`capability:live.getTranscriptionVocabulary`)
      fixed through a `harness:rules` issue at rollout, before the first
      `rules-hold` (the decision's rollout step).
- [ ] First `rules-hold` on `main_agent` after the upgrade. Static estimate
      at a55bd4d (every literal read and every helper's read and command of
      `common/mod.rs` against the needs and the manifests): **0 business
      test mis-mounted** out of 30 (22 active), **0 journey opening a screen
      off its needs** out of 22. Each journey `reaches … none of its needs
      covers` that shows is a Micro-UI manifest missing a `needs`/`actions`
      entry, not a test: count them (expected 0; above 2, the coverage rule
      is too strict — say so before routing them to the rules' task).
- [ ] 0 `variables off the manifest` red on the journeys of `main_agent`;
      each that shows becomes one `harness:agent` fix task on its Micro-UI.
- [ ] Replay the pre-fix tests from the product's history on a scratch
      branch, with their units built, through `scripts/rules-runtime.sh
      serve`, the business suite, then `status`:
      - R9 and R12 before e13063e (#470, `{ "id": … }`): both
        `mis-mounted — campaign.listEntityVersions has no input "id"; its
        capability.json takes entity (…)`;
      - the tests #439 (520d50c) and #424 (c77461c) fixed: **red as
        before**, not mis-mounted — the needs they added came in the same
        commit as the new helper calls (`new_battlemap`, `new_cast`,
        `new_illustration`), so the pre-fix tests never reached those units
        (a 422 on bm-gue, slug 404s, the `init` response key, the portrait
        type);
      - R101 before 520d50c (`entity.portrait: true`): **not caught**
        (a nested type, not a key) — expected red, as before.
      Expect R9 and R12 mis-mounted, the #439/#424 tests red. Of the 8
      contests of 2026-10-23/24 the checks catch **1** (#470): #439's
      `init` is a key read from the response, not a payload key; #430,
      #414/#417, #517, #488/#494 and #499 are out of reach; #459 is moot
      (D8 renames on `/commands`).
- [ ] A dev PR meeting a mis-mounted rule: its session (or `pr_fix`) writes
      `CONTEST.md` with the gate's line and changes nothing else; the
      `Rules contested` issue opens; the rules' session fixes the test
      (answer a) without weighing a or b (its transcript).
- [ ] A rules session that writes a new test lists ids (`<system>.<name>`),
      never short names (its PR diff).
- [ ] Over the next 2 days: mis-mounted contests still open (a dev copies
      the line into `CONTEST.md`, a `Rules contested` issue opens), but
      each is settled by answer (a) with **no dev diagnosis turns**.
      Measure, per mis-mounted contest, the dev session's turns spent on it
      (expected 0 beyond the copy) and the time from the PR's red to the
      settled contest. Baseline of that class: #470 only.
