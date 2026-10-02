# Takeover — what a human must do, and when

> **Where the harness stands.** It runs. `harness --dry-run --no-workspace`
> passes the version gate, queries `gh`, checks branch and CI, then **stops on
> labels** — because the seven `harness:*` don't exist yet. The two moves below
> are all that's missing.

The Rust harness takes over tracking rather than cohabiting: labels move from
`pipeline:*` to `harness:*`, and state moves to `.llocal/agent-loop/`. **Nothing
here is automatic**, deliberate: each of these moves stops the Python pipeline.

## Before switching

**Wait for no rounds in flight.** Python keeps a round's resumption state in
`.llocal/agent-loop/flow_states.db`, sqlite. The Rust harness **doesn't read
sqlite** — it writes JSONL (`docs/MIGRATION.md`, step 5 decision). A round
interrupted mid-way, switched, becomes a round no one knows which stages ran.

```bash
# Should output nothing: no pointer = no round running.
cat .llocal/agent-loop/state 2>/dev/null
```

If the file exists, finish or abandon the round with Python first.

## Rename the seven labels

A GitHub rename **keeps already-labeled issues**: nothing to re-label by hand.
But Python stops finding its board at the first command.

This is still a manual, per-repository gesture — **renaming is not creating**,
and `harness init-repo <url>` only creates a `harness:*` label that doesn't
exist at all (it never recolors or renames one a human already placed). A
repository with no `pipeline:*` history at all doesn't need this section:
`harness init-repo <url>` creates its eight `harness:*` labels directly.

```bash
gh label edit "pipeline:roadmap"       --name "harness:roadmap"
gh label edit "pipeline:milestone"     --name "harness:milestone"
gh label edit "pipeline:agent"         --name "harness:agent"
gh label edit "pipeline:human"         --name "harness:human"
gh label edit "pipeline:ready"         --name "harness:ready"
gh label edit "pipeline:spec-written"  --name "harness:spec-written"
gh label edit "pipeline:waiting-merge" --name "harness:waiting-merge"
```

`pipeline:refinement` is not in the list: the loop doesn't read it, and its
preflight would refuse to run if it did. Rename with refinement when it arrives.

To verify after:

```bash
gh label list --limit 100 | grep -E 'harness:|pipeline:'
```

Either way, `harness init-repo <url>` is also what creates the integration
branch and writes `TARGET_REPO_URL`/`INTEGRATION_BRANCH` into `.env.local` —
run it once the labels are settled, `--dry-run` first to see what it would do.

## Update `claude`

Preflight demands **`claude` >= 2.1.277**, and will refuse below. The reason is
in `docs/SESSION-CARRIER.md`: `total_cost_usd` on a `--resume` call is cumulative
for the entire conversation since that version, and covered only that call before.
The ledger can't be right without knowing which side you're on, and a gate costs
a local call where a wrong `costs.tsv` goes unnoticed.

```bash
claude --version   # 2.1.257 at writing — below the gate
claude update
```

### Caution: `claude update` can be hidden by a shim

On this machine, `claude update` did install 2.1.285 in `~/.local/share/claude/versions/2.1.285`,
and that binary reports its version. But `~/.local/bin/claude` is a hand-written shim that
resolves to **the binary embedded in the VS Code extension**:

```sh
bin=$(ls -d "$HOME"/.vscode/extensions/anthropic.claude-code-*/resources/native-binary/claude \
      | sort -V | tail -1)
exec "$bin" "$@"
```

With the extension at 2.1.257, `claude --version` reports 2.1.257 and the gate refuses
to run — though the update succeeded. Two choices:

1. **update the VS Code extension**: the shim already follows it, nothing else to change.
   The shim's original intent;
2. **point the shim to native install** (`~/.local/share/claude/versions/`, newer), or the
   newest of the two.

Check with the binary the `PATH` actually resolves, not what you think:

```bash
command -v claude && claude --version
```

**The gate says it itself**, by design: its message names the command *and* the shim
trap, because "`claude update` worked but the gate still refuses" is exactly where you
need to read it.

```
STOP: claude 2.1.257 is older than 2.1.277 — … Run: claude update (and check
that `command -v claude` resolves to what you just updated)
```

En attendant, un run se vérifie en mettant l'installation native en tête du
`PATH` pour cette commande-là seulement :

```bash
d=$(mktemp -d) && ln -s "$HOME/.local/share/claude/versions/2.1.285" "$d/claude"
PATH="$d:$PATH" harness --dry-run --no-workspace
```

## What remains shared, what doesn't

| Path | Who writes it after cutover |
| --- | --- |
| `.llocal/agent-loop/costs.tsv` | Rust, appending to history — header frozen, old lines remain readable |
| `.llocal/agent-loop/state` | Rust, same two-line format (`task=`, `flow_id=`) |
| `.llocal/agent-loop/flow_states.db` | nobody. Rust writes `flow-<id>.jsonl` alongside |

Sqlite isn't deleted by cutover: remains readable via `sqlite3` if an autopsy needs it.

## The first real run, when both moves are done

In this order, each answering a question the next presupposes.

```bash
# 1. What would it pick? Costs nothing, opens no session.
harness --dry-run --no-workspace --allow-dirty

# 2. One turn, in a clone, one stage: smallest real spend proving the whole chain holds.
harness --rounds 1 --stages business-analyst

# 3. One full round.
harness --rounds 1
```

Two things to know before the first run mounting a workspace:

- **the clone has no `node_modules` or venv** — nothing git doesn't track. A
  preflight gate says so and refuses to run, rather than burn a `/code` where
  every check command replies `command not found`. Must install them **in the
  workspace**, once; `PERMANENT` keeps them across runs;
- **what isn't pushed doesn't exist for the run.** Mounting clones `origin`. A
  warning says if checkout bears unpushed commits or dirty tree, but it's a
  warning: doesn't refuse.

`--rollover` is **not** the default: a finished milestone stops the loop instead of
paying a `/planner` that commits the project to a roadmap item no one read. Branching
is a decision, not a setting.
