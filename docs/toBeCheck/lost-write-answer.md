# A write whose answer `gh` lost

## Context
2026-10-20 14:13Z, ForgeDesMondes: `decide` on #125 halted on
"GitHub refused to comment … unexpected end of JSON input", while the comment
had landed (14:13:49Z).

## What shipped
- A lost answer to PATCH/PUT/DELETE is sent once more.
- A lost answer to a comment POST is checked against the issue's latest
  comment. The post stands if the comment is there, and is never repeated.

## To verify on a live watch
- [ ] The next `unexpected end of JSON input` on a comment does not halt
      the workflow, and the thread has a single copy of the comment.
