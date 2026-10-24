//! One sweep, from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that stays the
//! launcher's work, which also hands in the time: the inside reads no
//! clock.

use harness_core::ports::shell::database::{Dropped, Dropping};
use harness_core::traces::{Event, Logbook};

use crate::scratch_sweep::config::Config;
use crate::scratch_sweep::data::policy::stale;
use crate::scratch_sweep::ports::Ports;

/// What the watch keeps from one sweep to the next.
#[derive(Debug, Default)]
pub struct Sweeps {
    /// When the last sweep ran, in Unix seconds.
    last_at: Option<u64>,
    /// Whether the last sweep failed: its warning is said once per outage.
    failing: bool,
}

/// What one sweep came to.
#[derive(Debug, Default, PartialEq, Eq)]
struct Swept {
    /// The databases dropped.
    dropped: usize,
    /// The ones someone connected to before the drop: kept.
    busy: usize,
    /// The drops the server refused, each with its reason.
    refused: Vec<String>,
    /// The stale databases this sweep did not reach — past its cap or its
    /// deadline: the next sweep's.
    left: usize,
}

/// Sweeps the stale scratch databases, unless the last sweep ran less than
/// `config.every` seconds before `now`.
///
/// Never fails: what was dropped is one `ScratchDatabasesSwept` event, a
/// server that cannot be reached or a drop it refused is one warning — said
/// when the sweeps start failing, not again until one succeeds. The next
/// sweep tries everything again. One sweep drops `config.most` databases at
/// most, within `config.within`: the watch's loop waits for it.
pub async fn sweep(ports: &Ports, config: &Config, now: u64, sweeps: &mut Sweeps, log: &Logbook) {
    if sweeps
        .last_at
        .is_some_and(|last| now.saturating_sub(last) < config.every)
    {
        return;
    }
    sweeps.last_at = Some(now);
    let problem = match once(ports, config, now).await {
        Ok(swept) => {
            if swept.dropped + swept.busy + swept.left > 0 {
                log.event(&Event::ScratchDatabasesSwept {
                    dropped: swept.dropped,
                    busy: swept.busy,
                    left: swept.left,
                });
            }
            swept.refused.first().map(|first| {
                format!(
                    "{} scratch database(s) could not be dropped — {first}",
                    swept.refused.len()
                )
            })
        }
        Err(line) => Some(line),
    };
    match problem {
        Some(line) if !sweeps.failing => {
            log.warn(&format!(
                "{line}; the sweep tries again in {}s",
                config.every
            ));
            sweeps.failing = true;
        }
        Some(_) => {}
        None => sweeps.failing = false,
    }
}

