# Lot D: the disk budget of a parallel watch

Built 2026-10-24 on lane B (`lane-b/h3-h5-h7`, on f58d1d6). Not committed
yet, not yet seen in a live watch. Source: `harness-dnd2-run/night/priorities-H.md`,
entry `D`.

Measured before: five lanes each build the product in
`harness-workspaces/lane-N/target`; with incremental on a live lane target
reached 8.2 GB (~40 GB for five); on 2026-10-24 22:28 four cold lane
builds at once took the disk to 1 GB free. With sccache and incremental
off, a cold product build is 3.4 GB of `target` and 54 s, a warm one 44 s.

## What shipped

- **A shared compilation cache** (`crates/launcher/src/toolchain.rs`). When
  `sccache` is found (composed `PATH`, else `/opt/homebrew/bin`,
  `/usr/local/bin`), every process the harness spawns for the product and
  every Claude session gets `RUSTC_WRAPPER=<full path of sccache>`,
  `SCCACHE_DIR` (default `~/.harness/sccache`; a relative one is read from
  the home), `SCCACHE_CACHE_SIZE` (default `15G`), `CARGO_INCREMENTAL=0`
  and `SCCACHE_IGNORE_SERVER_IO_ERROR=1` (a server out of reach: the client
  runs rustc itself), through the H1-A seam (`Processes::with_env`,
  `ClaudeCliFactory::with_env`). Only a sccache whose `--version` exits 0
  is wired (`none, sccache does not run (…)` otherwise). `SCCACHE=off` in
  `.env.local` sets none of them; off or without a sccache that runs, an
  inherited `RUSTC_WRAPPER` is cleared (set empty). The journal's toolchain
  line names every variable, values in full: `… · SHELL=… CLAUDE_CODE_SHELL=…
  · build cache: sccache 0.18.0 — RUSTC_WRAPPER=/opt/homebrew/bin/sccache
  SCCACHE_DIR=/Users/…/.harness/sccache SCCACHE_CACHE_SIZE=15G
  CARGO_INCREMENTAL=0 SCCACHE_IGNORE_SERVER_IO_ERROR=1; inherited: … ·
  PATH=…` (`off (SCCACHE=off)`, or `none, sccache not found`).
- **The busy probe sees sccache.** The sccache server daemonizes, so a
  compile runs outside the session's process tree and its CPU time is not
  the session's. A `sccache` client in the tree now counts as busy
  (`adapters/shell/process.rs::tree_activity`), so a silent cold build is
  waited on, not killed as hung.
- **A disk floor** (`crates/launcher/src/router/floor.rs`, core port
  `ports/shell/volume.rs::Volume`, adapter `adapters/shell/volume.rs::Statvfs`).
  Every tick measures the free space of the lanes' workspaces folder
  (`AGENTIC_WORKSPACES_DIR`, else the factory). Under `DISK_FLOOR_GB`
  (default 12, `0` none) no lane starts; the merges, run in the tick, go
  on; with `--parallel 1` nothing is dispatched. Said once when it falls
  (`warning: idle: disk under the floor (N GB < M GB) — no lane starts until
  space is freed`, `Event::DiskUnderFloor`), once when it is back
  (`disk: back above the floor (N GB >= M GB) — lanes start again`). Notify:
  an error `The disk is under its floor`, cleared by the second or by the
  next `WatchStarted` (a new watch starts unheld). Under the floor the free
  slots' targets are pruned first and the volume measured again. A held
  floor whose volume cannot be measured stays held.
- **A lane target cap** (`crates/launcher/src/dispatch/lanes.rs::prune_target`,
  called by `router/lanes.rs::prune_slot` before a slot takes its next job,
  through the `Disk` port, which gained `size_bytes` and `modified_secs`).
  Over `LANE_TARGET_CAP_GB` (default 6, `0` never): `target/debug/incremental`
  and every executable a later build session superseded
  (`debug/deps/<name>-<16 hex>` more than 6 h older than the newest of its
  name, with its `.d` and objects; never a name uplifted into `debug/`);
  over twice the cap, `target/debug` and `target/release` —
  `target/rules-runtime` and `target/rules-status.jsonl` stay. Nothing
  removed and nothing refused: nothing said. Both the slot's checkouts: `lane-<k>` (tasks,
  under `AGENTIC_WORKSPACES_DIR`) and `router-lane-<k>` (routes, under
  `.llocal/agentic_workspaces`). Said as `lane K: <target> pruned X GB -> Y GB
  (<what>)`. A dry run prunes nothing.
