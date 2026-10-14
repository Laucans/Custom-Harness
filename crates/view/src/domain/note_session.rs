//! The AI session behind a pin: what it is told when it opens, what is kept
//! of it when it closes, and what is put back the next time.
//!
//! A brainstorm (or an asking) is a Claude Code in a terminal, opened from
//! its pin. When the human leaves it, its turns are read from Claude Code's
//! own transcript and appended to the pin's **conversation**, kept with the
//! pin and destroyed with it. The next session starts fresh and is handed
//! the conversation back, in its system prompt:
//!
//! - whole, while it weighs at most [`FULL_LIMIT`] tokens;
//! - beyond, as a summary of the older turns plus the last [`RECENT`] tokens
//!   word for word. The summary is kept and reused until the conversation
//!   has grown by [`REFRESH`] more — a context handed back identical is one
//!   the prompt cache can still serve.
//!
//! The whole conversation is always kept: only what is handed back is
//! compacted.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Up to this many tokens, the conversation is handed back whole.
pub const FULL_LIMIT: usize = 40_000;

/// Beyond, the last turns handed back word for word weigh about this much.
pub const RECENT: usize = 10_000;

/// A summary is made again once the conversation grew this much since.
pub const REFRESH: usize = 20_000;

/// The longest tool call kept in a turn, in characters.
const TOOL_TRACE: usize = 160;

/// One turn of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// `human` or `claude`.
    pub role: String,
    /// What was said; for Claude, the tools it used in brackets.
    pub text: String,
}

/// A summary of the older turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// The summary.
    pub text: String,
    /// It covers `turns[..covers]`.
    pub covers: usize,
    /// The conversation's weight, in tokens, when it was made.
    pub at_tokens: usize,
}

/// Everything a pin's sessions said, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conversation {
    /// Every turn, never compacted.
    pub turns: Vec<Turn>,
    /// The summary handed back instead of the older turns, once there is one.
    pub summary: Option<Summary>,
}

/// What the next session is handed back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handback {
    /// Nothing yet: a first session.
    Nothing,
    /// The whole conversation.
    Whole(String),
    /// The kept summary, then the recent turns word for word.
    Compacted {
        /// The summary of the older turns.
        summary: String,
        /// The recent turns.
        recent: String,
    },
    /// A summary of `turns[..covers]` must be made first.
    Summarize {
        /// The older turns, to summarize.
        older: String,
        /// How many turns the summary will cover.
        covers: usize,
    },
}

