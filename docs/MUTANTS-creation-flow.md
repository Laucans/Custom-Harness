# cargo mutants — creation flow rework

## Status: closed (2026-10-13, second run)

After the tests below were written: **97 mutants · 96 caught · 0 missed · 0 timeout · 1 unviable → gate green (exit 0).**
Tests added (8): `the_preparation_draws_the_tokens_only_from_some_material_and_only_once`,
`the_boards_of_a_branchs_mock_up_share_a_prefix_named_after_it` (+ a foreign-board pin in the `Pins` fake and a
`design.components` assertion in the main flow test) in `creation_desk.rs`; `the_requirements_prompt_lists_…`,
`two_requirements_sent_with_the_same_id_keep_the_first` in `domain/creation.rs`;
`the_preparation_starts_from_the_brief_…`, `the_micro_frontends_and_the_requirements_wait_for_the_mock_up` and a
`/viewport.js` assertion in `server/tests.rs`.

The report below is the first run, kept as the record of what was missing.

---

Run: `cargo mutants --workspace --in-diff change.diff -j 4` (2026-10-13), diff = `git diff main` on the
`.rs` files of the working tree (uncommitted). cargo-mutants 27.1.0, config `.cargo/mutants.toml`.

**97 mutants · 76 caught · 20 missed · 1 unviable · 0 timeout → CI gate would be RED (exit 2).**

Per file: `creation_desk.rs` 37, `domain/creation.rs` 53, `server/creation.rs` 4, `server/mod.rs` 2,
`kit_desk.rs` 1 (the last, `viewport_js` and `kit_desk.rs` come from changes already in the tree before this work).

## Missed mutants (20) — what no test pins down

### 1. `CreationDesk::prepared` — the background preparation (11 missed)
`creation_desk.rs:336-347`. The test `preparing_proposes_a_first_scope…` runs with a kit whose `oneshot`
is `None`, so only the `no_claude` branch is ever taken. Never exercised:
- `material` (moodboard / logo / design brief → `||`, `!`) : lines 336-338
- `drawn` (tokens already there → `!=`, `||`) : line 339
- the `!material` branch (342)
- the "queue the kit once tokens exist" branch and its `queued > 0` message (347, three comparison mutants)

**Test to add:** a kit with a Claude (`Styling`-like fake) and three cases — no material (steps say tokens
wait), material + no tokens (tokens drawn, kit queued), tokens already drawn (kit queued, no draw).
Assert on `job.steps` and on the kit's `reply()` state.

### 2. `CreationDesk::boards_of` (1 missed, `-> String::new()`)
`creation_desk.rs:736`. The prefix that selects the mock-up's pin boards (`mockup--<branch>--`).
The test's `Pins` fake returns `mockup--main-agent--home` for any prefix it is asked, and `context()` filters with
`starts_with("")` too — an empty prefix still passes.
**Test to add:** a second board (`mockup--other-branch--home`, or `data-model`) carrying a request pin;
assert it is *absent* from the record's `pins`. Also a unit test of `boards_of("main_agent")`,
`boards_of("feature/x")` and `boards_of("")` (→ `mockup--integration--`).

### 3. `CreationDesk::context` — kit components (1 missed, `==` → `!=` line 829)
The design system's `components` (generated kit components) are never asserted: the test's kit has no Claude and
no generated component.
**Test to add:** with a generated component in the kit, assert `kept["design"]["components"]` lists it and
not the pending ones.

### 4. `Creation::edit` — requirement ids (1 missed, `==` → `!=` line 322)
`domain/creation.rs:322`: the duplicate-id guard (`kept.iter().any(|k| k.id == r.id)`).
**Test to add:** `Edit` with two requirements sharing an id → only the first is kept.

### 5. `requirements_prompt` (2 missed, `delete !` lines 1046, 1058)
The pages and micro-frontends sections of the prompt are only checked with an empty `Context`.
**Test to add:** with a filled `Context`, assert the prompt contains each page path, objective, component
name, `needs` and `actions`; with an empty one, assert the sections are absent.

### 6. HTTP handlers `prepare`, `componentize`, `requirements` (3 missed, `-> Default::default()`)
`server/creation.rs:157, 174, 190`. No server test goes through `POST /api/creation/{prepare,componentize,requirements}`;
the desk is tested, the routes are not (status codes `202` / `400`, JSON body).
**Test to add** (in `server/tests.rs`, with `state_creating` and the `Says` fake): each route answers `202`
with a `job` (or `400` with the gate's reason without a brief / mock-up), `requirements` answers `200` with
`record.requirements`.

### 7. `viewport_js` (1 missed, `server/mod.rs:536`) — not from this work
The script route of `viewport.js` (already in the tree). `the_page_carries_the_creation_script` covers
`creation.js`; add the same assertion for `/viewport.js`.

## Not covered by mutation testing at all
The front-end (`creation.js`, `mockup.js`, `style.css`) is not mutated; the changes there were checked by hand in
the browser (brief + design easel, grilling shortcuts, scope "to redo", review requirements and chips, mock-ups
easel) — no AI call was made. Untested paths: the auto-start of the preparation on *Continue*, the
componentize-on-review-entry and the requirements auto-read, the draft flush (`MockupPane.flush`) on
*Continue* and *Accept*, and the "Draw again" confirmation.

## Next step
Add the tests above (≈ 8 tests: 3 in `creation_desk.rs`, 3 in `domain/creation.rs`, 2 in `server/tests.rs`), then
re-run only the missed mutants:

```
cargo mutants --workspace --in-diff change.diff -j 4 --regex 'prepared|boards_of|context|edit|requirements_prompt|prepare|componentize|requirements|viewport_js'
```
