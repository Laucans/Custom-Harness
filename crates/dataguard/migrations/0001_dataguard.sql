-- The DataGuard's own tables, apart from the product's: the queue
-- (contract G), the holds, the journal, the global version, and the rows
-- the applier keeps per partition. Idempotent: `migrate` runs it on every
-- start.
CREATE TABLE IF NOT EXISTS dataguard_queue (
    command          text PRIMARY KEY,
    partition        text NOT NULL,
    position         bigint NOT NULL,
    state            text NOT NULL,
    data_capability  text NOT NULL,
    idempotency_key  text,
    entry            jsonb NOT NULL,
    UNIQUE (partition, position)
);
CREATE INDEX IF NOT EXISTS dataguard_queue_pending
    ON dataguard_queue (partition, position)
    WHERE state NOT IN ('applied', 'rejected', 'cancelled', 'expired');
CREATE UNIQUE INDEX IF NOT EXISTS dataguard_queue_idempotency
    ON dataguard_queue (data_capability, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS dataguard_holds (
    id         text PRIMARY KEY,
    partition  text NOT NULL,
    hold       jsonb NOT NULL
);

-- One row per partition the applier wrote: the confirmed state a command
-- reads, and the tombstone of an archived one.
CREATE TABLE IF NOT EXISTS dataguard_rows (
    partition   text PRIMARY KEY,
    fields      jsonb NOT NULL DEFAULT '{}'::jsonb,
    tombstoned  boolean NOT NULL DEFAULT false
);

-- The rows that point at a partition, per relation: what the Resolver's
-- cascade walks. The product's server fills it from its foreign keys.
CREATE TABLE IF NOT EXISTS dataguard_links (
    relation   text NOT NULL,
    partition  text NOT NULL,
    rows       bigint NOT NULL,
    PRIMARY KEY (relation, partition)
);

CREATE TABLE IF NOT EXISTS dataguard_journal (
    seq           bigserial PRIMARY KEY,
    command       text NOT NULL,
    data_version  bigint,
    result        jsonb NOT NULL,
    at            timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS dataguard_journal_command ON dataguard_journal (command);

-- The global `dataVersion`: one row, one sequence, taken by the applier
-- alone.
CREATE TABLE IF NOT EXISTS dataguard_version (
    one      boolean PRIMARY KEY DEFAULT true CHECK (one),
    version  bigint NOT NULL DEFAULT 0
);
INSERT INTO dataguard_version (one, version) VALUES (true, 0) ON CONFLICT DO NOTHING;
