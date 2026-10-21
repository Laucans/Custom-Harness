# Contracts

The eighteen interface contracts of the agent-native architecture
(`docs/ARCHITECTURE.md`, "Interface contracts"), as JSON Schema draft 2020-12.
Installed by `harness init-repo`; the repository owns them from here. A change
to one of them is its own task, and lists the units it touches.

| File | Contract | Validates |
|---|---|---|
| `a-concept.schema.json` | A. Concept | `concepts/<Concept>/v<n>/concept.json` |
| `b-capability-manifest.schema.json` | B. Capability manifest | `crates/<system>/capabilities/<name>/capability.json` |
| `c-persisted-query.schema.json` | C. Persisted query registry | `queries/registry.json` |
| `d-micro-ui-manifest.schema.json` | D. Micro-UI manifest | `apps/<system>/<micro-ui>/micro-ui.json` |
| `e-screen-composition.schema.json` | E. Screen composition | `apps/screens/<screen>.json` |
| `f-data-capability.schema.json` | F. DataCapability | `crates/<system>/data-capabilities/<name>/data-capability.json` |
| `g-queue-entry.schema.json` | G. DataQueue entry | what the queue stores and shows an author |
| `h-command-result.schema.json` | H. Command result | what the DataGuard answers a caller |
| `i-aggregate.schema.json` | I. Aggregate | `crates/<system>/aggregates/<Aggregate>/aggregate.json` |
| `j-impact-plan.schema.json` | J. Impact plan | what the Resolver hands the DataGuard |
| `k-reconciliation.schema.json` | K. Reconciliation proposal | what the reconciliation AI hands an approver |
| `l-message.schema.json` | L. Message | what the Integrity DataGuard sends |
| `m-data-model.schema.json` | M. Data model | `data/model.json`: what every table and field is for |
| `n-mock-pages.schema.json` | N. Mock pages | `mock/pages.json`: the pages of the mock-up, one per screen |
| `o-design-kit.schema.json` | O. Design system and kit | `mock/design/design.json` and `mock/kit/kit.json` on the `design-system` branch: the tokens' record, the kit's components |
| `p-rules.schema.json` | P. Rules | `rules.json`: the rules the product keeps, each with the criterion its tests prove |
| `q-ports.schema.json` | Q. Ports | `ports.json`: every dependency outside the product, behind a port, with its provider, adapter, fake, secrets, cost and users |
| `r-plan-graph.schema.json` | R. Plan graph | `plan/graph.json`: what every milestone produces and consumes, and the dependencies that follow — the critical path, what may run side by side, and the register of contracts a consumer builds against before they are built |

Validate a manifest by hand the way the `gates` workflow does:

```sh
npx --yes ajv-cli@5 validate --spec=draft2020 \
  -s contracts/b-capability-manifest.schema.json \
  -d crates/credit/capabilities/calculate-risk/capability.json
```

Contracts A, B, F and I fix the frontier between reading and writing: they are
the first to stabilise, and the ones a change to is reviewed hardest.
