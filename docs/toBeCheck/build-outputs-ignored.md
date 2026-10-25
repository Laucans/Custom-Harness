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

- [x] On ForgeDesMondes, the next dev-loop round that builds Rust passes
      `ArchitectureHolds` with no `target/` line. (seen 2026-10-17 04:26 Paris: ArchitectureHolds passed from run 20261017-021114 on — 193 passes to 24 Oct, none naming target/)
- [x] The lane clone's `.git/info/exclude` lists the three lines once each
      after several rounds. (seen 2026-10-25 01:40 Paris: lane-0..4, every router-lane-* and catch-up exclude holds /.claude/, target/, node_modules/ once each after hundreds of rounds)
- [ ] A fresh `init-repo` on an empty product writes `.gitignore`; on a
      product with its own, keeps it.

## Also: a need a delivered sibling produces

ForgeDesMondes #32 (`needs: database`) halted before paying: `database` is
#31's infrastructure, merged into the milestone, but the tree index only
places vocabulary ids (Capability, aggregate, Micro-UI, Concept, screen).
Now an id a closed or `waiting-merge` sibling `produces` is held.

- [x] #32 narrows and runs its `code` stage with no `needs database` halt. (seen 2026-10-17 04:41 Paris: run 20261017-024150 narrowed #32 to 16 patterns, its code stage ran, PR #65 delivered at 05:04)
- [ ] A `needs` on an id no sibling delivered still halts before paying.

## Also: explicit members behind a comment that names them

ForgeDesMondes' root `Cargo.toml` quotes the gate `workspace-members-by-glob`
in a comment above `[workspace]`; `explicit_members` took the first
`members` in the text and read `[workspace]` as the array, so no named
member joined the closure (#32 stopped on `crates/data`, `crates/dataguard/server`).

- [ ] The next infrastructure/Rust task of milestone #16 narrows with
      `crates/data/` and `crates/dataguard/server/` in its tree note, no stop. (not seen: #32's tree got crates/data back by the found-missing widening, not by explicit members; narrowing is off since 17 Oct (NO_NARROW, narrowing-off-trial.md))
