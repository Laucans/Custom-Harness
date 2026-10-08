# harness

A Rust agent harness: it drives Claude Code sessions through verification
gates. A workflow declares its stages, each stage runs a session or a local
action, and gates before and after decide whether the run goes on, skips or
stops. The first workflow family is software development, driven from a
GitHub board: roadmap → milestones → tasks → PRs → merges, with a human in
the loop at the labels.

## Layout

| crate | role |
| --- | --- |
| `crates/core` | the framework: vocabulary, traces, execution shapes, ports, adapters |
| `crates/workflows` | the workflows declared against it: `dev_loop`, `refinement`, `planner`, `split`, `pr_review`, `pr_fix` |
| `crates/launcher` | the `harness` binary: CLI, the `watch` router, the place that builds adapters |
| `crates/view` | the factory view: a local page showing the plant, read from the traces |
| `crates/view-render` | the drawing: a Bevy scene compiled to WebAssembly |

The design is hexagonal and it is a rule, not a style: `ARCHITECTURE_OVERVIEW.md`
has the diagram, `CLAUDE.md` the rules a diff is checked against, and each
crate's `ARCHITECTURE.md` the detail.

## Getting started

```sh
git config core.hooksPath .githooks       # once per clone
cp .env.example .env.local                # fill in TARGET_REPO_URL at least
cargo build
cargo run -- --help                       # every flag and its env var
cargo run -- init-repo <url>              # labels, branch, CI on the target
cargo run -- watch                        # poll the board, run what is due
cargo run -p harness-view                 # the plant, on 127.0.0.1:7878
```

Before a commit: `cargo fmt --all`, then
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo test --workspace --all-features`. The hooks run the formatter on
commit and clippy on push.

Everything a run leaves behind lands under `.llocal/` (gitignored): logs
and ledgers in `.llocal/logs/<workflow>/`, mounted clones in
`.llocal/agentic_workspaces/` unless `AGENTIC_WORKSPACES_DIR` says otherwise.
`harness doctor` reads the last failure, repairs what it knows how to, and
names what sits in `.llocal/` that the harness did not put there.

## Reusing the framework

Another workflow family — a consulting report, a research digest — depends
on `harness-core` only and declares its own states, stages and ports.
`crates/workflows/ARCHITECTURE.md` § 14 walks through adding one.
