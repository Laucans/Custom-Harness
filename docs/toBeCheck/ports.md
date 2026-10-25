# Ports: every dependency outside the product

## Context

Business grill of 2026-10-14 (19 answers): every external dependency of a
product sits behind a port named by business function, listed in
`ports.json` (contract Q), visible first and switchable second. Postgres is
listed, not abstracted. Our Data is the truth for stateful providers. A new
port in development needs a human's approval. Secrets are committed with
sops + age. Tests always run on fakes. New initializations only.

## What shipped

- Milestone 1 (`e22c641`):
  - contract Q and `ports` in manifests B and F;
  - `init-repo` installs `ports.json` (the `database` port), `.sops.yaml` and
    `secrets/README.md`;
  - rule 16 and the architecture's "Ports" section.
- Milestone 2: `scripts/ports-gate.mjs` with `declared`, `contracts`,
  `behind` and `secrets`.
  - The gates are `ports-declared`, `externals-behind-ports` and
    `secrets-encrypted`, plus contract Q in `manifests-conform-to-contracts`.
  - The database client is allowed in `crates/data` (the `database` port's
    adapter) and `crates/dataguard` only.
  - `rules-runtime.sh` starts the server with `PORTS=fake`.
- Milestone 3: the creation step **Ports**, between Data and Initialize.
  - The AI lists the ports from the brief, decisions, rules and data model,
    proposing a provider and alternatives for each. PostgreSQL always comes
    first.
  - The human edits the cards and validates. Accept needs validated ports.
  - Initialize writes `ports.json` beside the mock-up and opens one
    `harness:human` secrets task per port that has secrets.
  - The roadmap task and the dossier carry the ports.

- Milestone 4: development.
  - The CODE prompts route every external call through a port. A port
    ports.json does not have stops the session with
    `AGENT_LOOP_STOP: port <id> — …`.
  - That stop opens a `harness:human` task "Approve the port …", which
    blocks the task, keeps `kind: port` in the task's `## Decision` and
    notifies through `HumanTaskOpened`. No decision is taken.
  - The planner and split prompts build a port in the milestone of its
    first user. The decide prompt handles a refused port.

- Milestone 5: the Ports pane, opened from the capabilities list's
  *⇄ ports*.
  - It reads `ports.json` on the integration branch and the
    `harness:human` tasks: the approvals waiting, and each port's secrets
    task, provided or not. A unit in `usedBy` opens its capability.
  - *Approve* closes the approval with a comment.
  - *Refuse* comments on the waiting task, labels it
    `harness:needs-decision`, then closes the approval.
  - *Change provider* opens a `harness:roadmap` + `harness:ready` item: a
    new adapter, with a replay first for a port that keeps state.
- Milestone 6: docs (workflows and view ARCHITECTURE, CLAUDE.md).

## To check on a fresh product

- [ ] `init-repo` installs `ports.json`, contract Q, `.sops.yaml`,
      `secrets/README.md` and `scripts/ports-gate.mjs`. The gates are green
      on an empty product.
- [ ] A task that adds `reqwest` to a Capability crate is refused by
      `externals-behind-ports`, naming the crate.
- [ ] A Capability naming a stateful port is refused by `ports-declared`.
- [x] The first port built (`crates/ports/<port>/` with fake, contract tests
      and adapter) passes `ports-declared` and `contracts`. (seen 2026-10-17 23:29 Paris: `crates/ports/llm` (fake, contract tests, adapter) landed with milestone #17; ports-declared and contracts green on main_agent since)
- [ ] `sops --encrypt` on `secrets/production.enc.yaml` with the human's age
      key passes `secrets-encrypted`. A plain `.env` is refused.
- [ ] The server honours `PORTS=fake`, and the rules' tests reach no
      provider.
- [ ] A real creation reaches Ports after Data. The AI lists sensible ports
      (PostgreSQL first, named by function, e.g. `mail` rather than
      `postmark`) with alternatives. Editing a card, picking an alternative,
      adding and removing a port all persist.
- [ ] Changing a rule after the ports were validated marks Ports "to redo"
      and blocks Accept until they are validated again.
- [ ] Initialize commits `ports.json` on the integration branch (contract Q
      validates it), opens one secrets task per port with secrets (none for
      `database`), and opens them only once on a retried initialization.
- [ ] The roadmap task's body and a planner's dossier show the ports.
- [ ] A task that needs a new provider stops with `AGENT_LOOP_STOP: port …`
      rather than calling it. The approval task opens, blocks it, and a
      notification shows. (contradicted 2026-10-18 22:26 Paris: #53 stopped `port archive-storage` -> `a decision is asked`; no `Approve the port …` issue on the board, no human_task_opened event)
- [ ] Approval closed, the next session adds the port to ports.json and
      builds the crate, the fake and the contract tests before the adapter.
- [ ] A refusal (comment + `harness:needs-decision`) is decided as "do
      without", or the task is cut.
- [ ] The Ports pane on a real factory lists `ports.json` from
      `main_agent`, with the secrets tasks' states. An approval appears
      after a port stop.
- [ ] *Approve* closes the approval on GitHub; the task is taken again at
      the next tick and adds the port.
- [ ] *Refuse* (with a reason) puts the task in `decide`, which answers
      "without it" or cuts it.
- [ ] *Change provider* opens a ready roadmap item; the planner and the
      loop deliver a new adapter with the contract tests green.
