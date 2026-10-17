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
