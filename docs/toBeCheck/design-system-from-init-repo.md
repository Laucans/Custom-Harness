# The design system's gates come from init-repo, not from a roadmap item

## Context
ForgeDesMondes' first roadmap item, "Install the design system in the
product's frontend" (opened by the view when the design system was
validated), cost ~$6 and three tasks to write generic CI gates: the
single-writer gate hardened, and a scanner refusing literal colours, fonts,
spacings, radii and shadows. The real frontend did not exist yet, so nothing
was styled. Every new product would pay for the same scanner again.

## Shipped
- `crates/architecture/assets/design-system-gate.mjs`, installed by
  `init-repo` as `scripts/design-system-gate.mjs` (from ForgeDesMondes, its
  four hard-coded placement files replaced by the in-file marker
  `design-system-gate: computed placement`); `gates.yml` gains the
  `design-system` job and the hardened `design-system-single-writer`.
- `crates/architecture/tests/design_system_gate.rs`: a synthetic mock-up,
  clean passes, each way around the tokens fails, the marker exempts.
- The view no longer opens the "Install the design system" roadmap item on
  validation; the proof of concept's roadmap says how the frontend wears it.

## To verify
- [ ] A new factory: `init-repo` installs `scripts/design-system-gate.mjs`;
      the `design-system` job runs on its first pull request.
- [ ] A freshly drawn mock-up passes the gate (or its first failures are true
      violations, not the gate misreading a generated page).
- [ ] Validating the design system opens no roadmap item.
- [ ] The proof of concept's first milestone that builds the shell loads
      `tokens.css` and `kit.css` once and styles through the variables.
