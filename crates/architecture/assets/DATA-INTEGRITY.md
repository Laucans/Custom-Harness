# Runtime data integrity

The engine's manual · installed by `harness init-repo` · version 2 — the
layout and the contracts are `docs/ARCHITECTURE.md`

<!-- harness:architecture v2 -->

This document is read by whoever builds the write side — the engine
(`harness-dataguard`, taken by git), the product's `crates/dataguard/server`,
an aggregate — and by nobody else: a Capability or a Micro-UI learns what it
needs from principle 10 and contracts F, H and I of `docs/ARCHITECTURE.md`.

## Resolver

Every modification, deletion or addition that touches existing data goes
through the Resolver. It checks three things and produces an **impact plan**:
the dependencies, the consequences on links, then the invariants on the final
state.

### Dependencies

| Level | Question | Source |
|---|---|---|
| Structural | Which Capabilities read this table or field? | The registry of persisted queries |
| Instance | Does someone depend on this row right now? | Queued commands on the row, incoming references, holds |

The instance level cannot be guessed: a workflow in progress declares it with a
**hold** (for example a credit review open on customer 881). A hold is an entry
of the DataQueue: it freezes the partition from its position, has a `ttl`, can
be renewed and disappears at expiry.

### Consequences on links

Every relation declares its policy (`restrict`, `cascade`, `nullify`,
`archive`) and can be marked `semantic` when the link exists outside a foreign
key. The Resolver walks the incoming references, computes the complete effect
and lists the derived values to invalidate.

### Decision

The impact plan ends in `auto`, `confirm`, `human_review`, `blocked` or
`rejected`. A `blocked` is not a rejection: the command keeps its place in the
DataQueue.

### Deletion

Deletion is a **soft delete** by default (archive with a tombstone). A hard
delete is rare and goes through human review. Everything stays reversible
thanks to the command journal. **The Data layer filters tombstones**: no
persisted query sees an archived row unless it asks for it explicitly.

## DataQueue and concurrency

A command is never rejected because of concurrency. It takes a place in a queue
ordered **by partition — one partition per aggregate** — keeps it, and its
author confirms whether it overwrites a value declared ahead of it. Different
partitions advance in parallel.

### Roles

| Component | Role |
|---|---|
| DataQueue | FIFO queue per partition; no global order |
| Integrity DataGuard | Watches the queue: computes the projected state, detects stale commands, checks invariants, emits the messages |
| DataResolver | Processes the head of the queue: final check, cascades, application. **One applier** for the whole store, which is what makes `dataVersion` a single sequence and lets an invariant spanning several aggregates be checked on the real state at application |

### Rules

1. **Fixed position.** A command keeps its place while it waits for a
   confirmation. It blocks nobody as long as it is not at the head.
2. **Two-layer projected view.** A command submitted behind others sees the
   confirmed projected state, plus the list of commands awaiting confirmation
   placed ahead of it.
3. **Overwrite confirmation.** If a value is declared ahead of its command, the
   author is notified and confirms or cancels. A confirmed command overwrites
   in its turn.
4. **Parked.** A command that reaches the head without confirmation is set
   aside and the queue goes on. The author is notified.
5. **Back in the queue.** The author of a parked command may ask for a recompute
   of the projected state: the command returns to the end of the queue, with a
   new base, and they confirm against this new view.
6. **Expiry.** A parked command with no action before its `ttl` disappears; the
   author is told.
7. **Holds.** A hold freezes the partition from its position, can be renewed,
   and disappears at expiry.
8. **Double check of the invariants.** At enqueue, on the projected state, to
   warn early; at application, on the real state, which alone decides.

A confirmed command advances in its place up to the head. Only a command that
reaches the head unconfirmed leaves the queue, and comes back only at the end,
after a recompute. The author may also cancel at any time before application.

**Relative commands** (`stock -= 2`) compose with the commands ahead instead of
overwriting them: they are never stale and **need no confirmation**; only the
invariants are checked, at application.

### Default durations

| What | Default |
|---|---|
| A hold's `ttl` | 1 minute, renewable while the workflow that holds it is alive |
| A parked command's `ttl` | 24 hours — a human confirmation takes hours, not seconds |

### Why a confirmation stays valid

