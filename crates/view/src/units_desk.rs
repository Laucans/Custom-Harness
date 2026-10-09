//! The capabilities list's reading, kept: the `Units` port is slow — a
//! fetch, a few `git` reads — so one reading serves every request for a
//! while, and two requests at once share one read.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::ports::{Oneshot, Units, UnitsReading};

/// A reading and when it was made.
type Kept = Option<(Instant, Arc<UnitsReading>)>;

/// The product's units, read through the port and kept.
#[derive(Clone)]
pub struct UnitsDesk {
    units: Arc<dyn Units>,
    kept: Arc<Mutex<Kept>>,
    every: Duration,
    /// The branch the agents integrate into.
    pub integration: String,
    /// What words a fusion test — `None` when no Claude Code can be found.
    pub oneshot: Option<Arc<dyn Oneshot>>,
    /// The model a fusion test asks.
    pub model: String,
}

impl UnitsDesk {
    /// A desk over `units`, a reading kept `every`.
    #[must_use]
    pub fn new(
        units: Arc<dyn Units>,
        every: Duration,
        integration: &str,
        oneshot: Option<Arc<dyn Oneshot>>,
        model: &str,
    ) -> Self {
        Self {
            units,
            kept: Arc::default(),
            every,
            integration: integration.to_string(),
            oneshot,
            model: model.to_string(),
        }
    }

    /// The kept reading, or a fresh one when it is older than `every` or
    /// `fresh` is asked. A failed read falls back on the kept one.
    ///
    /// # Errors
    ///
    /// Nothing was ever read, and the read failed.
    pub async fn reading(&self, fresh: bool) -> Result<Arc<UnitsReading>, String> {
        let mut kept = self.kept.lock().await;
        if let Some((at, reading)) = kept.as_ref()
            && !fresh
            && at.elapsed() < self.every
        {
            return Ok(Arc::clone(reading));
        }
        let units = Arc::clone(&self.units);
        let read = tokio::task::spawn_blocking(move || units.read())
            .await
            .map_err(|e| format!("the read did not finish: {e}"))?;
        match read {
            Ok(reading) => {
                let reading = Arc::new(reading);
                *kept = Some((Instant::now(), Arc::clone(&reading)));
                Ok(reading)
            }
            Err(why) => {
                tracing::warn!("capabilities unreadable: {why}");
                kept.as_ref()
                    .map(|(_, reading)| Arc::clone(reading))
                    .ok_or(why)
            }
        }
    }

    /// Asks the model `prompt`, on a blocking thread.
    ///
    /// # Errors
    ///
    /// No Claude Code here, or it failed.
    pub async fn ask(&self, prompt: String) -> Result<String, String> {
        let oneshot = self.oneshot.clone().ok_or("no Claude Code found to ask")?;
        let model = self.model.clone();
        tokio::task::spawn_blocking(move || oneshot.ask(&model, &prompt))
            .await
            .map_err(|e| format!("the question did not finish: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Counted(AtomicUsize);

    impl Units for Counted {
        fn read(&self) -> Result<UnitsReading, String> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(UnitsReading::default())
            } else {
                Err("offline".to_string())
            }
        }
    }

    #[tokio::test]
    async fn a_reading_is_kept_and_a_failed_one_falls_back_on_it() {
        let units = Arc::new(Counted(AtomicUsize::new(0)));
        let desk = UnitsDesk::new(
            Arc::clone(&units) as Arc<dyn Units>,
            Duration::from_secs(60),
            "main_agent",
            None,
            "haiku",
        );
        assert!(desk.reading(false).await.is_ok());
        assert!(desk.reading(false).await.is_ok());
        assert_eq!(units.0.load(Ordering::SeqCst), 1, "kept, not read again");
        assert!(
            desk.reading(true).await.is_ok(),
            "the kept reading stands in"
        );
        assert_eq!(units.0.load(Ordering::SeqCst), 2);
        assert!(desk.ask("q".to_string()).await.is_err());
    }
}
