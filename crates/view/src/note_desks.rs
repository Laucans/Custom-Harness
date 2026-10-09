//! The pins' AI sessions at work: one Claude Code per open pin, in a
//! terminal the page attaches to; its conversation read back from Claude
//! Code's transcript when the human leaves it, kept with the pin, and handed
//! to the next session (`domain::note_session`).
//!
//! The rules are `domain::note_session`'s; the store, Claude and the
//! terminals are ports. This is the part with a clock, threads and the map
//! of sessions open right now.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::desk::Desk;
use crate::domain::annotations::{Kind, MAX_TEXT, Note};
use crate::domain::note_session::{self, About, Brief, Conversation, Handback};
use crate::ports::{NoteStore, Oneshot, TerminalMaker, Transcripts};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the sessions are made of.
pub struct Setup {
    /// Where the pins and their conversations are kept.
    pub store: Arc<dyn NoteStore>,
    /// Makes a session's terminal.
    pub maker: Arc<dyn TerminalMaker>,
    /// Asks Claude once: a summary, a conclusion, a request.
    pub oneshot: Arc<dyn Oneshot>,
    /// Reads what Claude Code kept of a session.
    pub transcripts: Arc<dyn Transcripts>,
    /// The model sessions and one-shot answers run on.
    pub model: String,
    /// Where a session runs: the product's clone when there is one.
    pub cwd: PathBuf,
}

struct Open {
    desk: Desk,
    session: String,
}

/// The pins' sessions; cheap to clone, shared by the handlers.
#[derive(Clone)]
pub struct NoteDesks {
    setup: Arc<Setup>,
    open: Arc<Mutex<HashMap<String, Open>>>,
}

fn key(board: &str, id: &str) -> String {
    format!("{board}/{id}")
}

