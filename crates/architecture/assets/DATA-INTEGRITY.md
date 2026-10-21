# Runtime data integrity

The engine's manual · installed by `harness init-repo` · version 2 — the
layout and the contracts are `docs/ARCHITECTURE.md`

<!-- harness:architecture v2 -->

This document is read by whoever builds the write side — the engine
(`harness-dataguard`, taken by git), the product's `crates/dataguard/server`
(the engine planted with every unit of `crates/dataguard/registry`, which
`scripts/register.sh` writes) and `crates/server` (the product's binary),
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
4. **Parked.** A command that reaches the head without confirmation waits
   there for its author's answer a short grace (30 seconds, the time an
   HTTP confirmation takes), then is set aside and the queue goes on. The
   author is notified.
5. **Back in the queue.** The author of a parked command may ask for a recompute
   of the projected state: the command returns to the end of the queue, with a
   new base, and they confirm against this new view.
6. **Expiry.** A parked command with no action before its `ttl` disappears; the
   author is told.
7. **Holds.** A hold freezes the partition from its position, can be renewed,
   and disappears at expiry.
8. **Double check of the invariants.** At enqueue, on the projected state, to
   warn early; at application, on the real state, which alone decides.

An application is **one transaction**: the aggregate's row, its projections
into the product's own tables (an aggregate's `ProjectPg`, run after the row
and before the commit), the next `dataVersion` and the journal. A projection
that refuses rolls all of it back, and the command is rejected at the head as
an invariant would reject it (`CommandRejected`, with the projection's reason).

A confirmed command advances in its place up to the head. Only a command that
reaches the head unconfirmed leaves the queue, and comes back only at the end,
after a recompute. The author may also cancel at any time before application.

**Relative commands** (`stock -= 2`) compose with the commands ahead instead of
overwriting them: they are never stale and **need no confirmation**; only the
invariants are checked, at application.

**Merging fields** (`merge` in `aggregate.json`) are never stale either: a
value declared ahead, or one that moved since the author read it, asks
nothing. `merge: set` unions the arrays written, in order, without
duplicates (`tags: [a]` then `[b, a]` gives `[a, b]`); `merge: last-writer`
lets the last write win. The projected view still shows what is ahead.

**Derived fields** (`kind: derived`) are recomputed from other rows: the
engine refuses (`Forbidden`) a command that writes one, before it is
queued.

**What the engine checks of its units.** At start, every invariant a
`DataCapability` lists must be one its target aggregate declares, or the
engine refuses to start (`Misconfigured`). A payload key the manifest's
`payload` does not declare is accepted with a warning in `Submitted.warnings`.
`onUpdate` on a relation is deprecated and ignored: a partition's id never
changes.

### Default durations

| What | Default |
|---|---|
| A hold's `ttl` | 1 minute, renewable while the workflow that holds it is alive |
| A parked command's `ttl` | 24 hours — a human confirmation takes hours, not seconds |
| The grace at the head before parking | 30 seconds — the time an HTTP confirmation takes to arrive |

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

## The units' code

Every unit is a crate in its folder, registered by
`bash scripts/register.sh`; the registry's glue calls one item per crate.
An aggregate and a DataCapability take the engine from the workspace
(`harness-dataguard = { workspace = true }`), and their `plugin()` says what
they plug in. A Capability never names the engine. Short forms, from
`harness_dataguard::plugins`:

**`Prepare`** — a DataCapability's code
(`crates/credit/data-capabilities/request-limit/src/lib.rs`): the
submission turned into what it writes, or refused before anything queues.

```rust
pub struct Policy(pub f64); // filled by crates/server's `Shared`

struct RequestLimit;

#[async_trait]
impl Prepare for RequestLimit {
    async fn prepare(&self, cx: &Context<'_>, payload: &BTreeMap<String, Value>,
                     target_id: Option<&str>) -> Result<Prepared, Refusal> {
        let ceiling = cx.shared.get::<Policy>().map_or(0.0, |p| p.0);
        let asked = payload.get("creditLimit").and_then(Value::as_f64).unwrap_or(0.0);
        if asked > ceiling {
            return Err(Refusal::unprocessable("over the policy")); // 422
        }
        let customer = payload.get("customerId").and_then(Value::as_str).unwrap_or_default();
        if cx.row(&format!("Customer/{customer}")).await?.is_empty() {
            return Err(Refusal::not_found(format!("no customer {customer}"))); // 404
        }
        Ok(Prepared { payload: payload.clone(),
                      target_id: target_id.map(str::to_string), ..Prepared::default() })
    }
}

pub fn plugin() -> Plugin {
    Plugin::new().prepare("credit.requestLimitChange", RequestLimit).needs::<Policy>()
}
```

What the `Context` lends: `base`, the target row as the pending commands
leave it; `cx.row("<Aggregate>/<id>")`, any other row the same way (a
fixture the product's tables hold is hydrated by its aggregate's
`ProjectPg::hydrate`); `cx.rows`, read-only SQL over PostgreSQL (`None` in
memory); `now` and `now_millis`; `ids`; `shared`. What `Prepared` carries
beside the payload and the row: `warnings`; `then`, follow-up submissions
the engine makes once this command is queued, in order, by the same caller,
each through the whole submit path and not atomically (the first refused
stops the rest and undoes nothing; their outcomes come back in the
answer's `then`; a follow-up's own `then` is refused); `attachment`, what
the author reads in the command's result once it is applied (an export's
archive). The undeclared-key warning judges the payload the author sent,
before the `Prepare` rewrites it.

