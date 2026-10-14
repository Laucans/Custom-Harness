# The design dossier leaves the factory whole

## Context

The creation designed more than it handed over: the aggregates were drafted
in a shape contract I refuses, the audit's write effects and the data step's
touched fields were lost at Initialize, fragments did not have to name their
parts, and fixtures were named after the micro-frontend while the mock server
and the rules' tests read one per Capability. Milestone M1 of the plan
"autonomous development from the design dossier".

## What shipped

- One `mock/data/aggregates/<Aggregate>.aggregate.json` per aggregate,
  conforming to contract I (`DataDraft::aggregate_manifests`): fields in
  camelCase, the table joined in on a clash; kebab invariant ids; relations
  `<Aggregate>.<field>`; `holds` 15m renewable.
- `mock/proof-of-concept.json` gains `writes` (`dataCapability`, `effect`,
  `touches`) and `aggregates`; components carry `fragment`, `fixtures`, `props`.
- Fixtures: one per Capability, `mock/fixtures/<Capability>.json`, the only
  form contract N accepts (`fixtures` list; `fixture` refused). The
  micro-frontend generation reads and answers them so; componentize and the
  mock-up sessions are told so.
- A fragment that names no part for one of its needs or actions blocks the
  Rules step's validation (`audit::unnamed_parts`, finding `part_unnamed`).
- Gates: `manifests-conform-to-contracts` validates the aggregate drafts;
  `pages-mocked` checks every hand-written micro-frontend's parts and fixtures
  before any Micro-UI is built.

## To check on a real creation

- [ ] Componentize writes one fixture per Capability and `fixtures: [...]` in
      `mock/pages.json`; the fragments fetch `../fixtures/<Capability>.json`
      and show their data on the easel.
- [ ] Every fragment carries `data-reads` / `data-writes` for each need and
      action. Otherwise the Rules step refuses validation, naming the
      micro-frontend.
- [ ] Initialize writes `mock/data/aggregates/*.aggregate.json`;
      `npx ajv-cli@5 validate --spec=draft2020 -s contracts/i-aggregate.schema.json -d 'mock/data/aggregates/*.aggregate.json'`
      passes.
- [ ] `mock/proof-of-concept.json` lists `writes` with the audit's effects
      (when the audit ran) and the data step's `touches`.
- [ ] The gates `manifests-conform-to-contracts` and `pages-mocked` are
      green on the integration branch right after Initialize.
- [ ] An existing factory's mock-up with `fixture` (single) is refused by
      `manifests-conform-to-contracts`. Re-componentizing fixes it.
