# harness-notify

What the plant should tell a human, and how loud. A library with no I/O: it
decides, a reader shows.

```
events (harness-core EventLog)  ─┐
                                 ├─▶ feed() ─▶ Vec<Notification>  (loudest, then newest)
facts (watch process, board,     │
       Claude's windows)        ─┘
```

## Two sources, and why

- **Events** — what the harness itself recorded, once, where it happened
  (`harness_core::traces::Event`, kept in `.llocal/harness.db`): a lane that
  ended, a task parked for a human, the breaker refusing, a halt, a board
  read that failed, the watch's own life.
- **Facts** — what no part of the harness is there to tell: a watch process
  that is gone (a dead process says nothing), an issue a human labelled, a
  subscription window nearly spent. The reader observes them and passes them
  in (`Facts`).

## The modules

```
src/notification.rs  Level, Link, Notification; Board merges repeats by key
src/rules.rs         events → notifications, one rule per event, keyed so repeats count up and an end clears
src/facts.rs         Facts, and which board issues wait on a human
src/feed.rs          both together: the watch gone without a word, the windows, the waiting issues
```

## Decisions

- **Keyed, not listed.** `failing:#15` told twelve times is one notification
  counting twelve, at its latest words and its loudest level. A success
  clears it.
- **One warning per stop.** A lane that stops for a human ends with exit 1,
  is parked, and its dev loop halts: three events, one notification
  (`parked:#n`).
- **Pure.** No clock, no store, no GitHub: `Facts::now` and the events are
  given. Desktop or chat notifications later reuse `feed` unchanged.
