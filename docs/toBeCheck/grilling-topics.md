# Grilling questions sorted into topic cards

Shipped 2026-10-13, not yet seen in a real creation.

## What shipped

Each grilling question carries a `topic` (`Question::topic`, `Asked::topic`;
empty = the product as a whole). `grill_prompt` asks the AI to file every
question under a topic, reuse the topics in use (listed), and never ask one
decision under two topics. `add_round` folds a topic differing only by case
onto the one already given. The grilling step (`creation.js`,
`stepGrilling`) draws one card per topic, *General* first; the roadmap
issue's `## Decisions` gets one `###` per topic when there are topics.
Still one AI call per round.

Verified by unit tests only (domain + server); the prompt's effect on a
real model is unverified.

## To check in a real creation

- [ ] dnd-like brief (campaign, player characters, combat): round 1 lands in
      ≥ 3 topic cards plus a *General* one.
- [ ] Topics are written in the brief's language.
- [ ] Round 2 reuses the round-1 topics — no near-synonym card
      (*Combat* / *Combats* / *Fights*).
- [ ] No decision asked twice across two cards, in a round or across rounds.
- [ ] An old creation record (questions without `topic`) shows them all
      under *General*, and answering still saves.
- [ ] Answering, *use it*, delete and *use the N recommendations* work inside
      the cards; the record order after a save follows the cards.
- [ ] The roadmap issue opened at the end shows `### General` then one
      `### <topic>` per topic with decisions.
