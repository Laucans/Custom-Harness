//! The janitor at work: the yard weighed on a click or every two hours, swept
//! on a click or on their own past the threshold — one job at a time.
//!
//! The rules are `domain::cleanup`'s and the disk is the `Yard` port's; this
//! is the part with a clock and threads. A walk of a few gigabytes takes
//! seconds, so it runs on a blocking thread and never holds up the page.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;

use crate::domain::cleanup::{self, Diagnosis, Failed, Report, Settings};
use crate::ports::Yard;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The clock, as the traces write it.
fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// What the janitor is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Doing {
    /// Leaning on their broom.
    #[default]
    Idle,
    /// Weighing the yard.
    Diagnosing,
    /// Sweeping it.
    Sweeping,
}

#[derive(Default)]
struct Book {
    settings: Settings,
    diagnosis: Option<Diagnosis>,
    report: Option<Report>,
    doing: Doing,
    next_at: String,
}

/// What the page reads of the janitor.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// The janitor is here at all.
    pub available: bool,
    /// What a human allowed.
    pub settings: Settings,
    /// The last weighing.
    pub diagnosis: Option<Diagnosis>,
    /// The last sweep.
    pub report: Option<Report>,
    /// What they are doing now.
    pub doing: Doing,
    /// When the chronic weighing comes next.
    pub next_at: String,
}

/// The janitor, shared by the page and the chronic tick.
#[derive(Clone)]
pub struct Janitor {
    yard: Arc<dyn Yard>,
    book: Arc<Mutex<Book>>,
    /// Held for a whole job: a sweep never runs beside a weighing.
    job: Arc<tokio::sync::Mutex<()>>,
}

impl Janitor {
    /// The janitor in this yard, with the settings they kept there — or the
    /// defaults when they kept none, or kept nonsense.
    #[must_use]
    pub fn new(yard: Arc<dyn Yard>) -> Self {
        let settings = yard
            .read_settings()
            .and_then(|json| Settings::parse(&json).ok())
            .unwrap_or_default();
        Self {
            yard,
            book: Arc::new(Mutex::new(Book {
                settings,
                ..Book::default()
            })),
            job: Arc::default(),
        }
    }

    /// What the page shows.
    #[must_use]
    pub fn status(&self) -> Status {
        let book = lock(&self.book);
        Status {
            available: true,
            settings: book.settings.clone(),
            diagnosis: book.diagnosis.clone(),
            report: book.report.clone(),
            doing: book.doing,
            next_at: book.next_at.clone(),
        }
    }

    /// The settings now in force.
    #[must_use]
    pub fn settings(&self) -> Settings {
        lock(&self.book).settings.clone()
    }

    /// Takes new settings, keeps them on disk, and re-reads the last
    /// weighing against them — no new walk.
    ///
    /// # Errors
    ///
    /// The settings make no sense, or could not be kept.
    pub fn set(&self, settings: Settings) -> Result<Status, String> {
        settings.check()?;
        self.yard.write_settings(&settings.to_json())?;
        lock(&self.book).settings = settings;
        Ok(self.status())
    }

    /// When the chronic weighing comes next, for the page to say.
    pub fn plan(&self, at: String) {
        lock(&self.book).next_at = at;
    }

    /// Weighs the yard and judges it.
    pub async fn diagnose(&self) -> Diagnosis {
        let _job = self.job.lock().await;
        self.weigh().await
    }

    async fn weigh(&self) -> Diagnosis {
        lock(&self.book).doing = Doing::Diagnosing;
        let yard = Arc::clone(&self.yard);
        let survey = tokio::task::spawn_blocking(move || yard.survey())
            .await
            .unwrap_or_default();
        let mut book = lock(&self.book);
        let diagnosis = cleanup::diagnose(&survey, &book.settings);
        book.diagnosis = Some(diagnosis.clone());
        book.doing = Doing::Idle;
        diagnosis
    }

