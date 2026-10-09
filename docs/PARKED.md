# Parked questions

Decisions deliberately left for later, with what is already known. Each one
names who parked it and when; whoever picks it up removes it from here.

## How does an *asking* pin reach the next roadmap?

*Parked 2026-10-09, by the human, while building the pins' kinds.*

An **asking** is a pin on a board (the data model, for now) that holds a
change the human wants considered "when the next roadmap is built" — a
`request_change`. It has an AI session behind it, like a brainstorm; its
text is the request the AI formulated (or the human wrote).

Today it lives only in the view's store (`.llocal/harness.db`, table
`notes`, `kind = "asking"`), and nothing in the harness reads it.

Options on the table:

- **The planner reads the askings** when it builds a roadmap's milestones,
  through a port onto the pins' store, and marks each one consumed. The
  view would then write something a run reads — an exception to its
  read-only rule, like the prompt drafts.
- **An asking becomes a GitHub issue** (`harness:request-change`), opened
  from the pin on a click; the planner reads the label. Visible on the
  board, nothing new for the harness to read.
- **Something else** — e.g. a roadmap pass that collects the askings and
  asks the human to sort them.

To decide with it: when "the next roadmap" is built (the planner on a new
`harness:roadmap` issue? a dedicated step?), whether an asking is consumed
once or stays as a standing request, and what the human sees of it after.
