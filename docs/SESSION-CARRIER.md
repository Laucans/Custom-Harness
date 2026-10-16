# What carries a `Session` — decision report

> **Decided: C now, A as destination.** Implemented in `adapters::agent::claude_cli`.
> The port (`Session` / `SessionFactory`) didn't move, so moving to A won't touch
> `Stage`/`Round` or workflows.

Step 5. Three proposals. The port was already written (`adapters::agent::Session` /
`SessionFactory`), so this choice doesn't touch `Stage`/`Round` or workflows: it's
a **reversible** decision, and that counts in the decision.

## What implementing C revealed

A version trap, found by reading docs rather than discovering it in a wrong `costs.tsv`:

> **`total_cost_usd` on a `--resume` call is cumulative for the entire conversation
> since Claude Code v2.1.277.** Before that version, each call returned only its own.

So a stage's cost is the value of the **last** turn, and summing turns would double-count
— *or the reverse*, depending on the installed version. This machine is on **2.1.257**,
the wrong side of the flip: summing gives the right number today and a wrong number
after an update, with nothing to signal it.

Approach chosen: the adapter **doesn't decide**, it faithfully reports what the turn
said in `Reply::cost`. It's up to the spending ledger — not yet written — to accumulate,
and **a preflight gate on `claude --version` will have to decide it**. A gate costs a
local call; a wrong `costs.tsv` doesn't show.

Two other points noted along the way:

- `total_cost_usd` is a **client-side estimate** from an embedded price table. Good
  for budgeting, never for billing.
- `usage` **excludes subagents**; `total_cost_usd` and `model_usage` include them.
  Since the `code` stage launches them, counting tokens from `usage` would undercount
  — it's `total_cost_usd` to read.
- **Don't use `--bare`**: it skips skill discovery and `CLAUDE.md`, which the harness
  depends entirely on (`/business-analyst`, `/code`).

## Correction: what I wrote at step 4 was wrong

`MIGRATION.md` said, under tmux: "**cost and usage are not retrievable**". Wrong.
The error was assuming the pane is the only channel. It's not — and the question
"can we ask the session for the price?" has a better answer: **we don't ask the pane
at all.**

Four structured channels exist *alongside* the terminal, all compatible with an
interactive session hosted in tmux.

### 1. `statusLine` — the richest

Claude Code invokes a status line script by passing JSON on stdin. The schema
carries, among others:

```json
{
  "session_id": "...", "transcript_path": "...", "model": { "id": "..." },
  "cost": {
    "total_cost_usd": 0.01234,
    "total_duration_ms": 45000, "total_api_duration_ms": 2300,
    "total_lines_added": 156, "total_lines_removed": 23
  },
  "context_window": {
    "current_usage": { "input_tokens": 8500, "output_tokens": 1200,
      "cache_creation_input_tokens": 5000, "cache_read_input_tokens": 2000 }
  },
  "prompt_cache": { "hit_ratio": 0.91, "cache_write_tokens": 352000, "…": "…" },
  "effort": { "level": "high" },
  "rate_limits": {
    "five_hour":   { "used_percentage": 23.5, "resets_at": 1738425600 },
    "seven_day":   { "used_percentage": 41.2, "resets_at": 1738857600 },
    "spend_limit": { "used_percentage": 62.8, "resets_at": 1740787200 }
  }
}
```

- **Exact USD cost**, not tokens to convert.
- **Cache ratios**, which `runtime/monitoring/metrics.py` calculates by hand today.
- **Quotas, structured and *predictive*.** Today `Halt::Quota` is detected
  **after the fact**, searching for a phrase in an already-failed run. Here the
  harness can know *before* opening a stage that the 5h window is at 95% — and
  stop cleanly instead of burning a stage. A win stream-json and the transcript
  don't offer.
- Event-driven updates, debounced at 300 ms, plus an optional `refreshInterval`
  (minimum 1 s).
- **Trap**: a script in flight is **cancelled** if a new update arrives. So atomic
  write (temp file + `rename`), else torn record.
- It's a TUI element: available precisely in the interactive case (so tmux), not
  in `-p` mode.

### 2. OpenTelemetry — cleanest to aggregate

| Metric | Unit | When |
| --- | --- | --- |
| `claude_code.cost.usage` | **USD** | after each API request |
| `claude_code.token.usage` | tokens | after each API request |

Attributes: `session.id`, `model`, `effort`, `query_source` (`main` / `subagent` /
`auxiliary`), `speed`. So attributable per session, distinguishing a subagent from
the main thread.

- Exportable **locally, no network**: `OTEL_METRICS_EXPORTER=prometheus` exposes
  `http://localhost:9464/metrics`, which the harness scrapes.
