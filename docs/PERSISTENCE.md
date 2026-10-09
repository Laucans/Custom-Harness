# Persistence — PostgreSQL runs in Docker

**Decision (2026-10-09): every local database the harness or a product it
builds needs runs in Docker, from `docker/postgres/compose.yml`, started with
`scripts/postgres.sh`.** No Homebrew service, no `pg_ctl` by hand, no database
a session installs or starts for itself. This replaces the Homebrew
PostgreSQL that #73 had chosen for `dnd_helper`.

Why:

- **One recipe for every machine and every product.** The image, the version
  (16, as the products' CI runs), the roles and their rights are written in
  the repository, not in someone's shell history.
- **Disposable and repeatable.** `scripts/postgres.sh reset` gives a clean
  cluster; the provisioning brings it back to what `.env.local` says.
- **Isolated.** Bound to `127.0.0.1` only; the data lives in the named volume
  `harness-pgdata`, apart from the machine's own services.
- **The same rights as CI.** The application role may create databases and
  roles (it migrates, its tests create a database each), the read-only login
  can read and nothing else — what the products' `ci.yml` provisions too.

## Using it

```sh
scripts/postgres.sh up          # start it; created and provisioned the first time
scripts/postgres.sh status      # up? healthy? which port?
scripts/postgres.sh provision   # .env.local changed: roles and passwords back in line
scripts/postgres.sh psql [db]   # a psql as the superuser, inside the container
scripts/postgres.sh logs        # follow the server's log
scripts/postgres.sh down        # stop it — the data stays in the volume
scripts/postgres.sh reset       # stop it and delete the volume (asks first)
```

The container restarts with Docker (`restart: unless-stopped`): once up, it
stays up across reboots as long as Docker (OrbStack here) runs.

## What it reads from `.env.local`

Only these, for interpolation — nothing else from `.env.local` enters the
container:

| variable | what it provisions |
| --- | --- |
| `DATABASE_URL` | the application role (`LOGIN CREATEDB CREATEROLE`), its password, and its database, owned by it |
| `DATABASE_URL_READONLY` | the read-only login (`LOGIN`, no other right) |
| `DATABASE_READONLY_ROLE` | the role that login reads through — created `NOLOGIN` if the migrations have not yet, and granted to the login (`app_lecture` for `dnd_helper`) |
| `POSTGRES_PORT` | the host port, `5432` by default |
| `POSTGRES_SUPERUSER_PASSWORD` | the `postgres` superuser's, `postgres` by default — local only |
| `POSTGRES_IMAGE` | the image, `public.ecr.aws/docker/library/postgres:16` by default |

The image comes from AWS's public mirror of Docker Hub's official library:
the same image, pulled without a Docker Hub login and without its rate limit
(this machine's stored Hub credentials are refused).

## Who connects

- **The runs and the product's tests**, through `DATABASE_URL` /
  `DATABASE_URL_READONLY`: the launcher exports `.env.local` to every session.
- **The data snapshot** (`scripts/dump-schema.sh` in the product, or the
  harness's default): it creates a scratch database with the application
  role, applies the migrations, dumps the schema, drops it
  (`crates/workflows/src/data_snapshot/`).
- **A human**, with `scripts/postgres.sh psql`.

## For future development

- A product that needs a database uses **this** server: it gets its own
  database and roles by naming them in `.env.local` and running
  `scripts/postgres.sh provision`. Never a second PostgreSQL, never a local
  install.
- A product needing another store (Redis, an object store…) adds it as a
  service in a compose file under `docker/`, with the same shape: a pinned
  image, a named volume, `127.0.0.1` only, credentials from `.env.local`, a
  control script in `scripts/`, and a section in this document.
- Schemas change through the product's migrations only; nothing is created
  by hand in the container that a migration would not recreate.