**`Judge`** — an aggregate's invariants as code
(`crates/credit/aggregates/Account/src/lib.rs`, library
`aggregate_account`): the ids the row after the command violates. Only
the real state, at application, decides.

```rust
struct Ceiling;

impl Judge for Ceiling {
    fn violated(&self, state: &BTreeMap<String, Value>, _entry: &Entry) -> Vec<String> {
        let limit = state.get("creditLimit").and_then(Value::as_f64).unwrap_or(0.0);
        if limit > 10_000.0 { vec!["limit-under-ceiling".into()] } else { Vec::new() }
    }
}
```

**`Project` / `ProjectPg`** — what follows an applied write of the
aggregate. `Project` sees it in memory; `ProjectPg` writes the product's
tables inside the transaction that applies it (the engine's `postgres`
feature), and refusing rolls the whole command back. Its `hydrate` (none by
default) answers a row its tables hold but the engine never wrote, such as
fixtures loaded by SQL: the store asks it when it has no row of its own.

```rust
struct Ledger;

#[async_trait]
impl ProjectPg for Ledger {
    async fn project(&self, write: &Write, tx: &Transaction<'_>) -> Result<(), ProjectionError> {
        let limit = write.values.get("creditLimit").and_then(Value::as_f64);
        tx.execute("insert into limit_history (account, credit_limit) values ($1, $2)",
                   &[&write.partition, &limit]).await?;
        Ok(())
    }
}

pub fn plugin() -> Plugin {
    Plugin::new().judge("Account", Ceiling).project_pg("Account", Ledger)
}
```

**`Answer`** — a Capability's read, written as four items the glue wraps
into an `Answer` (`crates/credit/capabilities/calculate-risk/src/lib.rs`);
the server runs `SQL` in a `READ ONLY` transaction, the variables bound in
order, and hands the rows (a JSON array) to `answer`.

```rust
pub const CAPABILITY: &str = "credit.calculateRisk";
pub const SQL: &str = "select score from risk where account = $1";
pub const VARIABLES: &[&str] = &["accountId"];

pub fn answer(rows: &Value, _variables: &BTreeMap<String, Value>) -> Value {
    json!({ "score": rows.get(0).and_then(|r| r.get("score")).cloned() })
}
```

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
attachment: null              # what the Prepare computed, answered once applied (H)
```

### H. Command result

```yaml
commandId: c_130
status: applied               # applied | rejected | expired | cancelled
dataVersion: 1043             # for the consistent read after write
violations: []                # violated invariants if rejected
reviewId: null
attachment: { archive: c_130.zip }  # optional: what the Prepare computed, once applied
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
