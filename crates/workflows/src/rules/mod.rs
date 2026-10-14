//! The rules' task: the rules the human validated in the factory's design
//! (contract P, `rules.json`) and the tests that prove them, written before
//! any development, and only here.
//!
//! Triggered by `harness:rules` with `harness:ready` on an open issue. Three
//! kinds open one: the creation's Initialize (every rule, the first task of
//! the roadmap), a rule changed in the view afterwards (the rules again, the
//! changed ones marked `conform`), and a development's `CONTEST.md` (the CI
//! opens `Rules contested: PR #<n>`). While one is open, no development task
//! starts (`common::routing`): the tests are what the code conforms to.
//!
//! **One `harness:ready`, one attempt.** The request is consumed in the free
//! stage, before anything is paid for — the route outranks the dev loop, so
//! a request no session can settle would otherwise hold the whole plant.
//!
//! **Two sessions, never one judge.** The first writes the rules and their
//! tests and opens the pull request; the second, fresh, reads each test
//! against its criterion only and merges when every test is faithful — the
//! human reviewed the rules in the view, so nobody merges by hand. What it
//! finds unfaithful stays on the pull request, and the issue waits on a
//! human.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`): [`data`],
//! [`action`], [`checks`], [`orchestration`]; [`ports`], [`config`] and
//! [`run`] at the root.

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