- `.env.example` declares `DISK_FLOOR_GB=12`, `LANE_TARGET_CAP_GB=6`,
  `SCCACHE=`, `SCCACHE_DIR=`, `SCCACHE_CACHE_SIZE=`; empty values take the
  defaults.

## To check live

Before: `sccache --stop-server` (the running server keeps the size it
started with — 10 GiB seen tonight — until it restarts), `sccache
--zero-stats`, and note `df -g /` and `du -sh harness-workspaces/lane-*/target`.

- [ ] **The toolchain line.** Restart the watch; `watch.log` and the first
      lane's `run.log` show `SHELL=… CLAUDE_CODE_SHELL=… · build cache:
      sccache 0.18.0 — RUSTC_WRAPPER=/opt/homebrew/bin/sccache
      SCCACHE_DIR=/Users/…/.harness/sccache SCCACHE_CACHE_SIZE=15G
      CARGO_INCREMENTAL=0 SCCACHE_IGNORE_SERVER_IO_ERROR=1`, and `;
      inherited: …` lists what the watch's own environment passes on.
- [ ] **The builds use it.** After the first two lane builds, `sccache
      --show-stats`: `Compile requests` > 0, `Cache hits` > 0 on the second
      build, `Max cache size 15 GiB`. A lane's session Bash: `echo
      $RUSTC_WRAPPER $CARGO_INCREMENTAL` prints the sccache path and `0`.
- [ ] **No incremental grows.** After a task, `du -sh
      harness-workspaces/lane-N/target/debug/incremental` is absent or a few
      MB; a lane target after a cold build ~3.4 GB (not 7-8 GB). A cold build
      ~54 s, a warm one ~44 s (the session's `cargo build` timing in its log).
- [ ] **Off-switch.** `SCCACHE=off` in `.env.local`, restart: the line says
      `build cache: off (SCCACHE=off)` (with `RUSTC_WRAPPER= (inherited …
      cleared)` if the watch inherited one) and a session's `echo
      $RUSTC_WRAPPER` is empty. Put it back.
- [ ] **A server out of reach.** `SCCACHE_SERVER_PORT` set to a taken port
      in `.env.local`, restart: lane builds still pass (slower), no
      `sccache: error` failing a build. Remove it.
- [ ] **A silent cold build is not killed.** A lane's first build after a
      `rm -rf` of its target, with the cache cold (`SCCACHE_DIR` pointed at
      an empty folder): the session's log may say `silent for …s but its
      processes work on — waited on`, never `killed as hung`.
- [ ] **The floor.** `DISK_FLOOR_GB` set above the free space (e.g. free
      30 GB → `DISK_FLOOR_GB=40`), restart: one `warning: idle: disk under
      the floor (30 GB < 40 GB)` line, no `LaneTaken` while it holds, the
      merges still run; the view shows the error `The disk is under its
      floor`. Back to 12 and restart: the restart clears the notification
      (`WatchStarted`), no new `idle: disk under the floor` line, lanes start
      on the first tick.
- [ ] **The floor prunes what the watch owns.** Under the floor with an
      idle lane whose target is over the cap: that tick says `lane K: …
      pruned …` before deciding, and lifts the floor if it freed enough.
- [ ] **The cap.** A lane target over 6 GB (one from before this lot, or
      `LANE_TARGET_CAP_GB=2` for the test): when that slot takes its next
      job, `lane K: …/lane-K/target pruned 8.2 GB -> … GB (incremental
      caches, N stale executable(s))`, or `(the debug profile, above twice
      the cap)` above 12 GB (4 GB with the test cap). The task still builds
      and its gates pass after the prune; `target/rules-runtime` is still
      there, and `scripts/rules-runtime.sh stop` still finds its server. No
      line for a target over the cap with nothing to remove.
- [ ] **The budget overall.** Five lanes after a milestone split: `df -g /`
      never under the floor; the five lane targets together ≤ 5 × 6 GB, and
      `du -sh ~/.harness/sccache` ≤ 15 GB.