/// One pass: list, decide, drop the stale databases — `config.most` at
/// most, within `config.within`. `Err` is the line to warn with.
async fn once(ports: &Ports, config: &Config, now: u64) -> Result<Swept, String> {
    let owned = ports.databases.owned().await.map_err(|halt| {
        format!(
            "the scratch databases could not be listed — {}",
            halt.reason()
        )
    })?;
    let stale = stale(&owned, &config.keep, now, config.older_than);
    let mut swept = Swept::default();
    if stale.is_empty() {
        return Ok(swept);
    }
    let first = stale.get(..config.most).unwrap_or(&stale);
    let reached = ports
        .databases
        .drop_idle(first, config.within)
        .await
        .map_err(|halt| {
            format!(
                "the stale scratch databases could not be dropped — {}",
                halt.reason()
            )
        })?;
    swept.left = stale.len().saturating_sub(reached.len());
    for Dropping { name, dropped } in reached {
        match dropped {
            Dropped::Gone => swept.dropped += 1,
            Dropped::Busy => swept.busy += 1,
            Dropped::Refused(reason) => swept.refused.push(format!("{name}: {reason}")),
        }
    }
    Ok(swept)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use harness_core::traces::{Sink, Verbosity};

    use super::*;
    use crate::common::fake_databases::FakeDatabases;

    const NOW: u64 = 1_729_770_000;
    /// No database, no line.
    const NONE: [String; 0] = [];

    /// The journal, read back: its lines and its events.
    #[derive(Default)]
    struct Journal {
        lines: RefCell<Vec<String>>,
        events: RefCell<Vec<Event>>,
    }

    impl Sink for Journal {
        fn emit(&self, line: &str) {
            self.lines.borrow_mut().push(line.to_string());
        }
        fn record(&self, event: &Event) {
            self.events.borrow_mut().push(event.clone());
        }
    }

    struct Bench {
        databases: Rc<FakeDatabases>,
        ports: Ports,
        config: Config,
        journal: Rc<Journal>,
        log: Logbook,
        sweeps: Sweeps,
    }

    impl Bench {
        fn new(databases: FakeDatabases) -> Self {
            let databases = Rc::new(databases);
            let journal = Rc::new(Journal::default());
            Self {
                ports: Ports {
                    databases: Rc::clone(&databases) as Rc<_>,
                },
                databases,
                config: Config::new(vec!["forgedesmondes".to_string()]),
                log: Logbook::new(Rc::clone(&journal) as Rc<dyn Sink>, Verbosity::Normal),
                journal,
                sweeps: Sweeps::default(),
            }
        }

        async fn sweep_at(&mut self, now: u64) {
            sweep(&self.ports, &self.config, now, &mut self.sweeps, &self.log).await;
        }

        fn lines(&self) -> Vec<String> {
            self.journal.lines.borrow().clone()
        }

        fn events(&self) -> Vec<Event> {
            self.journal.events.borrow().clone()
        }
    }

    fn old(n: u32) -> String {
        format!("scratch_{}_4242_{n}_label", NOW - 86_400)
    }

    #[tokio::test]
    async fn drops_record_one_swept_event() {
        let mut bench = Bench::new(FakeDatabases::idle(&[
            &old(0),
            &old(1),
            "forgedesmondes",
            &format!("scratch_{NOW}_1_0_young"),
        ]));
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.dropped(), [old(0), old(1)]);
        assert_eq!(
            bench.events(),
            [Event::ScratchDatabasesSwept {
                dropped: 2,
                busy: 0,
                left: 0
            }]
        );
        assert_eq!(
            bench.lines(),
            ["watch: swept 2 stale scratch database(s), 0 busy kept"]
        );
    }

    #[tokio::test]
    async fn the_keep_list_and_the_threshold_come_from_the_config() {
        let mut bench = Bench::new(FakeDatabases::idle(&[&old(0), &old(1)]));
        bench.config.keep = vec![old(1)];
        bench.config.older_than = 86_401;
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.dropped(), NONE);
        bench.config.older_than = 86_400;
        bench.sweep_at(NOW + bench.config.every).await;
        assert_eq!(bench.databases.dropped(), [old(0)]);
    }

    #[tokio::test]
    async fn nothing_stale_drops_nothing_and_records_no_event() {
        let mut bench = Bench::new(FakeDatabases::idle(&["forgedesmondes"]));
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.listings(), 1);
        assert_eq!(bench.databases.passes(), [], "no pass for nothing");
        assert_eq!(bench.events(), []);
        assert_eq!(bench.lines(), NONE);
    }

    #[tokio::test]
    async fn drops_per_sweep_are_capped_and_the_rest_waits_for_the_next() {
        let mut bench = Bench::new(FakeDatabases::idle(&[&old(0), &old(1), &old(2)]));
        bench.config.most = 2;
        bench.sweep_at(NOW).await;
        assert_eq!(
            bench.databases.passes(),
            [(2, bench.config.within)],
            "the cap and the deadline reach the server"
        );
        assert_eq!(bench.databases.dropped(), [old(0), old(1)]);
        assert_eq!(
            bench.events(),
            [Event::ScratchDatabasesSwept {
                dropped: 2,
                busy: 0,
                left: 1
            }]
        );
        bench.sweep_at(NOW + bench.config.every).await;
        assert_eq!(bench.databases.dropped(), [old(0), old(1), old(2)]);
    }

    #[tokio::test]
    async fn a_pass_cut_short_by_its_deadline_says_what_is_left() {
        let mut databases = FakeDatabases::idle(&[&old(0), &old(1), &old(2)]);
        databases.reaches = Some(1);
        let mut bench = Bench::new(databases);
        bench.config.within = std::time::Duration::from_secs(7);
        bench.sweep_at(NOW).await;
        assert_eq!(
            bench.databases.passes(),
            [(3, std::time::Duration::from_secs(7))]
        );
        assert_eq!(
            bench.events(),
            [Event::ScratchDatabasesSwept {
                dropped: 1,
                busy: 0,
                left: 2
            }]
        );
        assert!(bench.lines().iter().all(|l| !l.starts_with("warning")));
    }

    #[tokio::test]
    async fn a_sweep_that_dropped_nothing_still_says_what_it_kept_or_left() {
        let mut databases = FakeDatabases::idle(&[&old(0)]);
        databases.busy = vec![old(0)];
        let mut bench = Bench::new(databases);
        bench.sweep_at(NOW).await;
        assert_eq!(
            bench.events(),
            [Event::ScratchDatabasesSwept {
                dropped: 0,
                busy: 1,
                left: 0
            }]
        );
        let mut databases = FakeDatabases::idle(&[&old(0)]);
        databases.reaches = Some(0);
        let mut bench = Bench::new(databases);
        bench.sweep_at(NOW).await;
        assert_eq!(
            bench.events(),
            [Event::ScratchDatabasesSwept {
                dropped: 0,
                busy: 0,
                left: 1
            }]
        );
    }

    #[tokio::test]
    async fn a_server_lost_between_the_listing_and_the_drops_is_said() {
        let mut databases = FakeDatabases::idle(&[&old(0)]);
        databases.lost = true;
        let mut bench = Bench::new(databases);
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.dropped(), NONE);
        assert_eq!(bench.events(), []);
        assert_eq!(
            bench.lines(),
            [
                "warning: the stale scratch databases could not be dropped — connection refused; \
              the sweep tries again in 600s"
            ]
        );
    }

    #[tokio::test]
    async fn a_sweep_waits_its_interval() {
        let mut bench = Bench::new(FakeDatabases::idle(&["forgedesmondes"]));
        bench.sweep_at(NOW).await;
        bench.sweep_at(NOW + bench.config.every - 1).await;
        assert_eq!(bench.databases.listings(), 1, "too soon: nothing listed");
        bench.sweep_at(NOW + bench.config.every).await;
        assert_eq!(bench.databases.listings(), 2, "due: listed again");
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.listings(), 2, "a clock gone back waits too");
    }

    #[tokio::test]
    async fn busy_on_drop_is_counted_not_fatal() {
        let mut databases = FakeDatabases::idle(&[&old(0), &old(1)]);
        databases.busy = vec![old(0)];
        let mut bench = Bench::new(databases);
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.dropped(), [old(1)]);
        assert_eq!(
            bench.events(),
            [Event::ScratchDatabasesSwept {
                dropped: 1,
                busy: 1,
                left: 0
            }]
        );
        assert!(bench.lines().iter().all(|l| !l.starts_with("warning")));
    }

    #[tokio::test]
    async fn a_refused_drop_is_said_and_the_others_still_go() {
        let mut databases = FakeDatabases::idle(&[&old(0), &old(1), &old(2)]);
        databases.refused = vec![old(0), old(1)];
        let mut bench = Bench::new(databases);
        bench.sweep_at(NOW).await;
        assert_eq!(bench.databases.dropped(), [old(2)]);
        assert_eq!(
            bench.lines(),
            [
                "watch: swept 1 stale scratch database(s), 0 busy kept".to_string(),
                format!(
                    "warning: 2 scratch database(s) could not be dropped — {}: permission \
                     denied; the sweep tries again in 600s",
                    old(0)
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_listing_is_said_once_per_outage_and_the_watch_goes_on() {
        let databases = FakeDatabases::idle(&[&old(0)]);
        databases.down.set(true);
        let mut bench = Bench::new(databases);
        let every = bench.config.every;
        bench.sweep_at(NOW).await;
        bench.sweep_at(NOW + every).await;
        assert_eq!(bench.databases.listings(), 2, "tried again when due");
        assert_eq!(
            bench.lines(),
            [
                "warning: the scratch databases could not be listed — connection refused; the \
              sweep tries again in 600s"
            ]
        );
        bench.databases.down.set(false);
        bench.sweep_at(NOW + 2 * every).await;
        assert_eq!(bench.databases.dropped(), [old(0)]);
        bench.databases.down.set(true);
        bench.sweep_at(NOW + 3 * every).await;
        assert_eq!(
            bench
                .lines()
                .iter()
                .filter(|l| l.starts_with("warning"))
                .count(),
            2,
            "a new outage is said again: {:?}",
            bench.lines()
        );
    }
}