- **Don't use**: `OTEL_METRICS_EXPORTER=console` — writes to stdout, so in the pane,
  over the TUI.
- Cumulative counter: a turn's cost is a delta between two scrapes. If one session
  = one stage, a single read at the end suffices.
- One port per process: no object here, the harness is sequential (decision #3).

### 3. The JSONL transcript — verified on your machine

`~/.claude/projects/<cwd-slug>/<session-id>.jsonl`, and `--session-id <uuid>`
lets the harness **choose** the uuid, so it knows the path ahead.

Measured on the current session transcript (332 `assistant` messages):

- `message.usage` present on **100%** of them: `input_tokens`, `output_tokens`,
  `cache_creation_input_tokens`, `cache_read_input_tokens`, `service_tier`,
  `speed`, `iterations`.
- `stopReason` at root level — an exploitable turn boundary.
- **No cost field**: `grep` every name containing `cost`/`usd` → nothing. Tokens
  yes, dollars no. Would need a price table to maintain, exactly the kind of
  drift-prone duplication.

### 4. The `Stop` hook — turn boundary

Payload: `session_id`, `transcript_path`, `cwd`, `permission_mode`,
`hook_event_name`. Fired when the main agent finishes answering — so **end of a
turn, as an event**, without reading a single byte of terminal.

- The project can already do it: `event_assistant` already runs a `PostToolUse`
  on `gh pr create` and a `PreToolUse` branch guard.
- **Known caveats**: reported bugs with `transcript_path` / `session_id`
  **stale** after `/exit` and `--continue`
  ([#8564](https://github.com/anthropics/claude-code/issues/8564),
  [#9188](https://github.com/anthropics/claude-code/issues/9188)). Test before
  relying.
- Payload doesn't carry usage; open request ([#91767](https://github.com/anthropics/claude-code/issues/91767)).

## The real cost of tmux, not what I thought

The cost isn't "no cost data". It's this:

> **tmux doesn't buy agent agnosticism. It buys a generic container plus an
> instrumentation port to write per agent.**

`statusLine`, OTel metric names, hooks, transcript schema: **all Claude Code
specific.** A tmux pane hosting `opencode` will have none of these four channels
— it will have others, or none. So the original motivation ("one tmux will carry
a claude or an opencode") holds for the *container*, not the instrumentation.
Defensible — even the right architecture — but it's two ports, not one.

What remains hard under tmux, unsolvable by any out-of-band channel:

- **Driving input remains keyboard typing.** `send-keys` in a TUI is more fragile
  than writing a JSON line: bracketed paste, multi-line prompts, a leading `/`
  interpreted, a pane in a modal state. `send-keys -l` and `load-buffer` +
  `paste-buffer` help, but you're simulating a human.
- **Permission requests block.** Unattended, a permission dialog is indefinite
  blocking. `--permission-mode bypassPermissions` settles it — and the project
  already chose it (`PERMISSION_MODE`).

## The three proposals

### A — tmux + Claude Code instrumentation

One pane per stage, interactive `claude`, `--session-id <uuid>` imposed by harness,
`--permission-mode bypassPermissions`. Input via `send-keys`. Turn end via `Stop`
hook. Cost and quotas from `statusLine`, atomically written to a file per session.

| | |
| --- | --- |
| **Gains** | attachable in flight (`tmux attach`): a human resumes a stuck stage at 3 AM. Exact USD cost, cache ratios, **predictive quotas**. Generic container for a future agent. |
| **Costs** | three Claude Code-specific surfaces to write and maintain (statusline, hook, keystrokes). TUI input fragility. Hook staleness caveats. |
| **Effort** | highest |

### B — stream-json, long-running process

`claude -p --input-format stream-json --output-format stream-json
--replay-user-messages`. One child process, JSON line-by-line both ways. Turn end
and `total_cost_usd` in the `result` message.

| | |
| --- | --- |
| **Gains** | one channel, typed, in-band. No side instrumentation. Native acknowledgement (`--replay-user-messages`). Real "session open". Works in CI, no TTY. |
| **Costs** | not attachable: no one can watch or resume. Bidirectional protocol to write — biggest adapter of the migration. No predictive quotas. |
| **Effort** | high, concentrated in one place |

### C — one process per action, stitched by `--resume`

Each `SessionAction` = a `claude -p --resume <uuid> --output-format json`. One
JSON object to parse, `total_cost_usd` inside, turn boundary = process exit.

| | |
| --- | --- |
| **Gains** | by far the least code: no protocol, hook, statusline, or keystrokes. USD cost and turn end **free** per call. Conversation continuity — what `lead` existed for — well preserved by `--resume`. |
| **Costs** | "session open" is fiction: context reload per action (prompt cache amortizes cost, not latency). Not attachable. No predictive quotas. |
| **Effort** | lowest |

## Recommendation — chosen

**C first, A as destination if attachability proves to matter.**

Three reasons:

1. **The project optimizes for simplicity and fast iteration**, not completeness
   (parent repo's `CLAUDE.md`, in so many words). C is small, and it *already*
   gives exact cost and turn boundaries.
2. **The decision is reversible by construction.** The `Session` trait exists,
   `Stage`/`Round` are tested against fakes. Moving from C to A touches no
   workflow. Taking the expensive path first is paying for an option we don't yet
   know we need.
3. **B is the wrong buy.** It costs the most (a bidirectional protocol) for a
   benefit — a truly live process — that C approximates at a fraction of the effort,
   gaining nothing on observability.

What would flip to **A**: if in real use stages get stuck and we want to resume
without killing the run, or if predictive quotas become necessary to avoid wasting
rounds. Both plausible — hence "destination", not "never".

## Prompt cache — what C pays, and how a round shares a prefix

Measured on Claude Code 2.1.292, sonnet, 2026-10-13:

- A fresh `claude -p` re-reads only the system prompt and tools (~21k
  tokens). Its whole first message (CLAUDE.md, skills, the harness's
  prompt) is a cache **write**, at the 1-hour TTL: twice the input price.
  Claude Code sets its cache breakpoint at the end of the message, so two
  sessions opening on the same repo map share nothing past the system.
- `--resume <base> --fork-session` is the only way for a session to start
  from another's prefix. A step forked from a base holding a 31k context
  cost $0.037 against $0.129 fresh.
- A fork reports `total_cost_usd` for the **whole** conversation, base
  included: the adapter takes the base's cost out (`net_of_base`).
- The operator's profile (claude.ai MCP servers, user plugins and skills)
  is kept out of every robot session (`LEAN` flags): 2.4k fewer tokens
  written per session.

How it is wired: `SessionFactory::fork` opens a branch of a `Base`; a
workflow that sets `Context::base` makes every later session stage of the
round a branch. Refinement does, in its `prime` stage — the map and the
hierarchy told once, right after `explore` — and its steps then send only
their own template. A carrier that reports no session (a dry-run) leaves no
base, and the steps carry the context themselves, as before.

Measure it with `scripts/cache-report.sh`: per stage, the share of cache
writes and the write per session.

Measured on Claude Code 2.1.292, haiku, 2026-10-15, a base primed in one
directory and forked from it:

- From the same directory, a fork writes 306 tokens: −94% against the base.
- From another directory (another lane's clone), `--resume` still finds the
  base — no transcript to copy — but re-sends **that** directory's
  CLAUDE.md (7.9k tokens written for a 30k-character file): −73%. A base
  shared by every lane pays CLAUDE.md once per fork, so the product's
  CLAUDE.md is kept short.

## Which checks fail

A repair keeps every failing check of the pull request it repairs as a
`checks_failed` event in `.llocal/harness.db`: what tells a rule whose gate
fails often (worth more detail in CLAUDE.md or its unit's card) from one
that never does. Grouped by the check's name:

```sh
sqlite3 -readonly .llocal/harness.db "
  SELECT substr(c.value, 1, instr(c.value || ' — ', ' — ') - 1) AS name, count(*)
  FROM events, json_each(events.body, '$.checks') AS c
  WHERE kind = 'checks_failed' GROUP BY name ORDER BY 2 DESC;"
```

## Sources

- [Monitoring — Claude Code Docs](https://code.claude.com/docs/en/monitoring-usage) — `claude_code.cost.usage` (USD), `claude_code.token.usage`, exporteurs locaux
- [Customize your status line — Claude Code Docs](https://code.claude.com/docs/en/statusline) — schéma JSON stdin, cadence, annulation en vol
- [Hooks reference — Claude Code Docs](https://code.claude.com/docs/en/hooks) — événements et champs communs
- [Manage costs effectively — Claude Code Docs](https://code.claude.com/docs/en/costs)
- [Track cost and usage — Agent SDK](https://code.claude.com/docs/en/agent-sdk/cost-tracking)
- Issues : [#8564](https://github.com/anthropics/claude-code/issues/8564), [#9188](https://github.com/anthropics/claude-code/issues/9188) (péremption hook), [#91767](https://github.com/anthropics/claude-code/issues/91767) (usage dans les hooks)
- Mesure locale : `claude` 2.1.257, `tmux` 3.6b, transcript de session (332 messages `assistant`)
