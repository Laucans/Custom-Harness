//! The steward's desk: one terminal, kept alive between visits.
//!
//! The browser's pane comes and goes — the pane closes, the tab reloads — and
//! the program behind it must not: a `claude` mid-command would lose its work.
//! So the desk owns the terminal, pumps its output into a broadcast every
//! connection subscribes to, and keeps the last quarter-megabyte of it to
//! replay to whoever comes back. Like a `tmux` with one window.
//!
//! The program is started on the first visit, not at start-up, and started
//! again on the next keystroke after it exits. `shutdown` ends it with the
//! server.

use std::collections::VecDeque;
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use tokio::sync::broadcast;

use crate::ports::{TerminalFactory, TerminalIo};

/// How much output is kept for a visitor who comes back.
const SCROLLBACK: usize = 256 * 1024;

/// One read from the terminal, at most.
const CHUNK: usize = 8 * 1024;

/// How many chunks a slow connection may fall behind before it skips.
const LAG: usize = 512;

/// What the desk says when the program ends.
const GONE: &[u8] =
    b"\r\n\x1b[2m[the steward left the desk \xe2\x80\x94 press Enter to call them back]\x1b[0m\r\n";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The desk, shared by every connection to the page.
#[derive(Clone)]
pub struct Desk {
    factory: Arc<dyn TerminalFactory>,
    live: Arc<Mutex<Option<Box<dyn TerminalIo>>>>,
    pump: Arc<Mutex<Option<JoinHandle<()>>>>,
    out: broadcast::Sender<Vec<u8>>,
    scrollback: Arc<Mutex<VecDeque<u8>>>,
}

impl Desk {
    /// An empty desk: the program starts on the first visit.
    #[must_use]
    pub fn new(factory: Arc<dyn TerminalFactory>) -> Self {
        let (out, _) = broadcast::channel(LAG);
        Self {
            factory,
            live: Arc::new(Mutex::new(None)),
            pump: Arc::new(Mutex::new(None)),
            out,
            scrollback: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// A program is at the desk right now.
    #[must_use]
    pub fn is_live(&self) -> bool {
        lock(&self.live).is_some()
    }

    /// The command the desk runs, for the page to name.
    #[must_use]
    pub fn command(&self) -> String {
        self.factory.command()
    }

    /// A visitor sits down: the program is started if it is not, and the
    /// visitor gets the output stream plus everything said so far.
    ///
    /// # Errors
    ///
    /// Why the program could not start.
    pub fn attach(
        &self,
        cols: u16,
        rows: u16,
    ) -> Result<(broadcast::Receiver<Vec<u8>>, Vec<u8>), String> {
        // Subscribed before the program starts, so its first bytes are not
        // missed between the spawn and the subscription.
        let stream = self.out.subscribe();
        self.summon(cols, rows)?;
        let replay = lock(&self.scrollback).iter().copied().collect();
        Ok((stream, replay))
    }

    /// Starts the program if nobody is at the desk.
    ///
    /// # Errors
    ///
    /// Why the program could not start.
    pub fn summon(&self, cols: u16, rows: u16) -> Result<(), String> {
        let mut slot = lock(&self.live);
        if slot.is_some() {
            return Ok(());
        }
        let mut io = self.factory.spawn(cols, rows)?;
        let reader = io
            .output()
            .ok_or_else(|| "the terminal has no output".to_string())?;
        *slot = Some(io);
        drop(slot);
        self.pump(reader);
        Ok(())
    }

    /// Reads the program's output on a thread of its own, into the broadcast
    /// and the scrollback, until the program ends.
    fn pump(&self, mut reader: Box<dyn Read + Send>) {
        let out = self.out.clone();
        let scrollback = Arc::clone(&self.scrollback);
        let live = Arc::clone(&self.live);
        let handle = std::thread::spawn(move || {
            let mut buf = vec![0_u8; CHUNK];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let bytes = buf.get(..n).map(<[u8]>::to_vec).unwrap_or_default();
                let mut kept = lock(&scrollback);
                kept.extend(bytes.iter().copied());
                let overflow = kept.len().saturating_sub(SCROLLBACK);
                kept.drain(..overflow);
                drop(kept);
                let _ = out.send(bytes);
            }
            *lock(&live) = None;
            let _ = out.send(GONE.to_vec());
        });
        *lock(&self.pump) = Some(handle);
    }

    /// Keystrokes for the program.
    ///
    /// # Errors
    ///
    /// Nobody is at the desk, or the program refused the bytes.
    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let mut slot = lock(&self.live);
        slot.as_mut().map_or_else(
            || Err("nobody is at the desk".to_string()),
            |io| io.write(bytes).map_err(|e| e.to_string()),
        )
    }