/// The clock, as the traces write it.
fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// A fresh UUID (v4 shaped) from the process's random hasher keys.
fn uuid() -> String {
    let word = || {
        let mut h = std::hash::RandomState::new().build_hasher();
        h.write_u128(jiff::Timestamp::now().as_nanosecond().unsigned_abs());
        h.finish()
    };
    let (a, b) = (word(), word());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0x0fff,
        ((b >> 48) & 0x3fff) | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

/// What a pin sits on, in words: `field:character.name` → `field character.name`.
fn described(target: &str) -> String {
    target.split_once(':').map_or_else(
        || target.to_string(),
        |(kind, name)| format!("{kind} {name}"),
    )
}

impl NoteDesks {
    /// The sessions, none open yet.
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        Self {
            setup: Arc::new(setup),
            open: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The live pin `id` of `board`.
    fn note(&self, board: &str, id: &str) -> Result<Note, String> {
        self.setup
            .store
            .notes(board)?
            .into_iter()
            .find(|k| k.note.id == id && k.deleted_at.is_none())
            .map(|k| k.note)
            .ok_or_else(|| format!("no pin {id} on {board}"))
    }

    fn conversation(&self, board: &str, id: &str) -> Result<Conversation, String> {
        Ok(self
            .setup
            .store
            .conversation(board, id)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    fn save(&self, board: &str, id: &str, conversation: &Conversation) -> Result<(), String> {
        let json = serde_json::to_string(conversation).map_err(|e| e.to_string())?;
        self.setup
            .store
            .save_conversation(board, id, &json, conversation.turns.len())
    }

    async fn ask(&self, prompt: String) -> Result<String, String> {
        let oneshot = Arc::clone(&self.setup.oneshot);
        let model = self.setup.model.clone();
        tokio::task::spawn_blocking(move || oneshot.ask(&model, &prompt))
            .await
            .map_err(|e| format!("the question was lost: {e}"))?
    }

    /// The session of a pin open right now.
    #[must_use]
    pub fn desk(&self, board: &str, id: &str) -> Option<Desk> {
        lock(&self.open)
            .get(&key(board, id))
            .map(|o| o.desk.clone())
    }

    /// Opens a pin's session — or finds it open. The conversation so far is
    /// handed back in its system prompt; a heavy one is summarized first.
    /// `brief` is what the page shows of what the pin sits on, its branch
    /// and that branch's schema.
    ///
    /// # Errors
    ///
    /// No such pin, the store unreadable, or the summary failed.
    pub async fn open(&self, board: &str, id: &str, brief: Brief) -> Result<String, String> {
        if let Some(open) = lock(&self.open).get(&key(board, id)) {
            return Ok(open.desk.command());
        }
        let note = self.note(board, id)?;
        let about = About {
            board: board.to_string(),
            target: described(&note.target),
            context: brief.context,
            note: note.text.clone(),
            kind: if note.kind == Kind::Asking {
                "asking"
            } else {
                "brainstorm"
            }
            .to_string(),
            branch: brief.branch,
            schema: brief.schema,
        };
        let mut conversation = self.conversation(board, id)?;
        if let Handback::Summarize { older, covers } = conversation.handback() {
            let summary = self
                .ask(note_session::summary_prompt(&about, &older))
                .await?;
            conversation.summarized(summary, covers);
            self.save(board, id, &conversation)?;
        }
        let briefing = note_session::briefing(&about, &conversation.handback());
        let session = uuid();
        let mut args = vec![
            "--model".to_string(),
            self.setup.model.clone(),
            // A conversation, not a change: Claude reads, it does not edit.
            "--permission-mode".to_string(),
            "plan".to_string(),
            "--session-id".to_string(),
            session.clone(),
            "--append-system-prompt".to_string(),
            briefing,
        ];
        // A first session opens on what the note says: its first prompt.
        if conversation.turns.is_empty() && !note.text.trim().is_empty() {
            args.push(note.text.trim().to_string());
        }
        let desk = Desk::new(self.setup.maker.make(&self.setup.cwd, args));
        let command = desk.command();
        {
            let mut open = lock(&self.open);
            // Another click opened it meanwhile: that one stays.
            if let Some(existing) = open.get(&key(board, id)) {
                return Ok(existing.desk.command());
            }
            open.insert(key(board, id), Open { desk, session });
        }
        Ok(command)
    }

    /// Closes a pin's session, if one is open: its program stops and its
    /// turns join the pin's conversation. With `compact`, when the session
    /// said something, a compact of the discussion becomes the pin's text —
    /// what the human reads on the pin. Answers the conversation's length.
    ///
    /// # Errors
    ///
    /// The store could not be read or written, or the compact failed.
    pub async fn close(&self, board: &str, id: &str, compact: bool) -> Result<usize, String> {
        let Some(open) = lock(&self.open).remove(&key(board, id)) else {
            return Ok(self.conversation(board, id)?.turns.len());
        };
        open.desk.shutdown();
        let transcripts = Arc::clone(&self.setup.transcripts);
        let session = open.session.clone();
        let transcript = tokio::task::spawn_blocking(move || transcripts.read(&session))
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut conversation = self.conversation(board, id)?;
        let said = note_session::turns_of(&transcript);
        let grew = !said.is_empty();
        conversation.turns.extend(said);
        self.save(board, id, &conversation)?;
        if compact && grew {
            let mut note = self.note(board, id)?;
            let about = Self::about(board, &note);
            let text = self
                .ask(note_session::compact_prompt(&about, &conversation.digest()))
                .await?;
            note.text = text.chars().take(MAX_TEXT).collect();
            note.at = now();
            self.setup.store.put(board, &note)?;
        }
        Ok(conversation.turns.len())
    }

    /// What a one-shot question is told of a pin.
    fn about(board: &str, note: &Note) -> About {
        About {
            board: board.to_string(),
            target: described(&note.target),
            context: String::new(),
            branch: String::new(),
            schema: String::new(),
            note: note.text.clone(),
            kind: if note.kind == Kind::Asking {
                "asking"
            } else {
                "brainstorm"
            }
            .to_string(),
        }
    }

    /// Turns a brainstorm into a plain note — the AI draws the conclusion,
    /// written as the note — or into an asking — the AI formulates the
    /// request. The conversation is kept either way.
    ///
    /// # Errors
    ///
    /// No such pin, nothing said yet, or Claude failed.
    pub async fn turn_into(&self, board: &str, id: &str, kind: Kind) -> Result<Note, String> {
        self.close(board, id, false).await?;
        let mut note = self.note(board, id)?;
        let conversation = self.conversation(board, id)?;
        if conversation.turns.is_empty() {
            return Err("nothing was said yet: open the session first".to_string());
        }
        let about = Self::about(board, &note);
        let said = conversation.digest();
        let prompt = match kind {
            Kind::Asking => note_session::request_prompt(&about, &said),
            Kind::Note | Kind::Brainstorm => note_session::conclusion_prompt(&about, &said),
        };
        let answer = self.ask(prompt).await?;
        note.text = answer.chars().take(MAX_TEXT).collect();
        note.kind = kind;
        note.at = now();
        self.setup.store.put(board, &note)?;
        note.turns = conversation.turns.len();
        Ok(note)
    }

    /// Stops every session — the view is going away. Their turns are not
    /// read: the transcripts stay with Claude Code, and the next open of a
    /// pin starts from what was last kept.
    pub fn shutdown(&self) {
        for (_, open) in lock(&self.open).drain() {
            open.desk.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::fake::Echoing;
    use crate::domain::annotations::Kind;
    use crate::ports::{KeptNote, TerminalFactory};
    use std::path::Path;

    /// A store in memory.
    #[derive(Default)]
    struct Shelf {
        notes: Mutex<Vec<(String, KeptNote, Option<String>)>>,
    }

    // A test fake: each method holds its lock for the whole of its few lines.
    #[allow(clippy::significant_drop_tightening)]
    impl NoteStore for Shelf {
        fn notes(&self, board: &str) -> Result<Vec<KeptNote>, String> {
            Ok(lock(&self.notes)
                .iter()
                .filter(|(b, ..)| b == board)
                .map(|(_, k, _)| k.clone())
                .collect())
        }
        fn put(&self, board: &str, note: &Note) -> Result<(), String> {
            let mut notes = lock(&self.notes);
            match notes
                .iter_mut()
                .find(|(b, k, _)| b == board && k.note.id == note.id)
            {
                Some((_, k, _)) => k.note = note.clone(),
                None => notes.push((
                    board.to_string(),
                    KeptNote {
                        note: note.clone(),
                        deleted_at: None,
                    },
                    None,
                )),
            }
            Ok(())
        }
        fn mark_deleted(&self, _board: &str, _id: &str, _at: Option<&str>) -> Result<(), String> {
            Ok(())
        }
        fn destroy(&self, _board: &str, _id: &str) -> Result<(), String> {
            Ok(())
        }
        fn conversation(&self, board: &str, id: &str) -> Result<Option<String>, String> {
            Ok(lock(&self.notes)
                .iter()
                .find(|(b, k, _)| b == board && k.note.id == id)
                .and_then(|(.., c)| c.clone()))
        }
        fn save_conversation(
            &self,
            board: &str,
            id: &str,
            json: &str,
            turns: usize,
        ) -> Result<(), String> {
            if let Some((_, k, c)) = lock(&self.notes)
                .iter_mut()
                .find(|(b, k, _)| b == board && k.note.id == id)
            {
                *c = Some(json.to_string());
                k.note.turns = turns;
            }
            Ok(())
        }
    }

    /// Claude asked once: answers with the start of the prompt it got.
    #[derive(Default)]
    struct Echo {
        asked: Mutex<Vec<String>>,
    }

    impl Oneshot for Echo {
        fn ask(&self, _model: &str, prompt: &str) -> Result<String, String> {
            lock(&self.asked).push(prompt.to_string());
            Ok(format!(
                "answer to: {}",
                prompt.chars().take(30).collect::<String>()
            ))
        }
    }

    /// A transcript for whatever session is read.
    struct Kept;

    impl Transcripts for Kept {
        fn read(&self, _id: &str) -> Option<String> {
            Some(
                r#"{"type":"user","message":{"content":"Is name unique?"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Per campaign, yes."}]}}"#
                    .to_string(),
            )
        }
    }

    /// Records the arguments of every terminal it makes.
    #[derive(Default)]
    struct Maker {
        args: Mutex<Vec<Vec<String>>>,
    }

    impl TerminalMaker for Maker {
        fn make(&self, _cwd: &Path, args: Vec<String>) -> Arc<dyn TerminalFactory> {
            lock(&self.args).push(args);
            Arc::new(Echoing::default())
        }
    }

    fn pin(kind: Kind) -> Note {
        Note {
            id: "n1".into(),
            target: "field:character.name".into(),
            dx: 0.0,
            dy: 0.0,
            x: 0.0,
            y: 0.0,
            text: "unique?".into(),
            at: String::new(),
            kind,
            turns: 0,
        }
    }

    #[tokio::test]
    async fn a_session_opens_closes_and_hands_its_turns_back_the_next_time() {
        let store = Arc::new(Shelf::default());
        store
            .put("data-model", &pin(Kind::Brainstorm))
            .expect("kept");
        let maker = Arc::new(Maker::default());
        let oneshot = Arc::new(Echo::default());
        let desks = NoteDesks::new(Setup {
            store: Arc::clone(&store) as Arc<dyn NoteStore>,
            maker: Arc::clone(&maker) as Arc<dyn TerminalMaker>,
            oneshot: Arc::clone(&oneshot) as Arc<dyn Oneshot>,
            transcripts: Arc::new(Kept),
            model: "claude-sonnet-5-5".into(),
            cwd: PathBuf::from("/tmp"),
        });
        desks
            .open(
                "data-model",
                "n1",
                Brief {
                    context: "type: varchar(40)".into(),
                    ..Brief::default()
                },
            )
            .await
            .expect("opened");
        assert!(desks.desk("data-model", "n1").is_some());
        desks
            .open("data-model", "n1", Brief::default())
            .await
            .expect("found open");
        assert_eq!(lock(&maker.args).len(), 1, "opened once");
        let first = lock(&maker.args)[0].clone();
        assert!(
            first
                .windows(2)
                .any(|w| w[0] == "--permission-mode" && w[1] == "plan")
        );
        assert!(
            first
                .windows(2)
                .any(|w| w[0] == "--session-id" && w[1].len() == 36)
        );
        let briefing_of = |args: &[String]| {
            args.windows(2)
                .find(|w| w[0] == "--append-system-prompt")
                .map(|w| w[1].clone())
                .expect("a briefing")
        };
        let briefing = briefing_of(&first);
        assert!(
            briefing.contains("field character.name") && briefing.contains("type: varchar(40)")
        );
        assert!(!briefing.contains("<conversation>"));
        assert_eq!(
            first.last().map(String::as_str),
            Some("unique?"),
            "the note is the first prompt"
        );

        // Leaving the session: its turns are kept, and a compact becomes the note.
        assert_eq!(
            desks.close("data-model", "n1", true).await.expect("closed"),
            2
        );
        assert!(desks.desk("data-model", "n1").is_none());
        assert!(lock(&oneshot.asked)[0].contains("where the discussion stands"));
        let compacted = store.notes("data-model").expect("read")[0].note.clone();
        assert!(
            compacted
                .text
                .starts_with("answer to: This is a discussion")
        );
        assert_eq!(compacted.kind, Kind::Brainstorm, "a compact keeps the kind");

        desks
            .open("data-model", "n1", Brief::default())
            .await
            .expect("opened again");
        let again = lock(&maker.args)[1].clone();
        assert!(
            briefing_of(&again).contains("Human: Is name unique?\n\nClaude: Per campaign, yes.")
        );
        assert_ne!(
            again.last(),
            Some(&compacted.text),
            "a second session has no first prompt"
        );

        let asking = desks
            .turn_into("data-model", "n1", Kind::Asking)
            .await
            .expect("formulated");
        assert_eq!(asking.kind, Kind::Asking);
        assert!(asking.text.starts_with("answer to: This is a brainstorm"));
        assert_eq!(
            asking.turns, 4,
            "the second session's turns joined the first's"
        );
        assert_eq!(
            lock(&oneshot.asked).len(),
            2,
            "turning it compacts nothing more"
        );
    }

    #[test]
    fn a_session_id_is_shaped_as_a_uuid() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().nth(14), Some('4'));
        assert_ne!(uuid(), id);
        assert_eq!(described("field:character.name"), "field character.name");
    }
}
