# The scope in named sections

## Context
The proof of concept's scope was one Markdown textarea; only `## Screens`
was read by the code (the drawing), and a scope without it failed late.

## Shipped
- `SCOPE_SECTIONS` (Goal, Users & roles, Screens — required, Journeys,
  Data, Integrations, Left out, Proof, Notes), `scope_prompt` asking one
  JSON key per section, `parse_scope` composing the `## ` text.
- Design step gate: "list at least one screen under Screens".
- `creation.js`: one field per section, a `?` each, Notes free with
  `### ` sub-sections; `composeParts`/`splitParts` shared with the brief.

## To verify in a real creation
- [ ] Entering the scope: the AI's proposal fills the 9 fields, Screens
      among them; no field shows a heading.
- [ ] Each `?` opens its help and closes it; open ones survive a refresh.
- [ ] Empty Screens: *Continue* is greyed at once, with its title; filled,
      it opens again without a reload.
- [ ] A creation started before this change: its scope falls into the right
      fields, anything unknown in Notes as `### ` sub-sections.
- [ ] *Propose again* after an edit asks to confirm; untouched, it does not.
- [ ] The new Users & roles and Integrations sections: do the rules and the
      ports steps read them (a role in a rule, a provider proposed)?
- [ ] Drawing after *Continue*: one page per Screens bullet.
