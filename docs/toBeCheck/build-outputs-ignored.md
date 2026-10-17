# Build outputs out of a round's changes

## Context

ForgeDesMondes, 2026-10-16 22:06: dev loop #31 (root Cargo workspace) halted
on `ArchitectureHolds` — `target/: changed by this round, outside the closure`.
The product had no root `.gitignore`; the session's own `cargo build` left an
untracked `target/`, which the gate reads through `dirty_files`. PR #64 itself
carried no `target/` file.

## Shipped

- `init-repo` installs a root `.gitignore` (`target/`, `node_modules/`,
  `.env.local`) when the product has none.
- Every lent clone's `.git/info/exclude` carries `target/` and
  `node_modules/` beside `/.claude/` — covers products made before.

## To verify

- [ ] On ForgeDesMondes, the next dev-loop round that builds Rust passes
      `ArchitectureHolds` with no `target/` line.
- [ ] The lane clone's `.git/info/exclude` lists the three lines once each
      after several rounds.
- [ ] A fresh `init-repo` on an empty product writes `.gitignore`; on a
      product with its own, keeps it.

## Also: a need a delivered sibling produces

ForgeDesMondes #32 (`needs: database`) halted before paying: `database` is
#31's infrastructure, merged into the milestone, but the tree index only
places vocabulary ids (Capability, aggregate, Micro-UI, Concept, screen).
Now an id a closed or `waiting-merge` sibling `produces` is held.

- [ ] #32 narrows and runs its `code` stage with no `needs database` halt.
- [ ] A `needs` on an id no sibling delivered still halts before paying.

## Also: explicit members behind a comment that names them

ForgeDesMondes' root `Cargo.toml` quotes the gate `workspace-members-by-glob`
in a comment above `[workspace]`; `explicit_members` took the first
`members` in the text and read `[workspace]` as the array, so no named
member joined the closure (#32 stopped on `crates/data`, `crates/dataguard/server`).

- [ ] The next infrastructure/Rust task of milestone #16 narrows with
      `crates/data/` and `crates/dataguard/server/` in its tree note, no stop.
