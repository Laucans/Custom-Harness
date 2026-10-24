//! Launcher wiring for the `scratch_sweep` deterministic command: the stale
//! scratch databases a product's tests left behind, dropped from the watch.
//!
//! Built once when the watch starts, from `DATABASE_URL` (the role that
//! owns the tests' databases, and the server it reaches) and
//! `DATABASE_URL_READONLY` (whose database is kept, like `DATABASE_URL`'s).
//! No `DATABASE_URL`, no sweep. The watch's loop calls [`Sweeper::sweep`]
//! after every tick with the time; the command decides whether it is due.

use std::rc::Rc;

use harness_adapters::shell::database::{PgDatabases, database_of};
use harness_core::traces::Logbook;
use harness_workflows::scratch_sweep::config::Config;
use harness_workflows::scratch_sweep::ports::Ports;
use harness_workflows::scratch_sweep::run::{Sweeps, sweep};

/// The watch's sweep: its adapter, its settings, what it keeps between two
/// sweeps.
pub struct Sweeper {
    ports: Ports,
    config: Config,
    sweeps: Sweeps,
}

impl Sweeper {
    /// The sweep of the server `database_url` names, keeping its database
    /// and `readonly_url`'s; `None` without a `database_url`.
    #[must_use]
    pub fn of(database_url: Option<String>, readonly_url: Option<&str>) -> Option<Self> {
        let url = database_url?;
        let keep = [Some(url.as_str()), readonly_url]
            .into_iter()
            .flatten()
            .filter_map(database_of)
            .collect();
        Some(Self {
            ports: Ports {
                databases: Rc::new(PgDatabases::new(url)),
            },
            config: Config::new(keep),
            sweeps: Sweeps::default(),
        })
    }

    /// Sweeps if a sweep is due at `now` (Unix seconds). Never fails: what
    /// happened is in `log`.
    pub async fn sweep(&mut self, now: u64, log: &Logbook) {
        sweep(&self.ports, &self.config, now, &mut self.sweeps, log).await;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use harness_core::traces::{Sink, Verbosity};

    use super::*;

    #[derive(Default)]
    struct Lines(RefCell<Vec<String>>);

    impl Sink for Lines {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[test]
    fn no_database_url_no_sweep() {
        assert!(Sweeper::of(None, Some("postgres://r@h/app")).is_none());
    }

    #[test]
    fn both_urls_databases_are_kept() {
        let sweeper = Sweeper::of(
            Some("postgres://app:pw@127.0.0.1:5432/forgedesmondes".to_string()),
            Some("postgres://ro:pw@127.0.0.1:5432/forgedesmondes_read?sslmode=disable"),
        )
        .expect("a sweep");
        assert_eq!(
            sweeper.config.keep,
            ["forgedesmondes", "forgedesmondes_read"]
        );
        let alone = Sweeper::of(Some("postgres://app@h/app".to_string()), None).expect("a sweep");
        assert_eq!(alone.config.keep, ["app"]);
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_one_warning_and_the_url_stays_untold() {
        // Port 1 serves nothing on a developer machine or a CI runner.
        let mut sweeper = Sweeper::of(
            Some("postgres://app:secret@127.0.0.1:1/forgedesmondes".to_string()),
            None,
        )
        .expect("a sweep");
        let lines = Rc::new(Lines::default());
        let log = Logbook::new(Rc::clone(&lines) as Rc<dyn Sink>, Verbosity::Normal);
        sweeper.sweep(1_729_770_000, &log).await;
        let said = lines.0.borrow().clone();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("warning: the scratch databases could not be listed"));
        assert!(!said[0].contains("secret"), "{said:?}");
    }
}
