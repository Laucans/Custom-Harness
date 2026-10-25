# The toolchain said and used, and a conflicting landing PR that no longer halts

## Context
ForgeDesMondes, H1 (round A + T + L):
- The watch's `PATH` put Homebrew first: cargo 1.98.1 while the product
  pins 1.99.1, `sed` = BSD `/usr/bin/sed`, sessions on zsh. BSD sed read
  `\s` in `scripts/queries.sh` as a literal `s` and wrote a wrong
  `queries/registry.json` (`notes` → `note`, `status` → `statu`) while
  exiting 0.
- #423/#442: a green milestone PR that conflicted with `main_agent` reached
  `merge_pr`; GitHub's refusal came back as a `Halt` and stopped the watch.
- The owner asked to see easily what is on the `PATH` the harness uses.

## Shipped (not committed yet)
- `crates/architecture/assets/queries.sh`: `[[:space:]]`, no `globstar`,
  `shasum -a 256` fallback; `tests/queries_script.rs` against a golden
  registry, on macOS under `PATH=/usr/bin:/bin` with `/bin/bash`.
- `crates/workflows/src/main_agent_merge/action/apply.rs`: `pr_mergeable`
  before `merge_pr`; `Conflicting` (with why) / `MergeabilityUnread` /
  `MergeRefused` outcomes, warnings in the watch journal, never a `Halt`.
- `crates/launcher/src/toolchain.rs`: one `PATH` — the shims
  `<factory>/.llocal/toolchain/bin` (`sed`→`gsed`, `grep`→`ggrep`,
  `bash`→bash ≥ 5, `node`→node ≥ 22, links, not folders), the product's
  pinned cargo (else rustup's proxies: `$CARGO_HOME/bin` or Homebrew's
  rustup keg), then the inherited `PATH` in its own order — and `SHELL` /
  `CLAUDE_CODE_SHELL`, handed to `Processes::with_env` and
  `ClaudeCliFactory::with_env` by the launcher;
  `Event::ToolchainResolved` at watch start and at every run's mount (a pin
  that cannot be read is a problem, never "unpinned"); `harness toolchain`.
  `.llocal/toolchain` is a known store for the janitor and the doctor.
- `crates/architecture/assets/dataguard-server/src/lib.rs`: `woken_applier`
  + `waking` (the product's own design, backported); `applier(engine)`
  kept. `assets/server/src/main.rs` uses the woken one.

## To check
- [x] The watch journal's first lines after a restart carry
      `toolchain: bash 5.x (/opt/homebrew/bin/bash) · cargo 1.99.1
      (~/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin, pinned 1.99.1) ·
      sed GNU 4.9 (shim→/opt/homebrew/bin/gsed) · grep GNU 3.11 (shim→…) ·
      node v22.x (…) · PATH=…` and no `problems:`. Same line printed by
      `harness toolchain` from the factory. (seen 2026-10-24 20:39 Paris: `toolchain: bash 5.3.15 (shim→/opt/homebrew/bin/bash) · cargo 1.99.1 (… pinned 1.99.1) · sed GNU 4.9 · grep GNU 3.11 · node v26.0.0 · PATH=…`, no `problems:` anywhere in watch.log; `harness toolchain` from the factory not compared)
- [x] A lane's `run.log` (any route) carries the same line, right after its
      mount; the pinned cargo, not Homebrew's 1.98.1. (seen 2026-10-24 22:15 Paris: run 20261024-201500-521 carries the same line right after its mount, cargo 1.99.1 pinned)
- [ ] A session's first Bash call runs under bash: in its `session.log`,
      `echo $BASH_VERSION` (or `ps -p $$`) answers bash 5, not zsh.
      (`CLAUDE_CODE_SHELL` is read by the installed CLI 2.1.292 — a path
      containing `bash` or `zsh`, executable.)
- [x] The composed `PATH` survives inside a session. Claude Code builds a
      shell snapshot from the user's rc file (`~/.bashrc` here: Fig, Docker's
      `init-bash.sh` prepending `~/.docker/bin`); a login shell would also run
      `/etc/profile`'s `path_helper` (a scratch `bash -l` put `/usr/bin:/bin`
      first and the shims last) and `~/.bash_profile`'s conda init. In a
      session's first Bash: `command -v sed cargo node`,
      `sed --version | head -1`, `echo $PATH | tr : '\n' | head -3`.
      Expected: the shims folder and the pinned toolchain's `bin` before
      `/usr/bin`, `sed (GNU sed) 4.9`. If reordered: consider `BASH_ENV` or
      `CLAUDE_CODE_SHELL_PREFIX` to put the composed folders back first.
      Note: a session's interactive `grep` is Claude Code's own shell
      function (its built-in ugrep), never the GNU shim, whatever the
      `PATH`; scripts run as `bash scripts/x.sh` do get the shim. (seen 2026-10-24 15:19 Paris: #503's session ran `which sed; sed --version; which cargo` -> `.llocal/toolchain/bin/sed`, `sed (GNU sed) 4.9`, `~/.rustup/toolchains/1.99.1-…/bin/cargo`; the PATH head itself not printed)
- [ ] The janitor (sweep strays on) never proposes `.llocal/toolchain`, and
      `harness doctor` does not call it a leftover.
- [ ] An inline catch-up's `cargo metadata` (watch journal, `catch-up`
      checkout) rewrites `Cargo.lock` with the pinned cargo: CI accepts the
      lock it pushes.
- [ ] The next conflicting landing PR: the watch journal says
      `main_agent_merge -> PR #n of #m conflicts with the integration
      branch: not merged, it waits for a catch-up — <why>`, every tick, and
      the watch keeps running (no `main_agent_merge` halt in the error
      ledger).
- [ ] A `gh` outage during a landing: `whether PR #n of #m can merge cannot
      be read: …` each tick, never silence.
- [ ] A merge GitHub refuses (branch protection, a race) is one warning per
      tick (`GitHub refused to merge PR …`), not a halt.
- [ ] A product created after this change: its server applies a command
      within the second it is posted (two tabs, one sees the other's write
      at once), and `cargo test -p dataguard-server` passes there.