    /// Weighs the yard afresh, removes what the weighing says goes, and
    /// weighs it again so the pie shows what is left.
    pub async fn sweep(&self, automatic: bool) -> Report {
        let _job = self.job.lock().await;
        let diagnosis = self.weigh().await;
        lock(&self.book).doing = Doing::Sweeping;
        let yard = Arc::clone(&self.yard);
        let plan = diagnosis.sweep;
        let mut report = tokio::task::spawn_blocking(move || {
            let mut report = Report {
                automatic,
                ..Report::default()
            };
            for heap in plan {
                match yard.remove(&heap.rel) {
                    Ok(bytes) => {
                        report.freed = report.freed.saturating_add(bytes);
                        report.removed.push(heap);
                    }
                    Err(why) => report.failed.push(Failed { rel: heap.rel, why }),
                }
            }
            report
        })
        .await
        .unwrap_or_default();
        report.at = now();
        lock(&self.book).report = Some(report.clone());
        self.weigh().await;
        report
    }

    /// The chronic tick: weigh, and sweep when allowed and past the
    /// threshold. Says whether it swept.
    pub async fn round(&self) -> Option<Report> {
        let diagnosis = self.diagnose().await;
        let settings = self.settings();
        if settings.auto_sweep && diagnosis.over_threshold && !diagnosis.sweep.is_empty() {
            Some(self.sweep(true).await)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cleanup::fake::Lot;
    use crate::ports::{Heap, Survey};

    fn lot(total: u64) -> Lot {
        Lot {
            survey: Survey {
                taken_at: "2026-10-09T10:00:00Z".to_string(),
                total,
                folders: vec![
                    Heap {
                        rel: "init".to_string(),
                        bytes: 300,
                        idle_secs: 3 * 86_400,
                        dir: true,
                    },
                    Heap {
                        rel: "archive".to_string(),
                        bytes: 200,
                        idle_secs: 3 * 86_400,
                        dir: true,
                    },
                ],
                ..Survey::default()
            },
            ..Lot::default()
        }
    }

    #[tokio::test]
    async fn a_sweep_removes_what_the_weighing_says_and_reports_it() {
        let lot = Arc::new(lot(500));
        let janitor = Janitor::new(Arc::clone(&lot) as Arc<dyn Yard>);
        let report = janitor.sweep(false).await;
        assert_eq!(report.freed, 300);
        assert_eq!(report.removed.len(), 1);
        assert!(!report.automatic);
        assert_eq!(*lock(&lot.removed), ["init"], "the stray stays");
        let status = janitor.status();
        assert_eq!(status.doing, Doing::Idle);
        assert!(status.report.is_some() && status.diagnosis.is_some());
    }

    #[tokio::test]
    async fn a_refused_removal_is_reported_not_fatal() {
        let mut stuck = lot(500);
        stuck.stuck = Some("init".to_string());
        let janitor = Janitor::new(Arc::new(stuck));
        let report = janitor.sweep(false).await;
        assert_eq!(report.freed, 0);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].why.contains("denied"));
    }

    #[tokio::test]
    async fn the_chronic_round_sweeps_only_when_allowed_and_past_the_threshold() {
        let gib = 1024 * 1024 * 1024;
        let lot = Arc::new(lot(45 * gib));
        let janitor = Janitor::new(Arc::clone(&lot) as Arc<dyn Yard>);
        assert!(
            janitor.round().await.is_none(),
            "auto sweep is off by default"
        );
        janitor
            .set(Settings {
                auto_sweep: true,
                threshold_percent: 95,
                ..Settings::default()
            })
            .expect("set");
        assert!(janitor.round().await.is_none(), "45 of 50 GB is under 95 %");
        janitor
            .set(Settings {
                auto_sweep: true,
                threshold_percent: 80,
                ..Settings::default()
            })
            .expect("set");
        let report = janitor.round().await.expect("swept");
        assert!(report.automatic);
        assert_eq!(*lock(&lot.removed), ["init"]);
    }

    #[test]
    fn settings_are_kept_and_found_again_and_nonsense_is_refused() {
        let lot = Arc::new(Lot::default());
        let janitor = Janitor::new(Arc::clone(&lot) as Arc<dyn Yard>);
        assert!(
            janitor
                .set(Settings {
                    limit_bytes: 10,
                    ..Settings::default()
                })
                .is_err()
        );
        let wanted = Settings {
            limit_bytes: 20 * 1024 * 1024 * 1024,
            ..Settings::default()
        };
        janitor.set(wanted.clone()).expect("set");
        let again = Janitor::new(Arc::clone(&lot) as Arc<dyn Yard>);
        assert_eq!(again.settings(), wanted);
    }
}
