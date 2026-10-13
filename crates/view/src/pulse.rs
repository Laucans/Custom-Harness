//! The pulse: a counter bumped whenever what the creation page draws from
//! may have moved — the product's workspace written, a desk's job stepped.
//! The server streams it on `/api/events` (`pulse`); the page reads again
//! when it beats instead of asking on a timer, and redraws only what changed.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::watch;

use crate::ports::{MockState, Mockups};

/// A shared counter; cheap to clone, safe to beat from a blocking thread.
#[derive(Clone)]
pub struct Pulse(Arc<watch::Sender<u64>>);

impl Default for Pulse {
    fn default() -> Self {
        Self(Arc::new(watch::channel(0).0))
    }
}

impl Pulse {
    /// One more change: every listener hears it.
    pub fn beat(&self) {
        self.0.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// The counter as it moves, its current value first.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

/// The mock-up port, beating the pulse after every write: whoever writes the
/// product's workspace — the design system's easel, the mock-up's, a
/// creation job — is heard without beating it themselves. Reads never beat.
pub struct PulsingMockups {
    inner: Arc<dyn Mockups>,
    pulse: Pulse,
}

impl PulsingMockups {
    /// `inner`, heard on `pulse`.
    #[must_use]
    pub fn new(inner: Arc<dyn Mockups>, pulse: Pulse) -> Self {
        Self { inner, pulse }
    }

    fn beating<T>(&self, outcome: T) -> T {
        self.pulse.beat();
        outcome
    }
}

impl Mockups for PulsingMockups {
    fn state(&self, branch: &str) -> Result<MockState, String> {
        self.inner.state(branch)
    }

    fn file(&self, branch: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        self.inner.file(branch, path)
    }

    fn workspace(&self, branch: &str) -> Result<PathBuf, String> {
        self.inner.workspace(branch)
    }

    fn save(&self, branch: &str, name: &str) -> Result<String, String> {
        self.beating(self.inner.save(branch, name))
    }

    fn discard(&self, branch: &str) -> Result<(), String> {
        self.beating(self.inner.discard(branch))
    }

    fn write(&self, branch: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.beating(self.inner.write(branch, path, bytes))
    }

    fn remove(&self, branch: &str, path: &str) -> Result<(), String> {
        self.beating(self.inner.remove(branch, path))
    }

    fn branches(&self) -> Result<Vec<String>, String> {
        self.inner.branches()
    }

    // Read on every look at the easel (`KitDesk::reply`): beating here would
    // make each read wake the page that read.
    fn ensure_branch(&self, branch: &str, from: &str) -> Result<(), String> {
        self.inner.ensure_branch(branch, from)
    }

    fn publish(&self, branch: &str, message: &str) -> Result<String, String> {
        self.beating(self.inner.publish(branch, message))
    }

    fn carry(
        &self,
        from: &str,
        to: &str,
        folders: &[&str],
        message: &str,
    ) -> Result<Option<String>, String> {
        self.beating(self.inner.carry(from, to, folders, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace that keeps nothing and answers everything.
    struct Blank;

    impl Mockups for Blank {
        fn state(&self, branch: &str) -> Result<MockState, String> {
            Ok(MockState {
                branch: branch.to_string(),
                ..MockState::default()
            })
        }
        fn file(&self, _branch: &str, _path: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(None)
        }
        fn workspace(&self, _branch: &str) -> Result<PathBuf, String> {
            Ok(PathBuf::from("/nowhere"))
        }
        fn save(&self, _branch: &str, name: &str) -> Result<String, String> {
            Ok(format!("mockup/{name}"))
        }
        fn discard(&self, _branch: &str) -> Result<(), String> {
            Ok(())
        }
        fn write(&self, _branch: &str, _path: &str, _bytes: &[u8]) -> Result<(), String> {
            Err("refused".to_string())
        }
        fn remove(&self, _branch: &str, _path: &str) -> Result<(), String> {
            Ok(())
        }
        fn branches(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
        fn ensure_branch(&self, _branch: &str, _from: &str) -> Result<(), String> {
            Ok(())
        }
        fn publish(&self, _branch: &str, _message: &str) -> Result<String, String> {
            Ok("abc".to_string())
        }
        fn carry(
            &self,
            _from: &str,
            _to: &str,
            _folders: &[&str],
            _message: &str,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    #[test]
    fn a_beat_is_heard_by_every_listener_even_one_that_came_late() {
        let pulse = Pulse::default();
        let mut early = pulse.subscribe();
        pulse.beat();
        let late = pulse.subscribe();
        assert!(early.has_changed().expect("the pulse lives"));
        assert_eq!(*early.borrow_and_update(), 1);
        assert_eq!(
            *late.borrow(),
            1,
            "a late listener reads the count as it is"
        );
        let twin = pulse.clone();
        twin.beat();
        pulse.beat();
        assert_eq!(*early.borrow(), 3, "a clone beats the same pulse");
    }

    #[test]
    fn every_write_beats_and_no_read_does() {
        let pulse = Pulse::default();
        let mut ears = pulse.subscribe();
        let mockups = PulsingMockups::new(Arc::new(Blank), pulse);
        let heard = |ears: &mut watch::Receiver<u64>| {
            let moved = ears.has_changed().expect("the pulse lives");
            ears.mark_unchanged();
            moved
        };

        let _ = mockups.state("b");
        let _ = mockups.file("b", "pages/x.html");
        let _ = mockups.workspace("b");
        let _ = mockups.branches();
        let _ = mockups.ensure_branch("b", "main");
        assert!(!heard(&mut ears), "a read wakes nobody");

        let _ = mockups.save("b", "n");
        assert!(heard(&mut ears), "save");
        let _ = mockups.discard("b");
        assert!(heard(&mut ears), "discard");
        let _ = mockups.remove("b", "x");
        assert!(heard(&mut ears), "remove");
        let _ = mockups.publish("b", "m");
        assert!(heard(&mut ears), "publish");
        let _ = mockups.carry("a", "b", &["kit"], "m");
        assert!(heard(&mut ears), "carry");
        // A write refused may still have touched the workspace: it beats too.
        assert!(mockups.write("b", "x", b"").is_err());
        assert!(heard(&mut ears), "write, refused");
    }

    #[test]
    fn the_wrapped_answers_go_through_untouched() {
        let mockups = PulsingMockups::new(Arc::new(Blank), Pulse::default());
        assert_eq!(mockups.save("b", "n").as_deref(), Ok("mockup/n"));
        assert_eq!(mockups.publish("b", "m").as_deref(), Ok("abc"));
        assert_eq!(mockups.write("b", "x", b""), Err("refused".to_string()));
        assert_eq!(mockups.state("b").map(|s| s.branch).as_deref(), Ok("b"));
    }
}