The queue is FIFO and a parked command only comes back at the end. The set of
commands ahead of a command can therefore only shrink: nothing can slip in.
When the author confirms, they have seen everything that can still apply before
them. Their confirmation covers every scenario and never needs to be asked
again.

A confirmation is not a guarantee of application: the DataResolver may still
reject at the head if an invariant or a cascade forbids it, and the
`CommandRejected` message says why.

### Two distinct confirmations

- **Overwrite confirmation**, by the author: they accept to overwrite what is
  declared ahead of them.
- **Critical review**, by an approver role: required for `critical` fields or
  an impact plan in `human_review`. It adds to the first; AI may propose a
  reconciliation, a human decides.

### Messages of the Integrity DataGuard

| Message | Recipients | When |
|---|---|---|
| `CommandQueued` | Author | At enqueue |
| `InvariantAtRisk` | Author | The projected state violates an invariant |
| `ValueDeclaredAhead` | Author | A value is declared ahead of their command |
| `BlockedByHold` | Author, hold owner | The partition is frozen |
| `HoldExpiring` | Hold owner | Before the end of the `ttl` |
| `ReviewRequired` | Approvers | Critical field or `human_review` |
| `Parked` | Author | Reached the head unconfirmed |
| `ParkedExpiring` | Author | Before the end of the `ttl` |
| `Expired` | Author | Parked command withdrawn |
| `AheadResolved` | Author | Optional: a command ahead was applied or cancelled |
| `CommandApplied` | Author, infrastructure components concerned | Application; invalidates caches |
| `CommandRejected` | Author | Rejection at the head, with the reason |

Messages travel over **server-sent events** from the Integrity DataGuard, one
subscription per author; infrastructure components subscribe the same way.

## Interface contracts of the engine

Contracts G, H, J, K and L, in the same shape as the others: JSON Schema in
`contracts/`, examples in YAML.

### G. DataQueue entry

```yaml
command: c_130
dataCapability: credit.requestLimitChange@2
by: carol
partition: Account/881
position: 3
basedOn: { version: 10, values: { creditLimit: 5000 } }
projection:
  confirmed: { creditLimit: 8000, after: [c_120] }
  pendingAhead:
    - { command: c_123, by: bob, value: 3000, state: awaiting_confirmation }
yourValue: 4500
state: awaiting_confirmation  # queued | awaiting_confirmation | confirmed | awaiting_review
                              # | parked | applied | rejected | cancelled | expired
confirmation: { by: carol }
parked: { ttl: 24h, onExpire: drop }
requeuedFrom: null            # the original commandId if requeued
```

### H. Command result

```yaml
commandId: c_130
status: applied               # applied | rejected | expired | cancelled
dataVersion: 1043             # for the consistent read after write
violations: []                # violated invariants if rejected
reviewId: null
```

### J. Impact plan

```yaml
command: c_201                # credit.archiveCustomer(881)
classification: delete
dependencies:
  readers: [customer.calculateRisk, billing.invoiceList, crm.timeline]
  holds:   [{ by: credit.limitReview, id: w_77, expires: 2026-10-09 }]
  pendingCommands: [c_198]
cascade:
  - { relation: Payment, rows: 42, policy: restrict, blocking: true }
  - { relation: Note, rows: 3, policy: cascade }
  - { relation: Document, rows: 1, policy: review, semantic: true }
invalidates: [Risk@3(customer:881), Dashboard(customer:881)]
invariantsAfter: ok
decision: blocked             # auto | confirm | human_review | blocked | rejected
reasons: ["active hold w_77", "Payment restrict"]
```

### K. Reconciliation proposal

```yaml
review: r_45
aggregate: Account/881
field: creditLimit
base:    { version: 10, value: 5000 }
candidates:
  - { command: c_120, by: alice, value: 8000 }
  - { command: c_123, by: bob,   value: 3000 }
proposal:
  by: ai
  value: 3000
  rationale: "Risk went HIGH between the two requests; c_120 was based on a stale Risk"
  invariantsChecked: [limit-under-ceiling]
decision: { status: pending, approver: role:credit-manager }
```

### L. Message

```yaml
message: ValueDeclaredAhead
to: [bob]
command: c_123
field: Account/881.creditLimit
yourValue: 3000
declaredAhead: { value: 8000, by: alice, command: c_120 }
actions: [confirm_overwrite, cancel, edit]
```

<!-- /harness:architecture -->