    /// The pane changed size.
    ///
    /// # Errors
    ///
    /// Nobody is at the desk, or the terminal refused.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let mut slot = lock(&self.live);
        slot.as_mut().map_or_else(
            || Err("nobody is at the desk".to_string()),
            |io| io.resize(cols, rows).map_err(|e| e.to_string()),
        )
    }

    /// Ends the program, forgets what it said, starts a fresh one.
    ///
    /// # Errors
    ///
    /// Why the fresh program could not start.
    pub fn restart(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.shutdown();
        lock(&self.scrollback).clear();
        self.summon(cols, rows)
    }

    /// Ends the program and waits for its output to drain.
    pub fn shutdown(&self) {
        let io = lock(&self.live).take();
        if let Some(mut io) = io {
            io.kill();
        }
        let pump = lock(&self.pump).take();
        if let Some(handle) = pump {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
pub mod fake {
    //! A terminal that echoes what is written, over an in-memory pipe.

    use std::io::{self, Read};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Spawns echoing terminals, and counts them.
    #[derive(Default)]
    pub struct Echoing {
        pub spawned: AtomicU32,
        pub refuse: Mutex<Option<String>>,
        pub sizes: Arc<Mutex<Vec<(u16, u16)>>>,
    }

    struct Pipe {
        rx: Receiver<Vec<u8>>,
        pending: Vec<u8>,
    }

    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pending.is_empty() {
                match self.rx.recv() {
                    Ok(bytes) => self.pending = bytes,
                    Err(_) => return Ok(0),
                }
            }
            let n = buf.len().min(self.pending.len());
            buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            Ok(n)
        }
    }

    struct Echo {
        tx: Option<Sender<Vec<u8>>>,
        reader: Option<Box<dyn Read + Send>>,
        sizes: Arc<Mutex<Vec<(u16, u16)>>>,
    }

    impl TerminalIo for Echo {
        fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.tx.as_ref().map_or_else(
                || Err(io::Error::other("killed")),
                |tx| {
                    tx.send(bytes.to_vec())
                        .map_err(|_| io::Error::other("gone"))
                },
            )
        }

        fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
            lock(&self.sizes).push((cols, rows));
            Ok(())
        }

        fn output(&mut self) -> Option<Box<dyn Read + Send>> {
            self.reader.take()
        }

        fn kill(&mut self) {
            // Dropping the sender is what ends the reader.
            self.tx = None;
        }
    }

    impl TerminalFactory for Echoing {
        fn spawn(&self, cols: u16, rows: u16) -> Result<Box<dyn TerminalIo>, String> {
            let refusal = lock(&self.refuse).clone();
            if let Some(why) = refusal {
                return Err(why);
            }
            self.spawned.fetch_add(1, Ordering::SeqCst);
            lock(&self.sizes).push((cols, rows));
            let (tx, rx) = channel();
            Ok(Box::new(Echo {
                tx: Some(tx),
                reader: Some(Box::new(Pipe {
                    rx,
                    pending: Vec::new(),
                })),
                sizes: Arc::clone(&self.sizes),
            }))
        }

        fn command(&self) -> String {
            "echo".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Echoing;
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    async fn next(stream: &mut broadcast::Receiver<Vec<u8>>) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(2), stream.recv())
            .await
            .expect("output within two seconds")
            .expect("an open stream")
    }

    #[tokio::test]
    async fn what_is_typed_comes_back_and_a_late_visitor_gets_the_replay() {
        let factory = Arc::new(Echoing::default());
        let desk = Desk::new(factory.clone());
        assert!(!desk.is_live());
        let (mut first, replay) = desk.attach(80, 24).expect("attach");
        assert!(replay.is_empty());
        assert!(desk.is_live());
        desk.write(b"hello").expect("write");
        assert_eq!(next(&mut first).await, b"hello");
        let (_, replay) = desk.attach(80, 24).expect("attach again");
        assert_eq!(replay, b"hello", "the late visitor reads what was said");
        assert_eq!(
            factory.spawned.load(Ordering::SeqCst),
            1,
            "one program for both"
        );
        desk.shutdown();
        assert!(!desk.is_live());
    }

    #[tokio::test]
    async fn when_the_program_ends_the_desk_says_so_and_the_next_summon_restarts_it() {
        let factory = Arc::new(Echoing::default());
        let desk = Desk::new(factory.clone());
        let (mut stream, _) = desk.attach(80, 24).expect("attach");
        desk.shutdown();
        let said = next(&mut stream).await;
        assert!(String::from_utf8_lossy(&said).contains("left the desk"));
        assert!(desk.write(b"x").is_err(), "nobody to type to");
        desk.summon(100, 30).expect("summon");
        assert!(desk.is_live());
        assert_eq!(factory.spawned.load(Ordering::SeqCst), 2);
        assert_eq!(lock(&factory.sizes).last().copied(), Some((100, 30)));
        desk.shutdown();
    }

    #[tokio::test]
    async fn a_restart_forgets_the_scrollback() {
        let factory = Arc::new(Echoing::default());
        let desk = Desk::new(factory.clone());
        let (mut stream, _) = desk.attach(80, 24).expect("attach");
        desk.write(b"before").expect("write");
        next(&mut stream).await;
        desk.restart(80, 24).expect("restart");
        let (_, replay) = desk.attach(80, 24).expect("attach");
        assert!(replay.is_empty(), "{replay:?}");
        assert_eq!(factory.spawned.load(Ordering::SeqCst), 2);
        desk.shutdown();
    }

    #[tokio::test]
    async fn a_program_that_cannot_start_is_a_sentence_for_the_page() {
        let factory = Arc::new(Echoing::default());
        *lock(&factory.refuse) = Some("cannot start `claude`: not on PATH".to_string());
        let desk = Desk::new(factory);
        let why = desk.attach(80, 24).expect_err("refused");
        assert!(why.contains("not on PATH"));
        assert!(!desk.is_live());
    }

    #[test]
    fn the_scrollback_keeps_only_its_last_quarter_megabyte() {
        let factory = Arc::new(Echoing::default());
        let desk = Desk::new(factory);
        desk.summon(80, 24).expect("summon");
        let big = vec![b'a'; SCROLLBACK + 10];
        desk.write(&big).expect("write");
        // Wait for the pump to have kept it.
        for _ in 0..200 {
            if lock(&desk.scrollback).len() >= SCROLLBACK {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(lock(&desk.scrollback).len(), SCROLLBACK);
        desk.shutdown();
    }
}
