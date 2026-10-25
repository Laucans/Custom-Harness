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

- [x] dnd-like brief (campaign, player characters, combat): round 1 lands in
      ≥ 3 topic cards plus a *General* one. (seen 2026-10-16 22:04 Paris, ForgeDesMondes' record (9e6e4db mock/proof-of-concept.json): round 1 filed under Préparation de campagne, Micro et suggestions IA, Médias, plus 4 under General)
- [x] Topics are written in the brief's language. (seen same record: topics in French; one, `DM view`, keeps the brief's own term)
- [x] Round 2 reuses the round-1 topics — no near-synonym card
      (*Combat* / *Combats* / *Fights*). (seen same record: 5 rounds, 8 topics, later rounds reuse them, no near-synonym card)
- [x] No decision asked twice across two cards, in a round or across rounds. (seen same record: 38 distinct question titles, none repeated across cards)
- [ ] An old creation record (questions without `topic`) shows them all
      under *General*, and answering still saves.
- [ ] Answering, *use it*, delete and *use the N recommendations* work inside
      the cards; the record order after a save follows the cards.
- [ ] *Continue grilling*: once the round lands, the page scrolls to the
      first open question (just under its topic's header); a new topic card
      may appear.
- [ ] While a round is asked (first one or *Continue grilling*), the button
      spins and reads *Generating the round…*; back to normal once it lands
      or fails.
- [ ] Scrolling down a topic, its header stays stuck at the pane's top edge,
      then gives way to the next topic's.
- [ ] The new round's questions are yellow; typing in one (or *use it*,
      *use the N recommendations*) greys it at once; after a reload the
      answered ones stay grey.
- [x] The roadmap issue opened at the end shows `### General` then one
      `### <topic>` per topic with decisions. (seen 2026-10-17 00:05 Paris: issue #4 folded Decisions into mock/proof-of-concept.md for size; there `### General` then 8 `### <topic>`)