/// About how many tokens a text weighs: a character in four.
#[must_use]
pub fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// The turns as text: `Human: …` and `Claude: …`, a blank line between.
#[must_use]
pub fn render(turns: &[Turn]) -> String {
    turns
        .iter()
        .map(|t| {
            let who = if t.role == "human" { "Human" } else { "Claude" };
            format!("{who}: {}", t.text.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Conversation {
    /// Its weight, in tokens.
    #[must_use]
    pub fn weight(&self) -> usize {
        tokens(&render(&self.turns))
    }

    /// What the next session is handed back.
    #[must_use]
    pub fn handback(&self) -> Handback {
        if self.turns.is_empty() {
            return Handback::Nothing;
        }
        let weight = self.weight();
        if weight <= FULL_LIMIT {
            return Handback::Whole(render(&self.turns));
        }
        let start = recent_start(&self.turns);
        match &self.summary {
            Some(s) if weight < s.at_tokens + REFRESH && s.covers <= self.turns.len() => {
                Handback::Compacted {
                    summary: s.text.clone(),
                    recent: render(self.turns.get(s.covers..).unwrap_or_default()),
                }
            }
            _ => Handback::Summarize {
                older: render(self.turns.get(..start).unwrap_or_default()),
                covers: start,
            },
        }
    }

    /// The conversation as a one-shot question reads it — for a compact, a
    /// conclusion, a request: whole while it is light, else the kept summary
    /// and the turns after it, else its last [`FULL_LIMIT`] tokens.
    #[must_use]
    pub fn digest(&self) -> String {
        match self.handback() {
            Handback::Nothing => String::new(),
            Handback::Whole(whole) => whole,
            Handback::Compacted { summary, recent } => {
                format!("(a summary of the start)\n{summary}\n\n{recent}")
            }
            Handback::Summarize { .. } => {
                let start = start_within(&self.turns, FULL_LIMIT);
                render(self.turns.get(start..).unwrap_or_default())
            }
        }
    }

    /// Keeps a summary just made of `turns[..covers]`.
    pub fn summarized(&mut self, text: String, covers: usize) {
        let at_tokens = self.weight();
        self.summary = Some(Summary {
            text,
            covers,
            at_tokens,
        });
    }
}

/// Where the recent turns begin: the last ones that weigh at most
/// [`RECENT`] together — always at least the last one.
fn recent_start(turns: &[Turn]) -> usize {
    start_within(turns, RECENT)
}

/// Where the last turns that weigh at most `budget` together begin — always
/// at least the last one.
fn start_within(turns: &[Turn], budget: usize) -> usize {
    let mut weight = 0;
    let mut start = turns.len();
    for (k, turn) in turns.iter().enumerate().rev() {
        weight += tokens(&turn.text) + 3;
        if weight > budget && start < turns.len() {
            break;
        }
        start = k;
    }
    start
}

/// The turns of a Claude Code transcript (`~/.claude/projects/…/<id>.jsonl`):
/// what the human typed, what Claude answered, and a short trace of each
/// tool it used. Tool results, thinking, and what the harness itself
/// injected (`<system-reminder>`, `<command-…>`) are left out.
#[must_use]
pub fn turns_of(jsonl: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for line in jsonl.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let role = match entry.get("type").and_then(Value::as_str) {
            Some("user") => "human",
            Some("assistant") => "claude",
            _ => continue,
        };
        if entry.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(content) = entry.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        let mut parts: Vec<String> = Vec::new();
        match content {
            Value::String(text) => parts.push(text.clone()),
            Value::Array(blocks) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                parts.push(text.to_string());
                            }
                        }
                        Some("tool_use") => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let input =
                                block.get("input").map(Value::to_string).unwrap_or_default();
                            let short: String = input.chars().take(TOOL_TRACE).collect();
                            parts.push(format!("[{name} {short}]"));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        let text = parts
            .into_iter()
            .filter(|p| {
                let t = p.trim_start();
                !t.is_empty()
                    && !t.starts_with("<system-reminder>")
                    && !t.starts_with("<command-")
                    && !t.starts_with("<local-command")
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        // One turn per speaker: Claude's text and its tool calls arrive as
        // several entries in a row.
        match turns.last_mut() {
            Some(last) if last.role == role => {
                last.text.push('\n');
                last.text.push_str(&text);
            }
            _ => turns.push(Turn {
                role: role.to_string(),
                text,
            }),
        }
    }
    turns
}

/// What a session is about: the pin and what it sits on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct About {
    /// The board: `data-model`.
    pub board: String,
    /// What the pin sits on, in words: `field character.name`.
    pub target: String,
    /// What the page knows of it — the field's type, purpose, rules, links,
    /// and the version shown — as the page sent it.
    pub context: String,
    /// The pin's own text.
    pub note: String,
    /// `brainstorm`, `request`, `link` or `design`.
    pub kind: String,
    /// On the mock-up: the page the pin sits on (`pages/home.html`), and
    /// for a link, the page it leads to or creates.
    #[serde(default)]
    pub page: String,
    /// For a design: the micro-frontend's name.
    #[serde(default)]
    pub component: String,
    /// A reference the session draws from — a Figma frame's link — or empty.
    #[serde(default)]
    pub source: String,
    /// The branch the pin belongs to: `main_agent`, `milestone/3-…`; on
    /// the mock-up, the branch shown.
    #[serde(default)]
    pub branch: String,
    /// That branch's `data/schema.sql`, as the version shown has it.
    #[serde(default)]
    pub schema: String,
    /// The milestone a feature branch belongs to, in words
    /// ([`milestone_words`]); empty for another branch.
    #[serde(default)]
    pub milestone: String,
    /// The session reworks the request in the files, with the human,
    /// rather than talks about it.
    #[serde(default)]
    pub rework: bool,
}

/// What the page tells a session it opens: what it shows of the pin's
/// target, the branch the pin belongs to, that branch's schema.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Brief {
    /// What the page shows of what the pin sits on.
    pub context: String,
    /// The pin's branch.
    pub branch: String,
    /// The branch's `data/schema.sql`; empty when it was not read.
    pub schema: String,
    /// The branch's milestone, in words; empty when it is not a feature
    /// branch or the board does not know it.
    pub milestone: String,
    /// Where the session runs instead of the product's clone: the branch's
    /// mock-up workspace, for a pin on the mock-up.
    pub cwd: Option<std::path::PathBuf>,
    /// On the mock-up: the page the pin sits on.
    pub page: String,
    /// A request reworked in the files with the human, not discussed.
    pub rework: bool,
}

/// The most of a milestone's description a session is handed.
const MILESTONE_BODY_CHARS: usize = 6_000;

/// A milestone as a session reads it: its title, where it stands, what it
/// says, and its tasks.
#[must_use]
pub fn milestone_words(milestone: &crate::ports::Milestone) -> String {
    let issue = &milestone.issue;
    let mut text = format!("#{} {} ({})\n", issue.number, issue.title, issue.state);
    let body: String = issue
        .body
        .trim()
        .chars()
        .take(MILESTONE_BODY_CHARS)
        .collect();
    if !body.is_empty() {
        let _ = writeln!(text, "{body}");
    }
    if !milestone.tasks.is_empty() {
        let _ = writeln!(text, "Its tasks:");
        for task in &milestone.tasks {
            let _ = writeln!(text, "  - #{} {} ({})", task.number, task.title, task.state);
        }
    }
    text
}

/// The system prompt a pin's session opens with.
#[must_use]
pub fn briefing(about: &About, handback: &Handback) -> String {
    let mut text = match about.kind.as_str() {
        _ if about.rework => rework_briefing(about),
        "link" => page_briefing(about),
        "design" => design_briefing(about),
        _ => talk_briefing(about),
    };
    if !about.context.trim().is_empty() {
        let _ = writeln!(
            text,
            "\nWhat the view shows of it:\n{}",
            about.context.trim()
        );
    }
    if !about.schema.trim().is_empty() {
        let _ = writeln!(
            text,
            "\nThe branch's database schema, `data/schema.sql` (pg_dump --schema-only):\n<schema.sql>\n{}\n</schema.sql>",
            about.schema.trim()
        );
    }
    match handback {
        Handback::Nothing | Handback::Summarize { .. } => {}
        Handback::Whole(whole) => {
            let _ = write!(
                text,
                "\nYou already talked about it with the human. The conversation so far, to continue from \
             — do not repeat it back:\n<conversation>\n{whole}\n</conversation>\n"
            );
        }
        Handback::Compacted { summary, recent } => {
            let _ = write!(
                text,
                "\nYou already talked about it with the human. A summary of the earlier conversation:\n\
             <summary>\n{summary}\n</summary>\nThen the most recent turns, word for word — continue \
             from there, do not repeat it back:\n<conversation>\n{recent}\n</conversation>\n"
            );
        }
    }
    text
}

/// What the mock-up's sessions share: where the pages are and what must
/// stay true of them.
#[must_use]
pub fn mockup_rules(about: &About) -> String {
    format!(
        "You are working on the product's mock-up: a static frontend with no backend under \
         `mock/` in this folder — one HTML page per screen under `mock/pages/`, plain HTML and \
         JavaScript, data read from JSON fixtures under `mock/fixtures/`, every page listed in \
         `mock/pages.json` (contract N: path, title, objective, optional screen and components). \
         A page is static HTML plus **micro-frontends**: a micro-frontend is a Micro-UI as the \
         mock shows it — `mock/components/<Name>.html`, its markup and the script that fills it, \
         on fixtures under `mock/fixtures/` shaped like what its Capabilities (`needs`) would \
         answer; its user actions (`actions`) show a command's lifecycle (queued, confirmed, \
         applied) on the fixture; its `props` come from `data-props`. **When the Micro-UI exists \
         under `apps/`, the micro-frontend is that real code**: an entry `mock/src/<Name>.js` \
         mounts it on the product's real shell client over a mock server `mock/src/server.js` \
         (the shell's wire protocol answered in memory from the fixtures, commands applied and \
         `dataVersion` bumped), bundled by `mock/build.sh` (esbuild through `npx`) into \
         `mock/components/<Name>.js`, which the fragment loads; the bundles are committed. Read \
         `mock/README.md` and the existing `mock/src/` first and follow them. A hand-written \
         fragment is only for a Micro-UI not built yet. Each is listed under \
         `components` of `mock/pages.json` (name, system, description, fragment, fixture, needs, \
         actions, props), and a page injects one with `<div data-component=\"<Name>\" \
         data-props='{{…}}'></div>` plus `<script src=\"../mock.js\"></script>` — `mock/mock.js` \
         (create it if absent) fetches each `components/<Name>.html` into its element and runs its \
         scripts; a fragment's script finds its root as `document.currentScript.closest('[data-component]')`. \
         The factory view previews a micro-frontend alone at `__preview/<Name>`: a page that loads \
         `../style.css`, `../mock.js` and one `data-component` element, so a fragment must stand \
         on its own. \
         This folder is a checkout of the branch `{branch}` made for you; the factory view serves \
         `mock/` from it and reloads the page as you save files. Rules: edit only under `mock/`; \
         keep `mock/pages.json` valid JSON and listing every page; keep each Micro-UI's root \
         element marked `data-component=\"<MicroUi>\"` and give the parts a human may point at a \
         stable `id`; relative links only, so a page works from any file server; no build step, no \
         framework that must compile, no network call. Do not commit, do not push, do not branch: \
         the human saves the mock-up from the view when they are satisfied. Say in one line what \
         you changed when you are done.\n\n\
         `mock/pages.json` has exactly this shape — a field is a string unless it is a list or an \
         object here, except a micro-frontend's `fixture`: one file, or a list of files when it \
         reads several; no other field is read:\n\n\
         ```json\n{shape}\n```\n",
        shape = crate::domain::mockup::manifest_shape(),
        branch = if about.branch.is_empty() {
            "shown"
        } else {
            &about.branch
        },
    )
}

/// A link that creates a page: the pin carries the page's objective.
fn page_briefing(about: &About) -> String {
    let mut text = mockup_rules(about);
    let _ = write!(
        text,
        "\nThe human asks for a new page of the mock-up: `{page}`. Create it — if `mock/` has no \
         page yet, start the mock-up: `mock/pages.json`, a `mock/index.html` that lists the pages \
         and a small shared `mock/style.css` — register it in `mock/pages.json` with its title and \
         its objective, and draw it from the objective and from what the repository already says \
         (the data model in `data/`, the Micro-UI manifests under `apps/`, the other pages): real \
         sections and components with example data in fixtures, not a lorem ipsum. Link it from \
         `mock/index.html`. When the page exists, ask the human what to adjust.\n",
        page = about.page,
    );
    if !about.note.trim().is_empty() {
        let _ = writeln!(text, "The page's objective:\n{}", about.note.trim());
    }
    if !about.source.trim().is_empty() {
        let _ = writeln!(
            text,
            "The human gave a reference to draw the page from: {}. When it is a Figma frame and a Figma \
             tool is available to you (an MCP server such as `get_code`, `get_image` or `get_design_context`), \
             fetch that frame and reproduce its layout, texts and components in the page; without such a \
             tool, say so in one line and ask the human for an export (an image or the frame's code), then \
             draw from the objective meanwhile.",
            about.source.trim()
        );
    }
    text
}

/// A design: a micro-frontend to make from its spec, and to inject in the
/// page the pin sits on when it sits on one.
/// A request made in the files with the human: the mock-up's rules, what
/// the request asks, how what is saved shows, and what stays the view's.
fn rework_briefing(about: &About) -> String {
    let mut text = mockup_rules(about);
    let _ = write!(
        text,
        "\nThe human pinned a request on {target} of the page `{page}` and asks you to make it in \
         the files, here, with them — the request is below, and what the view already tried for it \
         is in what the view shows. Edit the page under `mock/pages/`, its micro-frontends under \
         `mock/components/` and `mock/style.css` as the request needs, a layout included (columns, \
         a sticky part, a drawer, a breakpoint), keeping the micro-frontends' contract and the \
         design system (the kit's components, the `var(--…)` tokens). A micro-frontend that has a \
         draft (`mock/components/<Name>.draft.html`) shows its draft on the human's canvas: edit \
         the draft then, not its fragment — the human settles drafts in the bar above the page. \
         What you save shows on the canvas at once. Do what the request asks and keep the \
         product's rules the view lists; when a point is unclear, ask before doing it. After each \
         change, say in two lines what changed and where. When the human leaves you, they keep \
         all of it or drop all of it.\n",
        target = if about.target.is_empty() {
            "a spot"
        } else {
            &about.target
        },
        page = about.page,
    );
    text
}

fn design_briefing(about: &About) -> String {
    let mut text = mockup_rules(about);
    if about.page.is_empty() {
        let _ = write!(
            text,
            "\nThe human designs the micro-frontend `{name}` from the micro-frontends view. Make it: \
             when the Micro-UI exists under `apps/`, an entry `mock/src/{name}.js` that mounts the \
             real code on the shell over the mock server, the mock server taught its Capabilities and \
             DataCapabilities if it does not know them yet, and the bundle built with `mock/build.sh`; \
             otherwise `mock/components/{name}.html` with its script, drawn from the spec. Either way \
             a fixture shaped like what the Capabilities it needs would answer (read their manifests \
             and persisted queries under `crates/` when they exist, the data model under `data/` \
             otherwise), and its entry \
             under `components` of `mock/pages.json` with the spec below. Make `mock/mock.js` and \
             `mock/style.css` if the mock-up has none yet. Do not inject it in a page: the human \
             drops it where they want it. When it previews well, ask what to adjust.\n",
            name = about.component,
        );
    } else {
        let _ = write!(
            text,
            "\nThe human dropped the micro-frontend `{name}` on {target} of the page `{page}`. \
             Inject it there: a `<div data-component=\"{name}\" data-props='{{…}}'></div>` at that \
             spot with the props the page can give it, the page loading `../mock.js`, and `{name}` \
             added to the page's `components` in `mock/pages.json`. If `mock/components/{name}.html` \
             does not exist yet, make it first from the spec below, with its fixture and its entry \
             under `components`. Keep the rest of the page as it is.\n",
            name = about.component,
            target = if about.target.is_empty() {
                "a spot"
            } else {
                &about.target
            },
            page = about.page,
        );
    }
    if !about.note.trim().is_empty() {
        let _ = writeln!(text, "The micro-frontend's spec:\n{}", about.note.trim());
    }
    text
}

/// A brainstorm or an asking: a conversation, nothing edited.
fn talk_briefing(about: &About) -> String {
    let mut text = format!(
        "You are brainstorming with the human about one part of their product, from a pin they \
         dropped on the factory view's {board} board. Read freely — the product's repository is \
         in this folder (when it is the data snapshot clone, the archive branch `harness/data-model` \
         is checked out: read code with `git show <branch>:<path>`). Do not edit any file, do not \
         commit, do not post anything: this is a conversation, and what comes out of it is decided \
         by the human.\n\n\
         The pin sits on: {target}\n\
         It is {kind_words}.\n",
        board = about.board,
        target = if about.target.is_empty() {
            "a spot of the canvas"
        } else {
            &about.target
        },
        kind_words = if about.kind == "request" {
            "a *request*: an intention for the next roadmap — a change, a behaviour the human wants — help them make it precise; on the mock-up its visible part is applied to the draft by another, one-shot call"
        } else {
            "a *brainstorm*: explore options, trade-offs and consequences with the human"
        },
    );
    if !about.branch.is_empty() {
        let _ = writeln!(
            text,
            "The pin belongs to the branch `{}`: talk about the product as that branch has it.",
            about.branch
        );
    }
    if !about.milestone.trim().is_empty() {
        let _ = writeln!(
            text,
            "That branch is a milestone's: its tasks merge there. The milestone:\n<milestone>\n{}\n</milestone>",
            about.milestone.trim()
        );
    }
    if !about.note.trim().is_empty() {
        let _ = writeln!(text, "The pin's note says:\n{}", about.note.trim());
    }
    text
}

/// The first prompt a session opens on, when there is one: the note for a
/// brainstorm, an asking or a simulate; the page to make for a link.
#[must_use]
pub fn first_prompt(about: &About) -> Option<String> {
    let note = about.note.trim();
    match about.kind.as_str() {
        _ if about.rework => Some(format!(
            "Make this request in the files of the page `{}` — read the page and the \
             micro-frontends it concerns first, then say how you will do it before you start: {note}",
            about.page
        )),
        "link" => Some(if note.is_empty() {
            format!("Create the page `{}` of the mock-up.", about.page)
        } else {
            format!(
                "Create the page `{}` of the mock-up. Its objective: {note}",
                about.page
            )
        }),
        "design" => Some(if about.page.is_empty() {
            format!(
                "Make the micro-frontend `{}` of the mock-up from its spec.",
                about.component
            )
        } else {
            format!(
                "Inject the micro-frontend `{}` in the page `{}` where the pin sits; make it first if it does not exist.",
                about.component, about.page
            )
        }),
        _ if note.is_empty() => None,
        _ => Some(note.to_string()),
    }
}

/// What the summary is asked of.
#[must_use]
pub fn summary_prompt(about: &About, older: &str) -> String {
    format!(
        "Summarize this conversation between a human and an AI about {target} so that the AI can \
         continue it later without the original. Keep every decision, every open question, every \
         option still on the table and why the others were dropped, names, numbers and file paths. \
         Drop greetings and repetition. At most 600 words, plain prose and short lists. Output the \
         summary only.\n\n<conversation>\n{older}\n</conversation>",
        target = if about.target.is_empty() {
            "a part of their product"
        } else {
            &about.target
        },
    )
}

/// What the compact is asked of — written into the pin's note when its
/// session closes, so the note always says where the discussion stands.
#[must_use]
pub fn compact_prompt(about: &About, conversation: &str) -> String {
    format!(
        "This is a discussion between a human and an AI about {target}, from a pin of kind {kind}. \
         Compact it into the text the pin will show: where the discussion stands — what was decided, \
         the options still open and why, the questions left — so that the human, reading it in a \
         month, knows exactly where they are. At most 150 words, plain sentences and short lists, \
         no preamble. Output the text only.\n\n<conversation>\n{conversation}\n</conversation>",
        target = if about.target.is_empty() {
            "a part of their product"
        } else {
            &about.target
        },
        kind = about.kind,
    )
}

/// What the conclusion is asked of — the brainstorm becomes a plain note.
#[must_use]
pub fn conclusion_prompt(about: &About, conversation: &str) -> String {
    format!(
        "This is a brainstorm between a human and an AI about {target}. Draw its conclusion, to be \
         kept as a short note on that part of the product: what was decided, and what remains open, \
         if anything. Write it for the human who will read the note in a month: at most 120 words, \
         plain sentences, no preamble. Output the note only.\n\n\
         The note so far: {note}\n\n<conversation>\n{conversation}\n</conversation>",
        target = if about.target.is_empty() {
            "a part of their product"
        } else {
            &about.target
        },
        note = if about.note.trim().is_empty() {
            "(empty)"
        } else {
            about.note.trim()
        },
    )
}

/// What the request is asked of — the brainstorm becomes an asking.
#[must_use]
pub fn request_prompt(about: &About, conversation: &str) -> String {
    format!(
        "This is a brainstorm between a human and an AI about {target}. Formulate the change it \
         asks for, as a request to consider when the next roadmap is built: the first line a short \
         title, then what should change and why, the constraints that came up, and how to know it \
         is done. At most 200 words, no preamble. Output the request only.\n\n\
         The note so far: {note}\n\n<conversation>\n{conversation}\n</conversation>",
        target = if about.target.is_empty() {
            "a part of their product"
        } else {
            &about.target
        },
        note = if about.note.trim().is_empty() {
            "(empty)"
        } else {
            about.note.trim()
        },
    )
}

#[cfg(test)]
mod tests {
    use harness_core::domain::Issue;

    use super::*;

    #[test]
    fn the_mockup_rules_show_the_manifest_shape() {
        let rules = mockup_rules(&About::default());
        assert!(
            rules.contains(&crate::domain::mockup::manifest_shape()),
            "{rules}"
        );
        assert!(rules.contains("or a list of files"), "{rules}");
    }
    use proptest::prelude::*;

    fn turn(role: &str, text: &str) -> Turn {
        Turn {
            role: role.to_string(),
            text: text.to_string(),
        }
    }

    const TRANSCRIPT: &str = r#"{"type":"bridge-session","sessionId":"x"}
{"type":"user","message":{"role":"user","content":"<system-reminder>ignore me</system-reminder>"}}
{"type":"user","message":{"role":"user","content":"Should name be unique per campaign?"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Let me look at the migration."},{"type":"tool_use","name":"Bash","input":{"command":"git show main_agent:migrations/0002.sql"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"CREATE TABLE ..."}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Yes: an index already enforces it."}]}}
{"type":"user","isMeta":true,"message":{"role":"user","content":"meta"}}
not json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Then keep it."}]}}"#;

    #[test]
    fn a_transcript_reads_as_turns_without_the_noise() {
        let turns = turns_of(TRANSCRIPT);
        assert_eq!(turns.len(), 3, "{turns:?}");
        assert_eq!(
            turns[0],
            turn("human", "Should name be unique per campaign?")
        );
        assert_eq!(turns[1].role, "claude");
        assert!(
            turns[1]
                .text
                .starts_with("Let me look at the migration.\n[Bash {\"command\":\"git show")
        );
        assert!(
            turns[1]
                .text
                .ends_with("Yes: an index already enforces it.")
        );
        assert!(!turns[1].text.contains("hmm"), "thinking is left out");
        assert_eq!(turns[2], turn("human", "Then keep it."));
    }

    #[test]
    fn a_mockup_session_is_briefed_to_edit_and_a_talk_to_read() {
        let about = About {
            board: "mockup--main--home".into(),
            target: "component RiskBadge".into(),
            context: String::new(),
            note: "show a late customer".into(),
            kind: "request".into(),
            page: "pages/home.html".into(),
            branch: "main".into(),
            schema: String::new(),
            milestone: String::new(),
            component: String::new(),
            source: String::new(),
            rework: false,
        };
        let text = briefing(&about, &Handback::Nothing);
        assert!(text.contains("a *request*") && text.contains("Do not edit any file"));
        let drawn = About {
            kind: "link".into(),
            page: "pages/home.html".into(),
            source: "https://www.figma.com/design/abc/Home?node-id=1-2".into(),
            ..about.clone()
        };
        assert!(briefing(&drawn, &Handback::Nothing).contains("Figma frame"));
        let design = About {
            kind: "design".into(),
            component: "RiskBadge".into(),
            note: "name: RiskBadge\nneeds: credit.calculateRisk".into(),
            ..about.clone()
        };
        let made = briefing(&design, &Handback::Nothing);
        assert!(made.contains("Inject it there") && made.contains("`RiskBadge`"));
        assert!(
            first_prompt(&design)
                .expect("a prompt")
                .starts_with("Inject the micro-frontend `RiskBadge`")
        );
        let alone = About {
            page: String::new(),
            ..design
        };
        assert!(briefing(&alone, &Handback::Nothing).contains("Do not inject it in a page"));
        assert!(text.contains("show a late customer"));
        assert!(text.contains("branch `main`"));
        assert_eq!(
            first_prompt(&about).as_deref(),
            Some("show a late customer")
        );
        let link = About {
            kind: "link".into(),
            page: "pages/credit-review.html".into(),
            note: "assess a limit".into(),
            ..about.clone()
        };
        assert!(briefing(&link, &Handback::Nothing).contains("new page"));
        assert!(
            first_prompt(&link)
                .expect("a page to make")
                .contains("`pages/credit-review.html`")
        );
        let talk = About {
            kind: "brainstorm".into(),
            ..about
        };
        let text = briefing(&talk, &Handback::Nothing);
        assert!(text.contains("Do not edit any file"));
        assert!(!text.contains("edit only under"));
    }

    #[test]
    fn a_light_conversation_comes_back_whole() {
        let c = Conversation {
            turns: vec![turn("human", "a"), turn("claude", "b")],
            summary: None,
        };
        assert_eq!(
            c.handback(),
            Handback::Whole("Human: a\n\nClaude: b".to_string())
        );
        assert_eq!(Conversation::default().handback(), Handback::Nothing);
    }

    #[test]
    fn a_heavy_one_is_summarized_once_then_reused_until_it_grows() {
        let big = "x".repeat(4 * 3_000); // 3k tokens a turn
        let mut c = Conversation {
            turns: (0..20)
                .map(|k| turn(if k % 2 == 0 { "human" } else { "claude" }, &big))
                .collect(),
            summary: None,
        };
        let Handback::Summarize { covers, older } = c.handback() else {
            panic!("a 60k conversation is summarized");
        };
        assert!(covers > 0 && covers < 20);
        assert!(tokens(&render(&c.turns[covers..])) <= RECENT + 3 * 20);
        assert!(older.starts_with("Human: xxx"));
        c.summarized("the gist".to_string(), covers);
        let Handback::Compacted { summary, recent } = c.handback() else {
            panic!("the kept summary is reused");
        };
        assert_eq!(summary, "the gist");
        assert_eq!(recent, render(&c.turns[covers..]));
        // Grown by more than REFRESH: summarized again.
        for _ in 0..8 {
            c.turns.push(turn("human", &big));
        }
        assert!(matches!(c.handback(), Handback::Summarize { .. }));
    }

    #[test]
    fn the_briefing_carries_the_pin_and_hands_back_the_conversation() {
        let about = About {
            board: "data-model".into(),
            target: "field character.name".into(),
            context: "type: varchar(40)".into(),
            note: "unique?".into(),
            kind: "brainstorm".into(),
            page: String::new(),
            component: String::new(),
            source: String::new(),
            branch: "milestone/3-campaigns".into(),
            schema: "CREATE TABLE character (name varchar(40));".into(),
            rework: false,
            milestone: milestone_words(&crate::ports::Milestone {
                issue: Issue {
                    number: 3,
                    title: "Campaigns".into(),
                    state: "OPEN".into(),
                    body: "A campaign knows its party.".into(),
                    ..Issue::default()
                },
                tasks: vec![Issue {
                    number: 12,
                    title: "List the PCs".into(),
                    state: "CLOSED".into(),
                    ..Issue::default()
                }],
            }),
        };
        let first = briefing(&about, &Handback::Nothing);
        assert!(first.contains("field character.name"));
        assert!(first.contains("type: varchar(40)"));
        assert!(first.contains("branch `milestone/3-campaigns`"));
        assert!(first.contains("<schema.sql>\nCREATE TABLE character"));
        assert!(first.contains(
            "<milestone>\n#3 Campaigns (OPEN)\nA campaign knows its party.\nIts tasks:\n  - #12 List the PCs (CLOSED)"
        ));
        assert!(!first.contains("<conversation>"));
        let again = briefing(&about, &Handback::Whole("Human: a".into()));
        assert!(again.contains("<conversation>\nHuman: a\n</conversation>"));
        assert!(request_prompt(&about, "Human: a").contains("next roadmap"));
        assert!(conclusion_prompt(&about, "Human: a").contains("unique?"));
        assert!(compact_prompt(&about, "Human: a").contains("where the discussion stands"));
    }

    proptest! {
        #[test]
        fn any_transcript_reads_without_a_panic(text in "\\PC{0,400}") {
            let _ = turns_of(&text);
        }

        #[test]
        fn the_recent_turns_always_hold_the_last_one(sizes in prop::collection::vec(1_usize..20_000, 1..30)) {
            let turns: Vec<Turn> = sizes.iter().map(|n| turn("human", &"y".repeat(*n))).collect();
            let start = recent_start(&turns);
            prop_assert!(start < turns.len());
        }
    }
}
