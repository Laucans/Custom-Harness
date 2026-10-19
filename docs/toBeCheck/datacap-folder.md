# A DataCapability's folder is its id's namespace — never `crates/dataguard/`

## Context

The installed document (`assets/ARCHITECTURE.md`, *DataCapability*) says a
DataCapability's system is its id's namespace: `campaign.saveEntity` lives
under `crates/campaign/data-capabilities/save-entity/`. On ForgeDesMondes the
`ai` system followed it (`crates/ai/data-capabilities/save-settings`,
`test-connection`), but milestone 18 laid its three `campaign`
DataCapabilities (`save-entity`, `restore-entity-version`, `export-campaign`)
under `crates/dataguard/data-capabilities/`: the split had written
`system: dataguard` on #51–#53, and `tree.rs` had been widened (1dcadc1) to
read both folders. Two conventions coexisted and no gate said so. Owner's
ruling (2026-10-18): a DataCapability's system = its id's namespace; the
DataGuard owns every DataCapability (`owner: dataguard`) and houses none.

## What shipped

- 9a1786f — the rule, in every place that reads or writes the layout:
  - `tree::violations` refuses a `data-capability.json` whose folder is not
    its id's namespace, naming the folder it belongs in (`… belongs under
    crates/campaign/data-capabilities/save-entity/, its id's namespace …`),
    and an id whose namespace is `dataguard` or missing;
    `Tree::read` still reads `crates/dataguard/data-capabilities/` so the
    rule can name it.
  - `assets/gates.yml`: the job `data-capability-placed`, mirrored in
    `tests/gates_parity.rs` (healthy passes both, a manifest under the
    DataGuard fails both); the DataCapability rule card names it.
  - `assets/isolation.sh` (comment), `assets/contracts/README.md`,
    `assets/CLAUDE-rules.md` (one word: *under its id's system*, no version
    bump), the script fixtures moved to `crates/credit/data-capabilities/`.
  - The split: the slice prompt names the folder from the id;
    `plan::spec_gaps` refuses a slice whose `brief` or `technical` places a
    DataCapability under another system's or the DataGuard's folder.
  - The refinement's `layout_violation` asks for
    `crates/<namespace>/data-capabilities/<name>/` and never accepts
    `crates/dataguard/data-capabilities/`.
  - The closure's fixtures: the unit resolves at its system's folder with
    that system's plumbing, nothing under `crates/dataguard/data-capabilities`.
- ForgeDesMondes PR [#267](https://github.com/Laucans/ForgeDesMondes/pull/267) (branch
  `chore/datacapabilities-under-their-system`, against milestone/18,
  label `harness:to-review`): the three crates moved with `git mv`, the
  server's three path dependencies, `contracts/README.md` and `CLAUDE.md`
  line 38. `cargo check --workspace --all-targets` (toolchain 1.99.1) and
  `scripts/isolation.sh` green locally; the new gate's script passes on the
  moved layout and lists the three manifests on the original.

## To verify live

- [ ] The product PR's CI green (`gates`, `ci`, `test-scope`), then merged by
      the supervising session; `git ls-files crates/dataguard/data-capabilities`
      empty on milestone/18.
- [ ] The next `dev_loop` round on milestone/18 after the merge: its
      architecture check (`tree::violations`) lists no `belongs under` line
      (expected 0); `Tree::read` counts 5 DataCapabilities (2 `ai`, 3
      `campaign`), the Micro-UIs' `actions` still resolve (no `triggers …
      no manifest declares`).
- [ ] The product's `.github/workflows/gates.yml` does not carry
      `data-capability-placed` until `init-repo` reinstalls it (the asset is
      not overwritten by a plain install): check what the reinstall path does
      with `gates.yml`, and that the job is green on milestone/18 once there.
- [ ] A split of a milestone with a DataCapability: every slice's
      `technical` names `crates/<namespace>/data-capabilities/<name>/`; a
      reply naming `crates/dataguard/data-capabilities/` is refused by
      `SlicesCarryASpec` with `places … under crates/dataguard/…` and
      retried once.
- [ ] A refinement's technical section naming the old folder stops with
      `places none at crates/<namespace>/data-capabilities/<name>/`.
- [ ] The closure of a `campaign` DataCapability task (`harness trial` or the
      next such task): the sparse patterns carry
      `/crates/campaign/data-capabilities/<name>/`, `/crates/dataguard/server/`
      and its aggregate, nothing under `crates/dataguard/data-capabilities/`.
- [ ] `CLAUDE-rules.md` v4 was edited without a version bump: existing
      products keep the old wording (*system `dataguard`*) until the owner
      decides a reinstall; ForgeDesMondes' copy is fixed by the PR.
