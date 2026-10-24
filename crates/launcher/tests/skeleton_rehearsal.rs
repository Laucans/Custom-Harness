//! The dev loop's `Scaffold`, wired as the composition root wires it —
//! `RealDisk`, `Processes`, `GitRepos` — and run on a real checkout of a
//! product: the rehearsal of a skeleton before a night uses it.
//!
//! Ignored, and a no-op unless asked: `SKELETON_PRODUCT` names a clean
//! scratch clone of the product (never a lane's checkout: the skeleton is
//! written into it), `SKELETON_BODY` a file holding the task's body with
//! its `## Architecture`. It prints what the journal says and the files
//! written, which stay for the rehearsal to read:
//! `SKELETON_PRODUCT=… SKELETON_BODY=… cargo test -p harness-launcher
//! --test skeleton_rehearsal -- --ignored --nocapture`.

use std::path::PathBuf;
use std::rc::Rc;

use harness_adapters::shell::disk::RealDisk;
use harness_adapters::shell::git::GitRepos;
use harness_adapters::shell::process::Processes;
use harness_core::domain::Named;
use harness_core::execution::{Action, Context, Settings};
use harness_core::ports::shell::git::Repos;
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::dev_loop::action::scaffold::Scaffold;
use harness_workflows::dev_loop::data::state::Loop;

/// The journal, on the test's output.
struct Printed;

impl Sink for Printed {
    fn emit(&self, line: &str) {
        println!("{line}");
    }
}

#[tokio::test]
#[ignore = "writes a skeleton into SKELETON_PRODUCT; run on demand"]
async fn a_skeleton_is_written_into_a_real_checkout() {
    let (Ok(product), Ok(body)) = (
        std::env::var("SKELETON_PRODUCT"),
        std::env::var("SKELETON_BODY"),
    ) else {
        println!("SKELETON_PRODUCT and SKELETON_BODY are not both set: nothing rehearsed");
        return;
    };
    let root = PathBuf::from(product);
    let body = std::fs::read_to_string(&body).expect("the task's body");
    let mut ctx = Context::new(
        Settings {
            dry_run: false,
            stages: String::new(),
        },
        Loop {
            task: Named {
                number: "0".to_string(),
                title: "rehearsal".to_string(),
                body,
            },
            ..Loop::default()
        },
        Logbook::new(Rc::new(Printed), Verbosity::Verbose),
    );
    Scaffold {
        repo: GitRepos.at(&root),
        disk: Rc::new(RealDisk),
        process: Rc::new(Processes::default()),
        root,
    }
    .run(&mut ctx)
    .await
    .expect("never stops the round");
    for path in &ctx.state.skeleton {
        println!("written: {path}");
    }
}
