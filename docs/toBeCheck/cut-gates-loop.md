# The sessions that write the cut loop on its gates

## Context
Validating the Rules step failed on a real product: 102 needs/actions with
no `data-reads`/`data-writes` part (contract D). The sessions that wrote
the fragments never ran the gate the validation runs; a human had to fix
22 fragments by hand. Now every session that writes the cut checks the
gates and is sent back until they pass.

## Shipped
- Componentize: `gated` runs `not_componentized` + `unnamed_parts` after
  the session, sends the gaps back (`cut_repair_prompt`), 3 repairs max.
- A guard's finding applied: same loop; the finding stays when the gates
  still refuse.
- A micro-frontend generated alone: asked again twice while a part is
  unnamed (`retag_prompt`).
- Validation: a part-unnamed finding on a part named since is dropped
  (`Audit::named_since`); a gap is said once, not twice.

Green tests with fake sessions only. **No real AI call made.**

## To verify in a real creation
- [ ] Componentize on a real mock-up: the job's steps show
      "the gates refuse N point(s) — the session fixes them (k of 3)"
      when the first pass leaves gaps, and end on
      "every page is made of micro-frontends, every part named".
- [ ] Then *Validate* on the Rules step passes with no "names no part" gap.
- [ ] A part using a kit element (`data-kit="Button"`): the attribute sits
      on the element in the fragment, the gate counts it.
- [ ] Repair cost: note how many repairs a real cut needs (0–3) and the
      time each takes.
- [ ] A record from before with stored part-unnamed findings, parts named
      by hand since: *Validate* drops those findings.
- [ ] ↻ on a micro-frontend: a fragment missing a part is asked again,
      not written with the "ask for it with a request" warning.
