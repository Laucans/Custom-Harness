//! A factory's creation, step by step: what the human wrote, what the AI
//! asked and proposed, where it stands, and the gates between the steps —
//! then the questions put to the AI and the reading of its answers.
//!
//! The record is what the register keeps beside a factory being created
//! (`Factories::creation`, opaque to `core`): its shape is decided here.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The most questions one grilling round may bring.
pub const ROUND_MAX: usize = 8;

/// The steps, in order: the page's breadcrumb.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// The GitHub repository and the factory's folder — done before the
    /// record exists.
    Repository,
    /// What kind of factory it is.
    #[default]
    Kind,
    /// What the application does, in the human's words.
    Brief,
    /// The AI's questions, the human's answers.
    Grilling,
    /// The proof of concept's scope, as the AI proposes it.
    Scope,
    /// The design system: moodboard, logo, tokens, kit.
    Design,
    /// The proof of concept drawn as interactive mock-ups.
    Mockups,
    /// The mock-ups reviewed: changed, regenerated, or accepted.
    Review,
    /// The factory initialized: the roadmap task opened, the watch started.
    Initialize,
}

impl Step {
    /// Every step, in order.
    pub const ALL: [Self; 9] = [
        Self::Repository,
        Self::Kind,
        Self::Brief,
        Self::Grilling,
        Self::Scope,
        Self::Design,
        Self::Mockups,
        Self::Review,
        Self::Initialize,
    ];
}

/// The kinds of factory the harness can build. One today; the page shows
/// the others to come, unselectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Vertical slices, the architecture `init-repo` installs: what this
    /// plant is.
    Vertical,
}

impl Kind {
    /// What a human calls it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Vertical => "vertical architecture development factory",
        }
    }
}

/// One question of the grilling, and the human's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Its number in the creation, never reused.
    pub id: u32,
    /// The decision it asks for, in a few words.
    pub title: String,
    /// The question itself, the choices it offers.
    #[serde(default)]
    pub body: String,
    /// What the AI would answer.
    #[serde(default)]
    pub recommended: String,
    /// What the human answered; empty while open.
    #[serde(default)]
    pub answer: String,
    /// The round that asked it, counted from 1.
    #[serde(default)]
    pub round: u32,
}

impl Question {
    /// The human answered it.
    #[must_use]
    pub fn answered(&self) -> bool {
        !self.answer.trim().is_empty()
    }
}

/// Where a factory's creation stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Creation {
    /// The factory's name.
    pub name: String,
    /// Its repository, `owner/name`.
    pub slug: String,
    /// The step the human is on.
    pub step: Step,
    /// What kind of factory it is, once chosen.
    pub kind: Option<Kind>,
    /// What the application does.
    pub brief: String,
    /// Every question asked, in order, the deleted ones gone.
    pub questions: Vec<Question>,
    /// The AI said its last round left nothing to ask.
    pub grilled: bool,
    /// How many grilling rounds were asked.
    pub rounds: u32,
    /// The highest number a question was ever given, deleted ones included.
    pub last_id: u32,
    /// The proof of concept's scope, Markdown.
    pub scope: String,
    /// What the scope was proposed from (`fingerprint`); empty while never
    /// proposed. Differs from the fingerprint now: the decisions moved
    /// since.
    pub scope_from: String,
    /// The human retouched the scope since it was proposed: nothing
    /// proposes over it by itself.
    pub scope_edited: bool,
    /// The requirements the proof of concept must meet, each tied to the
    /// pages and the micro-frontends that carry it.
    pub requirements: Vec<Requirement>,
    /// The highest number a requirement was ever given.
    pub last_requirement: u32,
    /// How many times the mock-ups were sent back to the brief.
    pub regenerations: u32,
    /// The mock-ups were drawn; what the session said of them.
    pub mocked: Option<String>,
    /// The human accepted the proof of concept.
    pub accepted: bool,
    /// The accepted mock-up was made again of micro-frontends; what the
    /// session said of it.
    pub componentized: Option<String>,
    /// Where the mock-up was put by the initialization: `<branch>@<commit>`
    /// once published, else the `mockup/*` branch it was saved on.
    pub mockup_at: String,
    /// The design system's publication: its commit, and why it was not
    /// carried onto the integration branch when it was not.
    pub design_at: String,
    /// Why the design system was not carried onto the integration branch;
    /// empty when it was.
    pub design_refused: String,
    /// The roadmap task the initialization opened.
    pub issue: String,
}

/// One requirement of the proof of concept: what it must do, and where
/// the mock-up shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Requirement {
    /// Its number in the creation, never reused.
    pub id: u32,
    /// The requirement, in a few words.
    pub title: String,
    /// What it asks for: the behaviour, the rule, the data.
    pub body: String,
    /// The pages of the mock-up that carry it (`pages/…`).
    pub pages: Vec<String>,
    /// The micro-frontends that carry it, by name.
    pub components: Vec<String>,
    /// Written or retouched by the human: a new reading keeps it.
    pub human: bool,
}

/// What a gate reads that the record does not hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// The design system is validated, on its branch.
    pub design_validated: bool,
    /// The kit's components ticked and not generated yet, or generating.
    pub kit_waiting: usize,
}

/// What a step needs done before it may be entered; `None` when nothing.
fn needs(c: &Creation, step: Step, facts: Facts) -> Option<&'static str> {
    match step {
        Step::Repository | Step::Kind => None,
        Step::Brief => c.kind.is_none().then_some("choose the kind of factory"),
        Step::Grilling => c
            .brief
            .trim()
            .is_empty()
            .then_some("describe what the application does"),
        Step::Scope => {
            // The AI may find nothing to ask of a complete brief; the human
            // may delete every question: neither keeps the scope closed.
            if c.questions.is_empty() && !c.grilled {
                Some("let the AI ask its questions")
            } else if c.questions.iter().any(|q| !q.answered()) {
                Some("answer every question, or delete it")
            } else {
                None
            }
        }
        Step::Design => c
            .scope
            .trim()
            .is_empty()
            .then_some("settle the proof of concept's scope"),
        Step::Mockups => {
            if !facts.design_validated {
                Some("validate the design system")
            } else if facts.kit_waiting > 0 {
                Some("let the kit's components be generated")
            } else {
                None
            }
        }
        Step::Review => c.mocked.is_none().then_some("draw the mock-ups"),
        Step::Initialize => (!c.accepted).then_some("accept the proof of concept"),
    }
}

impl Creation {
    /// A creation that starts at the kind of factory: the repository exists.
    #[must_use]
    pub fn new(name: &str, slug: &str) -> Self {
        Self {
            name: name.to_string(),
            slug: slug.to_string(),
            ..Self::default()
        }
    }

    /// Whether `step` may be entered: every step up to it has what it needs.
    ///
    /// # Errors
    ///
    /// What is missing first, in words.
    pub fn may_enter(&self, step: Step, facts: Facts) -> Result<(), String> {
        Step::ALL
            .iter()
            .take_while(|s| **s <= step)
            .find_map(|s| needs(self, *s, facts))
            .map_or(Ok(()), |why| Err(why.to_string()))
    }

    /// The furthest step that may be entered now.
    #[must_use]
    pub fn reachable(&self, facts: Facts) -> Step {
        Step::ALL
            .iter()
            .take_while(|s| needs(self, **s, facts).is_none())
            .last()
            .copied()
            .unwrap_or_default()
    }

    /// Goes to `step`, when it may be entered.
    ///
    /// # Errors
    ///
    /// What is missing first; the record is unchanged.
    pub fn go(&mut self, step: Step, facts: Facts) -> Result<(), String> {
        if step == Step::Repository {
            return Err("the repository is made: it cannot be made again".to_string());
        }
        self.may_enter(step, facts)?;
        self.step = step;
        Ok(())
    }

    /// The human's edits, applied: the fields they sent, the questions they
    /// kept — in that order, the others deleted — with their answers.
    pub fn edit(&mut self, edit: Edit) {
        if let Some(kind) = edit.kind {
            self.kind = Some(kind);
        }
        if let Some(brief) = edit.brief {
            self.brief = brief;
        }
        if let Some(kept) = edit.questions {
            let mut questions = Vec::with_capacity(kept.len());
            for answer in kept {
                if let Some(mut q) = self
                    .questions
                    .iter()
                    .find(|q| {
                        q.id == answer.id && !questions.iter().any(|k: &Question| k.id == q.id)
                    })
                    .cloned()
                {
                    q.answer = answer.answer;
                    questions.push(q);
                }
            }
            self.questions = questions;
        }
        if let Some(scope) = edit.scope {
            if scope.trim() != self.scope.trim() {
                self.scope_edited = true;
            }
            self.scope = scope;
        }
        if let Some(requirements) = edit.requirements {
            let mut kept = Vec::with_capacity(requirements.len());
            for mut r in requirements {
                if r.id == 0 {
                    self.last_requirement = self.last_requirement.saturating_add(1);
                    r.id = self.last_requirement;
                }
                if r.title.trim().is_empty() && r.body.trim().is_empty() {
                    continue;
                }
                if kept.iter().any(|k: &Requirement| k.id == r.id) {
                    continue;
                }
                // Sent by the page: the human's, from now on.
                r.human = true;
                kept.push(r);
            }
            self.requirements = kept;
        }
    }

    /// What the scope and the requirements are proposed from: the kind, the
    /// brief and the decisions, hashed — a change there makes a proposal
    /// stale.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        product(self).hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// The scope was proposed from decisions that moved since.
    #[must_use]
    pub fn scope_stale(&self) -> bool {
        !self.scope_from.is_empty() && self.scope_from != self.fingerprint()
    }

    /// The scope as the AI proposed it, from the decisions as they are now.
    pub fn propose(&mut self, scope: String) {
        self.scope = scope;
        self.scope_from = self.fingerprint();
        self.scope_edited = false;
    }

    /// The requirements as the AI read them off the scope and the mock-up:
    /// numbered after the last ever given, the human's own kept before
    /// them.
    pub fn add_requirements(&mut self, read: Vec<Read>) {
        self.requirements.retain(|r| r.human);
        for r in read {
            if r.title.trim().is_empty() {
                continue;
            }
            self.last_requirement = self.last_requirement.saturating_add(1);
            self.requirements.push(Requirement {
                id: self.last_requirement,
                title: r.title,
                body: r.body,
                pages: r.pages,
                components: r.components,
                human: false,
            });
        }
    }

    /// A grilling round, added: its questions numbered after the last ever
    /// given. A round with nothing to ask marks the grilling done.
    pub fn add_round(&mut self, round: Round) {
        self.rounds = self.rounds.saturating_add(1);
        let mut next = self
            .questions
            .iter()
            .map(|q| q.id)
            .max()
            .unwrap_or(0)
            .max(self.last_id);
        for asked in round.questions.into_iter().take(ROUND_MAX) {
            next = next.saturating_add(1);
            self.questions.push(Question {
                id: next,
                title: asked.title,
                body: asked.body,
                recommended: asked.recommended,
                answer: String::new(),
                round: self.rounds,
            });
        }
        self.last_id = next;
        self.grilled = round.done;
    }

    /// Back to the brief to draw the mock-ups again: everything written is
    /// kept, the mock-ups are not.
    pub fn regenerate(&mut self) {
        self.regenerations = self.regenerations.saturating_add(1);
        self.mocked = None;
        self.accepted = false;
        self.componentized = None;
        self.grilled = false;
        self.step = Step::Brief;
    }
}

/// The human's edits, as the page sends them: only what it sends changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Edit {
    /// The kind of factory chosen.
    pub kind: Option<Kind>,
    /// What the application does.
    pub brief: Option<String>,
    /// The questions kept, in order, with their answers.
    pub questions: Option<Vec<Answer>>,
    /// The scope, retouched.
    pub scope: Option<String>,
    /// The requirements kept, in order, as the human left them; one with
    /// no id is new.
    pub requirements: Option<Vec<Requirement>>,
}

/// One question kept, and its answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Answer {
    /// The question's number.
    pub id: u32,
    /// The answer; empty while open.
    #[serde(default)]
    pub answer: String,
}

/// One question as the AI asks it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Asked {
    /// The decision, in a few words.
    pub title: String,
    /// The question, the choices.
    pub body: String,
    /// What the AI would answer.
    pub recommended: String,
}

/// One requirement as the AI reads it off the scope and the mock-up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Read {
    /// The requirement, in a few words.
    pub title: String,
    /// What it asks for.
    pub body: String,
    /// The pages that carry it, as `mock/pages.json` lists their paths.
    pub pages: Vec<String>,
    /// The micro-frontends that carry it, by name.
    pub components: Vec<String>,
}

/// The requirements, as the AI answers them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Requirements {
    /// The requirements read.
    pub requirements: Vec<Read>,
}

/// The requirements out of the AI's answer: those with a title, their
/// pages and micro-frontends kept to the ones the mock-up has.
///
/// # Errors
///
/// The answer carries no requirements.
pub fn parse_requirements(
    answer: &str,
    pages: &[String],
    components: &[String],
) -> Result<Vec<Read>, String> {
    let object = object_in(answer).ok_or("the AI answered no requirements")?;
    let read: Requirements = serde_json::from_str(object)
        .map_err(|e| format!("the AI's requirements are not readable: {e}"))?;
    let mut kept = Vec::new();
    for mut r in read.requirements {
        r.title = r.title.trim().to_string();
        r.body = r.body.trim().to_string();
        if r.title.is_empty() {
            continue;
        }
        r.pages.retain(|p| pages.iter().any(|known| known == p));
        r.components
            .retain(|c| components.iter().any(|known| known == c));
        kept.push(r);
    }
    if kept.is_empty() {
        return Err("the AI read no requirement off the scope".to_string());
    }
    Ok(kept)
}

/// A grilling round, as the AI answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Round {
    /// The questions of the frontier.
    pub questions: Vec<Asked>,
    /// Nothing is left to ask.
    pub done: bool,
}

/// The JSON object an answer carries, wherever it sits in the text: the AI
/// may wrap it in a fence or a sentence.
fn object_in(answer: &str) -> Option<&str> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    answer.get(start..=end)
}

/// A grilling round out of the AI's answer: the questions with a title, at
/// most [`ROUND_MAX`].
///
/// # Errors
///
/// The answer carries no round.
pub fn parse_round(answer: &str) -> Result<Round, String> {
    let object = object_in(answer).ok_or("the AI answered no round of questions")?;
    let mut round: Round = serde_json::from_str(object)
        .map_err(|e| format!("the AI's round of questions is not readable: {e}"))?;
    round.questions.retain(|q| !q.title.trim().is_empty());
    round.questions.truncate(ROUND_MAX);
    for q in &mut round.questions {
        q.title = q.title.trim().to_string();
        q.body = q.body.trim().to_string();
        q.recommended = q.recommended.trim().to_string();
    }
    // A round that asks something is not the last, whatever it says.
    round.done = round.done && round.questions.is_empty();
    Ok(round)
}

/// The text of an answer, its Markdown fence taken off when it is wrapped
/// in one.
#[must_use]
pub fn unfenced(answer: &str) -> String {
    let text = answer.trim();
    let Some(rest) = text.strip_prefix("```") else {
        return text.to_string();
    };
    let body = rest.split_once('\n').map_or("", |(_, body)| body);
    body.strip_suffix("```").unwrap_or(body).trim().to_string()
}

/// What the human wants built: the kind, the brief, the decisions taken.
fn product(c: &Creation) -> String {
    let mut text = format!(
        "The application is `{name}`, built by a {kind}.\n\nWhat the application does, in the human's words:\n\n{brief}\n",
        name = c.name,
        kind = c.kind.map_or("development factory", Kind::label),
        brief = c.brief.trim(),
    );
    let answered: Vec<&Question> = c.questions.iter().filter(|q| q.answered()).collect();
    if !answered.is_empty() {
        text.push_str("\nDecisions already taken (question → the human's answer):\n\n");
        for q in answered {
            let _ = writeln!(text, "- **{}** — {}", q.title, q.answer.trim());
        }
    }
    text
}

/// The question that asks the next grilling round.
#[must_use]
pub fn grill_prompt(c: &Creation) -> String {
    let mut text = String::from(
        "You are grilling a human about an application they want a development factory to \
         build, before its proof of concept is scoped. Interview them relentlessly until you \
         share their understanding: map the decisions as a design tree, where every decision \
         branches into the ones that hang off it. Ask in rounds. A round is the frontier: every \
         decision whose prerequisites are already settled — the questions that can be asked now \
         without guessing at answers not heard yet. A question whose answer depends on another \
         question of the same round belongs to a later round. Cover who uses the application and \
         why, what they do with it step by step, the data it keeps, the rules it enforces, what \
         is out of scope, and what would prove it works. Ask decisions, never facts a developer \
         can look up. Give each question your recommended answer.\n\n\
         Answer with one JSON object and nothing else: \
         {\"questions\": [{\"title\": \"the decision in a few words\", \"body\": \"the question, \
         the choices it offers\", \"recommended\": \"your recommended answer\"}], \"done\": false}. \
         At most ",
    );
    let _ = write!(
        text,
        "{ROUND_MAX} questions. When the tree has no open branch left, answer \
         {{\"questions\": [], \"done\": true}}. Write in the language of the human's brief.\n\n"
    );
    text.push_str(&product(c));
    let open: Vec<&Question> = c.questions.iter().filter(|q| !q.answered()).collect();
    if !open.is_empty() {
        text.push_str("\nAlready asked, waiting for an answer — do not ask them again:\n\n");
        for q in open {
            let _ = writeln!(text, "- {}", q.title);
        }
    }
    text
}

/// The question that proposes the proof of concept's scope.
#[must_use]
pub fn scope_prompt(c: &Creation) -> String {
    let mut text = String::from(
        "You scope the proof of concept of an application a development factory will build. \
         The proof of concept is the smallest thing that proves the application's value to the \
         human: a few screens, one or two user journeys end to end, the data they need — nothing \
         that can wait. Answer in Markdown and nothing else, with these sections: `## Goal` (one \
         paragraph: what the proof of concept proves), `## Screens` (one bullet per screen: its \
         name in bold, then what the user sees and does there), `## Journeys` (the user journeys \
         it carries, step by step), `## Data` (the things it keeps, with their main fields), \
         `## Left out` (what waits for later, and why), `## Proof` (how the human will know it \
         works). Write in the language of the human's brief.\n\n",
    );
    text.push_str(&product(c));
    if !c.scope.trim().is_empty() {
        let _ = write!(
            text,
            "\nThe scope proposed last time, which the human may have retouched — start from it, \
             keep what the decisions still support:\n\n{}\n",
            c.scope.trim()
        );
    }
    text
}

/// The session that draws the proof of concept as the product's mock-up:
/// `rules` are the mock-up's own (`note_session::mockup_rules`), `kit` the
/// design system's components, one line each.
#[must_use]
pub fn mockups_prompt(c: &Creation, rules: &str, kit: &str) -> String {
    let mut text = rules.to_string();
    text.push_str(
        "\nDraw the proof of concept below as the product's mock-up: one page per screen of its \
         scope under `mock/pages/`, each registered in `mock/pages.json` with its title and its \
         objective (what the user does there), a `mock/index.html` that lists them, the fixtures \
         they read under `mock/fixtures/` with believable example data in the language of the \
         brief — never a lorem ipsum — and the links between the pages that carry the journeys, \
         so a human can click through them. Build every page from the design system: \
         `mock/design/tokens.css` holds its variables (`var(--…)`, never a colour or a size \
         written by hand), and its kit components are dropped as \
         `<div data-kit=\"<Name>\" data-props='{…}'></div>`, which `../mock.js` fills — use one \
         wherever it fits rather than writing that markup again. When there is no page yet, \
         start the mock-up. When pages already exist from an earlier draft, replace them: this \
         proof of concept is the one to show. Say in a few lines which pages you made.\n\n",
    );
    if kit.trim().is_empty() {
        text.push_str(
            "The kit has no generated component yet: style the pages with the tokens alone.\n\n",
        );
    } else {
        let _ = write!(
            text,
            "The kit's components, one line each:\n\n{}\n\n",
            kit.trim()
        );
    }
    text.push_str(&product(c));
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The session that makes the accepted mock-up again of micro-frontends,
/// keeping what the human saw: `rules` are the mock-up's own, `kit` the
/// design system's components.
#[must_use]
pub fn componentize_prompt(c: &Creation, rules: &str, kit: &str) -> String {
    let mut text = rules.to_string();
    text.push_str(
        "\nThe human accepted this proof of concept's mock-up, drawn fast: its pages may carry \
         their markup inline. Make it again of **micro-frontends**, as the product's architecture \
         wants it, without changing what the human sees. Read every page listed in \
         `mock/pages.json`; cut each into the Micro-UIs it is made of — a unit a user recognises \
         (an invoice list, an upload form, a balance), named in PascalCase, its system the bounded \
         context it belongs to — and for each one: write its fragment \
         `mock/components/<Name>.html` (its markup, its style on the design system's variables, its \
         script finding its root as `document.currentScript.closest('[data-component]')`, its data \
         read from `mock/fixtures/<Name>.json`, the kit's components kept as `data-kit` elements), \
         and list it under `components` of `mock/pages.json` (name, system, description, fragment, \
         fixture, props; `needs` and `actions` named as the Capabilities and DataCapabilities it \
         would read and trigger, `<system>.<verb><Thing>`). A micro-frontend shared by several pages \
         is written once. Then rewrite each page as layout only: its regions, and in them \
         `<div data-component=\"<Name>\" data-props='{…}'></div>` for each micro-frontend, with \
         `<script src=\"../mock.js\"></script>` — no business markup left inline. Keep each page's \
         look, texts, links and example data as they are. List each page's micro-frontends under its \
         `components` in `mock/pages.json`. Say in a few lines which micro-frontends you made.\n\n",
    );
    if !kit.trim().is_empty() {
        let _ = write!(
            text,
            "The kit's components, one line each:\n\n{}\n\n",
            kit.trim()
        );
    }
    text.push_str(&product(c));
    text
}

/// The micro-frontends a page injects, by name, in order, each once.
#[must_use]
pub fn injected(html: &str) -> Vec<String> {
    const ATTRIBUTE: &str = "data-component=";
    let mut names: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(ATTRIBUTE) {
        rest = rest.get(at + ATTRIBUTE.len()..).unwrap_or_default();
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let value = rest.get(1..).unwrap_or_default();
        let Some(end) = value.find(quote) else {
            break;
        };
        let name = value.get(..end).unwrap_or_default().trim();
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = value.get(end..).unwrap_or_default();
    }
    names
}

/// What keeps a mock-up from being made of micro-frontends: a page that
/// injects none, one that injects a micro-frontend `mock/pages.json` does
/// not list, one listed with no fragment. `pages` are each page's path and
/// markup; `components` each listed micro-frontend and whether its fragment
/// is there. Empty: it is.
#[must_use]
pub fn not_componentized(pages: &[(String, String)], components: &[(String, bool)]) -> Vec<String> {
    if pages.is_empty() {
        return vec!["the mock-up has no page".to_string()];
    }
    let mut problems = Vec::new();
    for (path, html) in pages {
        let names = injected(html);
        if names.is_empty() {
            problems.push(format!("`{path}` injects no micro-frontend"));
        }
        for name in names {
            match components.iter().find(|(n, _)| *n == name) {
                None => problems.push(format!(
                    "`{path}` injects `{name}`, which mock/pages.json does not list"
                )),
                Some((_, false)) => problems.push(format!("`{name}` has no fragment")),
                Some((_, true)) => {}
            }
        }
    }
    problems.dedup();
    problems
}

/// One page of the mock-up, as the roadmap task and the record name it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageRef {
    /// Its title.
    pub title: String,
    /// Its path under `mock/`.
    pub path: String,
    /// What the user does there, as `mock/pages.json` says.
    pub objective: String,
}

/// One micro-frontend of the mock-up, with the Capabilities it names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentRef {
    /// Its name.
    pub name: String,
    /// Its system.
    pub system: String,
    /// What it shows and lets the user do.
    pub description: String,
    /// The Capabilities it reads.
    pub needs: Vec<String>,
    /// The `DataCapabilities` it triggers.
    pub actions: Vec<String>,
}

/// A pin left on the mock-up that no backlog issue carries yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PinRef {
    /// Its kind: request, link, design.
    pub kind: String,
    /// The board it sits on.
    pub board: String,
    /// The part it points at.
    pub target: String,
    /// Its text.
    pub text: String,
}

/// The design system, as published.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignRef {
    /// When it was validated.
    pub validated_at: String,
    /// How many tokens it has.
    pub tokens: usize,
    /// The kit's components generated.
    pub components: Vec<String>,
}

/// What the initialization gathered around the record: the mock-up as
/// accepted, the design system, the pins left. What the roadmap task and
/// the record kept in the repository carry beside the decisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Context {
    /// The pages, in the manifest's order.
    pub pages: Vec<PageRef>,
    /// The micro-frontends, in the manifest's order.
    pub components: Vec<ComponentRef>,
    /// The pins no backlog issue carries.
    pub pins: Vec<PinRef>,
    /// The design system.
    pub design: DesignRef,
}

/// Where the record of the creation is kept in the repository, beside the
/// mock-up it describes: the planner and every later session read it there.
pub const RECORD_JSON: &str = "proof-of-concept.json";
/// The same, written for a human.
pub const RECORD_MD: &str = "proof-of-concept.md";

/// The record and its context, as one JSON document for the repository.
///
/// # Errors
///
/// Never in practice: the record is plain data.
pub fn record_json(c: &Creation, ctx: &Context) -> Result<String, String> {
    #[derive(Serialize)]
    struct Kept<'a> {
        version: u32,
        name: &'a str,
        repository: &'a str,
        kind: Option<Kind>,
        brief: &'a str,
        decisions: Vec<&'a Question>,
        scope: &'a str,
        requirements: &'a [Requirement],
        pages: &'a [PageRef],
        components: &'a [ComponentRef],
        pins: &'a [PinRef],
        design: &'a DesignRef,
        mockup_at: &'a str,
        design_at: &'a str,
    }
    serde_json::to_string_pretty(&Kept {
        version: 1,
        name: &c.name,
        repository: &c.slug,
        kind: c.kind,
        brief: &c.brief,
        decisions: c.questions.iter().filter(|q| q.answered()).collect(),
        scope: &c.scope,
        requirements: &c.requirements,
        pages: &ctx.pages,
        components: &ctx.components,
        pins: &ctx.pins,
        design: &ctx.design,
        mockup_at: &c.mockup_at,
        design_at: &c.design_at,
    })
    .map_err(|e| e.to_string())
}

/// Where the mock-up lives, in words: for the task and the record.
fn mockup_place(c: &Creation) -> String {
    if c.mockup_at.is_empty() {
        "under `mock/` on the integration branch".to_string()
    } else if c.mockup_at.starts_with("mockup/") {
        format!(
            "under `mock/` on the branch `{}` — the integration branch refused it",
            c.mockup_at
        )
    } else {
        format!(
            "under `mock/` on the integration branch (`{}`)",
            c.mockup_at
        )
    }
}

/// The roadmap task's design-system section: where it is, whether it was
/// carried, what it holds.
fn design_section(c: &Creation, design: &DesignRef) -> String {
    let mut body = String::from("\n## Design system\n\nOn the `design-system` branch");
    if !c.design_at.is_empty() {
        let _ = write!(body, " (`{}`)", c.design_at);
    }
    if c.design_refused.is_empty() {
        body.push_str(", carried onto the integration branch");
    } else {
        let _ = write!(
            body,
            "; **not carried onto the integration branch**: {}",
            c.design_refused
        );
    }
    if !design.validated_at.is_empty() {
        let _ = write!(body, ". Validated {}", design.validated_at);
    }
    let _ = write!(body, ". {} token(s)", design.tokens);
    if !design.components.is_empty() {
        let _ = write!(body, "; the kit: {}", design.components.join(", "));
    }
    body.push_str(".\n");
    body
}

/// The roadmap task the factory starts on: the proof of concept as decided
/// — the brief, the decisions, the scope, the requirements — with the pages
/// that mock it, the micro-frontends and the Capabilities they name, the
/// design system, and the pins left on the mock-up.
#[must_use]
pub fn roadmap_issue(c: &Creation, ctx: &Context) -> (String, String) {
    let title = format!("Proof of concept — {}", c.name);
    let mut body = format!(
        "The proof of concept of `{name}`, as decided when the factory was created. \
         The whole record is `mock/{record}` {place}.\n\n\
         ## What the application does\n\n{brief}\n\n## Scope\n\n{scope}\n",
        name = c.name,
        record = RECORD_JSON,
        place = mockup_place(c),
        brief = c.brief.trim(),
        scope = c.scope.trim(),
    );
    let answered: Vec<&Question> = c.questions.iter().filter(|q| q.answered()).collect();
    if !answered.is_empty() {
        body.push_str("\n## Decisions\n\n");
        for q in answered {
            let _ = writeln!(body, "- **{}** — {}", q.title, q.answer.trim());
            if !q.body.trim().is_empty() {
                let _ = writeln!(body, "  (asked: {})", q.body.trim().replace('\n', " "));
            }
        }
    }
    if !c.requirements.is_empty() {
        body.push_str("\n## Requirements\n\n");
        for r in &c.requirements {
            let _ = write!(body, "- **R{} {}**", r.id, r.title.trim());
            if !r.body.trim().is_empty() {
                let _ = write!(body, " — {}", r.body.trim().replace('\n', " "));
            }
            let mut on = Vec::new();
            on.extend(r.pages.iter().map(|p| format!("`mock/{p}`")));
            on.extend(r.components.iter().map(|m| format!("`{m}`")));
            if !on.is_empty() {
                let _ = write!(body, " — on {}", on.join(", "));
            }
            body.push('\n');
        }
    }
    if !ctx.pages.is_empty() {
        let _ = write!(
            body,
            "\n## Mock-ups\n\nThe screens as the human accepted them, {} (contract N).\n\n",
            mockup_place(c)
        );
        for p in &ctx.pages {
            let _ = write!(body, "- {} — `mock/{}`", p.title, p.path);
            if !p.objective.trim().is_empty() {
                let _ = write!(body, " — {}", p.objective.trim());
            }
            body.push('\n');
        }
    }
    if !ctx.components.is_empty() {
        body.push_str(
            "\n## Micro-frontends\n\nAs `mock/pages.json` lists them, with the Capabilities \
             they read (`needs`) and the DataCapabilities they trigger (`actions`).\n\n",
        );
        for m in &ctx.components {
            let _ = write!(body, "- **{}**", m.name);
            if !m.system.is_empty() {
                let _ = write!(body, " ({})", m.system);
            }
            if !m.description.trim().is_empty() {
                let _ = write!(body, " — {}", m.description.trim());
            }
            if !m.needs.is_empty() {
                let _ = write!(body, " — needs {}", m.needs.join(", "));
            }
            if !m.actions.is_empty() {
                let _ = write!(body, " — actions {}", m.actions.join(", "));
            }
            body.push('\n');
        }
    }
    body.push_str(&design_section(c, &ctx.design));
    if !ctx.pins.is_empty() {
        body.push_str(
            "\n## Requests left on the mock-up\n\nPins the human left on the pages, that no \
             backlog issue carries: intentions for the development to take up.\n\n",
        );
        for p in &ctx.pins {
            let _ = write!(body, "- *{}* on `{}`", p.kind, p.board);
            if !p.target.is_empty() {
                let _ = write!(body, " at `{}`", p.target);
            }
            let _ = writeln!(body, ": {}", p.text.trim().replace('\n', " "));
        }
    }
    (title, body)
}

/// The question that reads the requirements off the scope and the mock-up:
/// each tied to the pages and the micro-frontends that carry it.
#[must_use]
pub fn requirements_prompt(c: &Creation, ctx: &Context) -> String {
    let mut text = String::from(
        "You read the requirements of a proof of concept off its scope and its mock-up, for \
         the human to retouch before the development starts. A requirement is one thing the \
         application must do or keep: a user journey's step, a rule it enforces, a thing it \
         keeps, a proof the human expects — never a page or a component as such. Write each \
         in the language of the human's brief, and tie it to the pages and the micro-frontends \
         of the mock-up that carry it, by the exact paths and names listed below. Answer with \
         one JSON object and nothing else: {\"requirements\": [{\"title\": \"the requirement \
         in a few words\", \"body\": \"what it asks for, one or two sentences\", \"pages\": \
         [\"pages/….html\"], \"components\": [\"Name\"]}]}. Between eight and twenty-five \
         requirements, the scope's order.\n\n",
    );
    text.push_str(&product(c));
    let _ = write!(text, "\nThe scope:\n\n{}\n", c.scope.trim());
    if !ctx.pages.is_empty() {
        text.push_str("\nThe mock-up's pages (path — title — objective):\n\n");
        for p in &ctx.pages {
            let _ = writeln!(
                text,
                "- `{}` — {} — {}",
                p.path,
                p.title,
                p.objective.trim()
            );
        }
    }
    if !ctx.components.is_empty() {
        text.push_str(
            "\nThe micro-frontends (name — system — what it does — needs — actions):\n\n",
        );
        for m in &ctx.components {
            let _ = writeln!(
                text,
                "- `{}` — {} — {} — {} — {}",
                m.name,
                m.system,
                m.description.trim(),
                m.needs.join(", "),
                m.actions.join(", ")
            );
        }
    }
    if c.requirements.iter().any(|r| r.human) {
        text.push_str("\nThe human wrote these already; they are kept, do not repeat them:\n\n");
        for r in c.requirements.iter().filter(|r| r.human) {
            let _ = writeln!(text, "- {}", r.title.trim());
        }
    }
    text
}

/// What the initialization says of the design system it published:
/// `refused` is why it was not carried onto the integration branch, empty
/// when it was.
#[must_use]
pub fn design_published(refused: &str) -> String {
    if refused.is_empty() {
        "the design system is on its branch and the integration branch".to_string()
    } else {
        format!("the design system is on its branch; not carried: {refused}")
    }
}

/// A repository name GitHub takes: letters, digits, `-`, `_`, `.`, at most
/// a hundred, not `.` or `..`.
///
/// # Errors
///
/// What is wrong with it.
pub fn repository_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name the repository".to_string());
    }
    if name.len() > 100 {
        return Err("a repository name is 100 characters at most".to_string());
    }
    if name == "." || name == ".." {
        return Err("a repository cannot be named . or ..".to_string());
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(format!(
            "a repository name takes letters, digits, -, _ and . only, not `{c}`"
        ));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    type Spoil = fn(&mut Creation);

    fn previous(step: Step) -> Option<Step> {
        let at = Step::ALL.iter().position(|s| *s == step)?;
        at.checked_sub(1).and_then(|i| Step::ALL.get(i)).copied()
    }

    const DESIGNED: Facts = Facts {
        design_validated: true,
        kit_waiting: 0,
    };

    fn question(id: u32, answer: &str) -> Question {
        Question {
            id,
            title: format!("q{id}"),
            answer: answer.to_string(),
            ..Question::default()
        }
    }

    fn ready() -> Creation {
        Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            questions: vec![question(1, "yes")],
            scope: "## Goal".to_string(),
            mocked: Some("two pages".to_string()),
            accepted: true,
            ..Creation::new("app", "me/app")
        }
    }

    #[test]
    fn a_new_creation_starts_at_the_kind_and_reaches_no_further() {
        let c = Creation::new("app", "me/app");
        assert_eq!(c.step, Step::Kind);
        assert_eq!(c.reachable(DESIGNED), Step::Kind);
        assert_eq!(
            c.may_enter(Step::Brief, DESIGNED),
            Err("choose the kind of factory".to_string())
        );
    }

    #[test]
    fn each_gate_names_what_is_missing_first() {
        let full = ready();
        assert_eq!(full.reachable(DESIGNED), Step::Initialize);
        let cases: [(Spoil, Step, &str); 7] = [
            (|c| c.kind = None, Step::Brief, "choose the kind of factory"),
            (
                |c| c.brief = "  ".to_string(),
                Step::Grilling,
                "describe what the application does",
            ),
            (
                |c| c.questions.clear(),
                Step::Scope,
                "let the AI ask its questions",
            ),
            (
                |c| c.questions.push(question(2, " ")),
                Step::Scope,
                "answer every question, or delete it",
            ),
            (
                |c| c.scope = String::new(),
                Step::Design,
                "settle the proof of concept's scope",
            ),
            (|c| c.mocked = None, Step::Review, "draw the mock-ups"),
            (
                |c| c.accepted = false,
                Step::Initialize,
                "accept the proof of concept",
            ),
        ];
        for (spoil, blocked, why) in cases {
            let mut c = ready();
            spoil(&mut c);
            assert_eq!(
                c.may_enter(blocked, DESIGNED),
                Err(why.to_string()),
                "{why}"
            );
            assert_eq!(
                c.reachable(DESIGNED),
                previous(blocked).expect("has one"),
                "{why}"
            );
            assert_eq!(
                c.may_enter(Step::Initialize, DESIGNED),
                Err(why.to_string())
            );
        }
        assert_eq!(
            full.may_enter(Step::Mockups, Facts::default()),
            Err("validate the design system".to_string())
        );
        assert_eq!(full.reachable(Facts::default()), Step::Design);
    }

    #[test]
    fn going_moves_only_through_an_open_gate_and_never_back_to_the_repository() {
        let mut c = Creation::new("app", "me/app");
        assert!(c.go(Step::Brief, DESIGNED).is_err());
        assert_eq!(c.step, Step::Kind);
        c.kind = Some(Kind::Vertical);
        c.go(Step::Brief, DESIGNED).expect("open");
        assert_eq!(c.step, Step::Brief);
        assert!(c.go(Step::Repository, DESIGNED).is_err());
        c.go(Step::Kind, DESIGNED).expect("back is open");
        assert_eq!(c.step, Step::Kind);
    }

    #[test]
    fn an_edit_keeps_the_questions_sent_in_their_order_with_their_answers() {
        let mut c = Creation {
            questions: vec![question(1, ""), question(2, "b"), question(3, "")],
            ..Creation::default()
        };
        c.edit(Edit {
            questions: Some(vec![
                Answer {
                    id: 3,
                    answer: "three".to_string(),
                },
                Answer {
                    id: 1,
                    answer: "one".to_string(),
                },
                Answer {
                    id: 1,
                    answer: "twice".to_string(),
                },
                Answer {
                    id: 9,
                    answer: "unknown".to_string(),
                },
            ]),
            brief: Some("new".to_string()),
            ..Edit::default()
        });
        let kept: Vec<(u32, &str)> = c
            .questions
            .iter()
            .map(|q| (q.id, q.answer.as_str()))
            .collect();
        assert_eq!(kept, [(3, "three"), (1, "one")]);
        assert_eq!(c.brief, "new");
        assert_eq!(c.scope, "");
        assert_eq!(c.kind, None);
    }

    #[test]
    fn an_edit_without_questions_leaves_them_alone() {
        let mut c = Creation {
            questions: vec![question(1, "a")],
            ..Creation::default()
        };
        c.edit(Edit {
            kind: Some(Kind::Vertical),
            scope: Some("s".to_string()),
            ..Edit::default()
        });
        assert_eq!(c.questions, vec![question(1, "a")]);
        assert_eq!((c.kind, c.scope.as_str()), (Some(Kind::Vertical), "s"));
    }

    #[test]
    fn a_round_numbers_its_questions_after_the_highest_ever_given() {
        let mut c = Creation {
            questions: vec![question(4, "x")],
            rounds: 2,
            ..Creation::default()
        };
        c.add_round(Round {
            questions: vec![
                Asked {
                    title: "a".to_string(),
                    ..Asked::default()
                },
                Asked {
                    title: "b".to_string(),
                    ..Asked::default()
                },
            ],
            done: false,
        });
        let ids: Vec<(u32, u32)> = c.questions.iter().map(|q| (q.id, q.round)).collect();
        assert_eq!(ids, [(4, 0), (5, 3), (6, 3)]);
        assert_eq!(c.last_id, 6);
        // The last ones deleted, their numbers are not given again.
        c.questions.truncate(1);
        c.add_round(Round {
            questions: vec![Asked {
                title: "c".to_string(),
                ..Asked::default()
            }],
            done: false,
        });
        assert_eq!(c.questions.last().map(|q| q.id), Some(7));
        assert_eq!(c.rounds, 4);
        assert!(!c.grilled);
        c.add_round(Round {
            questions: vec![],
            done: true,
        });
        assert!(c.grilled);
        assert_eq!(c.questions.len(), 2);
    }

    #[test]
    fn regenerating_goes_back_to_the_brief_keeping_what_was_written() {
        let mut c = ready();
        c.grilled = true;
        c.step = Step::Review;
        c.regenerate();
        assert_eq!(c.step, Step::Brief);
        assert_eq!(
            (c.mocked.clone(), c.accepted, c.grilled),
            (None, false, false)
        );
        assert_eq!(c.regenerations, 1);
        assert_eq!(
            (c.brief.as_str(), c.scope.as_str(), c.questions.len()),
            ("a ledger", "## Goal", 1)
        );
    }

    #[test]
    fn a_round_is_read_from_a_fenced_answer_untitled_questions_dropped() {
        let answer = "Here it is:\n```json\n{\"questions\": [{\"title\": \" Who \", \"body\": \"b\", \
                      \"recommended\": \"r\"}, {\"title\": \"  \"}], \"done\": true}\n```";
        let round = parse_round(answer).expect("read");
        assert_eq!(round.questions.len(), 1);
        assert_eq!(
            round.questions.first().map(|q| q.title.as_str()),
            Some("Who")
        );
        assert!(!round.done, "a round that asks is not the last");
        assert!(
            parse_round("{\"questions\": [], \"done\": true}")
                .expect("read")
                .done
        );
        assert!(parse_round("no json here").is_err());
        assert!(parse_round("{not json}").is_err());
    }

    #[test]
    fn a_round_brings_at_most_round_max_questions() {
        let many: Vec<String> = (0..12)
            .map(|i| format!("{{\"title\": \"q{i}\"}}"))
            .collect();
        let round = parse_round(&format!("{{\"questions\": [{}]}}", many.join(","))).expect("read");
        assert_eq!(round.questions.len(), ROUND_MAX);
    }

    #[test]
    fn a_fence_is_taken_off_an_answer_and_a_bare_one_kept() {
        assert_eq!(unfenced("```markdown\n## Goal\nx\n```"), "## Goal\nx");
        assert_eq!(unfenced("  ## Goal  "), "## Goal");
        assert_eq!(unfenced("```"), "");
    }

    #[test]
    fn the_prompts_carry_the_brief_and_the_answered_decisions_only() {
        let mut c = ready();
        c.questions.push(question(2, ""));
        let grill = grill_prompt(&c);
        assert!(grill.contains("a ledger"));
        assert!(grill.contains("- **q1** — yes"));
        assert!(grill.contains("do not ask them again:\n\n- q2"));
        assert!(grill.contains("vertical architecture development factory"));
        let scope = scope_prompt(&c);
        assert!(scope.contains("start from it") && scope.contains("## Goal"));
        assert!(!scope.contains("q2"));
        c.scope = String::new();
        assert!(!scope_prompt(&c).contains("start from it"));
    }

    #[test]
    fn the_mockups_prompt_carries_the_rules_the_kit_and_the_scope() {
        let c = ready();
        let with = mockups_prompt(&c, "RULES", "Button: a button");
        assert!(with.starts_with("RULES"));
        assert!(with.contains("Button: a button"));
        assert!(with.ends_with("## Goal\n"));
        assert!(mockups_prompt(&c, "RULES", " ").contains("no generated component yet"));
    }

    #[test]
    fn the_roadmap_issue_carries_the_decisions_the_requirements_and_the_mock_up() {
        let mut c = ready();
        c.questions[0].body = "the client or the bank?".to_string();
        c.requirements.push(Requirement {
            id: 3,
            title: "Pay an invoice".to_string(),
            body: "The client pays from its list".to_string(),
            pages: vec!["pages/home.html".to_string()],
            components: vec!["InvoiceList".to_string()],
            human: true,
        });
        c.mockup_at = "main_agent@abcdef0123".to_string();
        c.design_at = "0123456789".to_string();
        let ctx = Context {
            pages: vec![PageRef {
                title: "Home".to_string(),
                path: "pages/home.html".to_string(),
                objective: "see the invoices".to_string(),
            }],
            components: vec![ComponentRef {
                name: "InvoiceList".to_string(),
                system: "billing".to_string(),
                description: "the invoices".to_string(),
                needs: vec!["billing.listInvoices".to_string()],
                actions: vec!["billing.payInvoice".to_string()],
            }],
            pins: vec![PinRef {
                kind: "request".to_string(),
                board: "mockup--main-agent--home".to_string(),
                target: "p1".to_string(),
                text: "the total above the lines".to_string(),
            }],
            design: DesignRef {
                validated_at: "2026-10-12".to_string(),
                tokens: 12,
                components: vec!["Button".to_string()],
            },
        };
        let (title, body) = roadmap_issue(&c, &ctx);
        assert_eq!(title, "Proof of concept — app");
        assert!(body.contains("- **q1** — yes\n  (asked: the client or the bank?)"));
        assert!(body.contains("- **R3 Pay an invoice** — The client pays from its list — on `mock/pages/home.html`, `InvoiceList`"));
        assert!(body.contains("- Home — `mock/pages/home.html` — see the invoices"));
        assert!(body.contains("- **InvoiceList** (billing) — the invoices — needs billing.listInvoices — actions billing.payInvoice"));
        assert!(body.contains("(`main_agent@abcdef0123`)"));
        assert!(body.contains("(`0123456789`), carried onto the integration branch. Validated 2026-10-12. 12 token(s); the kit: Button."));
        assert!(body.contains(
            "- *request* on `mockup--main-agent--home` at `p1`: the total above the lines"
        ));
        assert!(body.contains("mock/proof-of-concept.json"));
        c.design_refused = "protected".to_string();
        c.mockup_at = "mockup/proof-of-concept".to_string();
        let (_, refused) = roadmap_issue(&c, &ctx);
        assert!(refused.contains("**not carried onto the integration branch**: protected"));
        assert!(refused.contains(
            "on the branch `mockup/proof-of-concept` — the integration branch refused it"
        ));
        let (_, bare) = roadmap_issue(&Creation::new("app", "me/app"), &Context::default());
        assert!(!bare.contains("## Decisions") && !bare.contains("## Mock-ups"));
        assert!(!bare.contains("## Requirements") && !bare.contains("## Requests left"));
        let json = record_json(&c, &ctx).expect("written");
        let read: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(read["requirements"][0]["title"], "Pay an invoice");
        assert_eq!(read["components"][0]["needs"][0], "billing.listInvoices");
        assert_eq!(read["decisions"][0]["answer"], "yes");
        assert_eq!(read["mockup_at"], "mockup/proof-of-concept");
    }

    #[test]
    fn the_scope_opens_when_the_ai_had_nothing_to_ask_or_every_question_was_deleted() {
        let mut c = ready();
        c.questions.clear();
        assert_eq!(
            c.may_enter(Step::Scope, DESIGNED),
            Err("let the AI ask its questions".to_string())
        );
        c.grilled = true;
        assert!(c.may_enter(Step::Scope, DESIGNED).is_ok());
    }

    #[test]
    fn a_scope_proposed_goes_stale_once_the_decisions_move_and_an_edit_keeps_it() {
        let mut c = ready();
        assert!(!c.scope_stale(), "never proposed: nothing to be stale");
        c.propose("## Goal\nproposed".to_string());
        assert!(!c.scope_stale() && !c.scope_edited);
        c.edit(Edit {
            questions: Some(vec![Answer {
                id: 1,
                answer: "no".to_string(),
            }]),
            ..Edit::default()
        });
        assert!(c.scope_stale(), "a decision moved");
        c.edit(Edit {
            scope: Some("## Goal\nproposed".to_string()),
            ..Edit::default()
        });
        assert!(!c.scope_edited, "the same text is no edit");
        c.edit(Edit {
            scope: Some("## Goal\nmine".to_string()),
            ..Edit::default()
        });
        assert!(c.scope_edited);
        c.propose("## Goal\nagain".to_string());
        assert!(!c.scope_stale() && !c.scope_edited);
    }

    #[test]
    fn the_requirements_are_read_off_the_answer_tied_to_known_pages_and_the_humans_kept() {
        let pages = vec!["pages/home.html".to_string()];
        let components = vec!["InvoiceList".to_string()];
        let read = parse_requirements(
            "Here: {\"requirements\": [{\"title\": \"Pay\", \"body\": \"b\", \"pages\": [\"pages/home.html\", \"pages/nope.html\"], \"components\": [\"Ghost\", \"InvoiceList\"]}, {\"title\": \" \"}]}",
            &pages,
            &components,
        )
        .expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].pages, vec!["pages/home.html".to_string()]);
        assert_eq!(read[0].components, vec!["InvoiceList".to_string()]);
        assert!(parse_requirements("nothing", &pages, &components).is_err());
        assert!(parse_requirements("{\"requirements\": []}", &pages, &components).is_err());

        let mut c = ready();
        c.edit(Edit {
            requirements: Some(vec![Requirement {
                title: "Mine".to_string(),
                ..Requirement::default()
            }]),
            ..Edit::default()
        });
        assert_eq!((c.requirements[0].id, c.requirements[0].human), (1, true));
        c.add_requirements(read.clone());
        assert_eq!(c.requirements.len(), 2);
        assert_eq!((c.requirements[1].id, c.requirements[1].human), (2, false));
        c.add_requirements(read);
        assert_eq!(
            c.requirements.len(),
            2,
            "the AI's are replaced, the human's kept"
        );
        assert_eq!(c.requirements[1].id, 3, "never reused");
        c.edit(Edit {
            requirements: Some(vec![Requirement {
                id: 3,
                title: String::new(),
                body: String::new(),
                ..Requirement::default()
            }]),
            ..Edit::default()
        });
        assert!(c.requirements.is_empty(), "an empty one is deleted");
        let prompt = requirements_prompt(&ready(), &Context::default());
        assert!(prompt.contains("## Goal") && prompt.contains("a ledger"));
        assert!(!prompt.contains("The mock-up's pages") && !prompt.contains("The micro-frontends"));
    }

    #[test]
    fn the_requirements_prompt_lists_the_pages_the_micro_frontends_and_the_humans_own() {
        let mut c = ready();
        c.requirements.push(Requirement {
            id: 1,
            title: "Mine already".to_string(),
            human: true,
            ..Requirement::default()
        });
        c.requirements.push(Requirement {
            id: 2,
            title: "The AI's".to_string(),
            ..Requirement::default()
        });
        let ctx = Context {
            pages: vec![PageRef {
                title: "Home".to_string(),
                path: "pages/home.html".to_string(),
                objective: "see the invoices".to_string(),
            }],
            components: vec![ComponentRef {
                name: "InvoiceList".to_string(),
                system: "billing".to_string(),
                description: "the invoices".to_string(),
                needs: vec!["billing.listInvoices".to_string()],
                actions: vec!["billing.payInvoice".to_string()],
            }],
            ..Context::default()
        };
        let prompt = requirements_prompt(&c, &ctx);
        assert!(prompt.contains("- `pages/home.html` — Home — see the invoices"));
        assert!(prompt.contains(
            "- `InvoiceList` — billing — the invoices — billing.listInvoices — billing.payInvoice"
        ));
        assert!(prompt.contains("do not repeat them:\n\n- Mine already"));
        assert!(!prompt.contains("- The AI's"), "only the human's are kept");
        let bare = requirements_prompt(&ready(), &ctx);
        assert!(!bare.contains("do not repeat them"));
    }

    #[test]
    fn two_requirements_sent_with_the_same_id_keep_the_first() {
        let mut c = ready();
        c.edit(Edit {
            requirements: Some(vec![
                Requirement {
                    id: 5,
                    title: "first".to_string(),
                    ..Requirement::default()
                },
                Requirement {
                    id: 5,
                    title: "second".to_string(),
                    ..Requirement::default()
                },
                Requirement {
                    id: 6,
                    title: "other".to_string(),
                    ..Requirement::default()
                },
            ]),
            ..Edit::default()
        });
        let titles: Vec<&str> = c.requirements.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["first", "other"]);
    }

    #[test]
    fn the_mockups_wait_for_the_kit_once_the_design_is_validated() {
        let mut c = ready();
        c.mocked = None;
        let waiting = Facts {
            design_validated: true,
            kit_waiting: 2,
        };
        assert_eq!(
            c.may_enter(Step::Mockups, waiting),
            Err("let the kit's components be generated".to_string())
        );
        assert_eq!(c.reachable(waiting), Step::Design);
        assert!(c.may_enter(Step::Mockups, DESIGNED).is_ok());
    }

    #[test]
    fn regenerating_forgets_the_micro_frontends_too() {
        let mut c = ready();
        c.componentized = Some("three".to_string());
        c.regenerate();
        assert_eq!(c.componentized, None);
    }

    #[test]
    fn the_injected_micro_frontends_are_read_in_order_each_once() {
        let html = r#"<div data-component="InvoiceList" data-props='{}'></div>
            <section data-component='Balance'></section><div data-component="InvoiceList"></div>
            <p data-component=bare></p><i data-component=""></i><b data-component="Open"#;
        assert_eq!(injected(html), ["InvoiceList", "Balance"]);
        assert!(
            injected("<main>inline</main>").is_empty(),
            "expected nothing"
        );
    }

    #[test]
    fn a_mock_up_is_made_of_listed_micro_frontends_with_their_fragments() {
        let page = |path: &str, html: &str| (path.to_string(), html.to_string());
        let listed = [("List".to_string(), true), ("Form".to_string(), false)];
        assert!(
            not_componentized(
                &[page("pages/a.html", r#"<div data-component="List"></div>"#)],
                &listed
            )
            .is_empty(),
            "expected nothing"
        );
        assert_eq!(
            not_componentized(
                &[
                    page("pages/a.html", "<main>inline</main>"),
                    page(
                        "pages/b.html",
                        r#"<div data-component="Form"></div><div data-component="Ghost"></div>"#
                    ),
                ],
                &listed
            ),
            [
                "`pages/a.html` injects no micro-frontend",
                "`Form` has no fragment",
                "`pages/b.html` injects `Ghost`, which mock/pages.json does not list",
            ]
        );
        assert_eq!(not_componentized(&[], &listed), ["the mock-up has no page"]);
    }

    #[test]
    fn the_componentize_prompt_keeps_the_look_and_carries_the_kit_and_the_brief() {
        let c = ready();
        let text = componentize_prompt(&c, "RULES", "Button: a button");
        assert!(text.starts_with("RULES"));
        assert!(text.contains("without changing what the human sees"));
        assert!(text.contains("Button: a button"));
        assert!(text.contains("a ledger"));
        assert!(!componentize_prompt(&c, "RULES", "  ").contains("The kit's components"));
    }

    #[test]
    fn a_publication_says_whether_the_design_system_was_carried() {
        assert_eq!(
            design_published(""),
            "the design system is on its branch and the integration branch"
        );
        assert_eq!(
            design_published("moved"),
            "the design system is on its branch; not carried: moved"
        );
    }

    #[test]
    fn a_repository_name_takes_what_github_takes() {
        assert_eq!(
            repository_name(" my_app-2.0 "),
            Ok("my_app-2.0".to_string())
        );
        assert!(repository_name("").is_err());
        assert!(repository_name("..").is_err());
        assert!(repository_name("a b").is_err());
        assert!(repository_name(&"a".repeat(101)).is_err());
        assert!(repository_name(&"a".repeat(100)).is_ok());
    }

    #[test]
    fn a_record_reads_back_what_it_wrote() {
        let c = ready();
        let text = serde_json::to_string(&c).expect("written");
        assert_eq!(serde_json::from_str::<Creation>(&text).expect("read"), c);
        assert_eq!(
            serde_json::from_str::<Creation>("{}").expect("an empty record reads"),
            Creation::default()
        );
    }

    proptest! {
        #[test]
        fn reading_a_round_never_panics_and_never_exceeds_the_cap(answer in ".{0,400}") {
            if let Ok(round) = parse_round(&answer) {
                prop_assert!(round.questions.len() <= ROUND_MAX);
                prop_assert!(round.questions.iter().all(|q| !q.title.is_empty()));
            }
        }

        #[test]
        fn reading_the_injected_names_never_panics(html in ".{0,300}") {
            let names = injected(&html);
            prop_assert!(names.iter().all(|n| !n.is_empty() && html.contains(n.as_str())));
        }

        #[test]
        fn a_round_written_reads_back(titles in proptest::collection::vec("[a-zA-Z ]{1,20}", 0..12)) {
            let questions: Vec<String> = titles
                .iter()
                .map(|t| format!("{{\"title\": {}}}", serde_json::to_string(t).expect("json")))
                .collect();
            let round = parse_round(&format!("{{\"questions\": [{}]}}", questions.join(","))).expect("read");
            let kept: Vec<String> = titles.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).take(ROUND_MAX).collect();
            prop_assert_eq!(round.questions.iter().map(|q| q.title.clone()).collect::<Vec<_>>(), kept);
        }

        #[test]
        fn the_reachable_step_may_always_be_entered(
            kind in any::<bool>(),
            brief in any::<bool>(),
            answered in proptest::collection::vec(any::<bool>(), 0..3),
            scope in any::<bool>(),
            designed in any::<bool>(),
            mocked in any::<bool>(),
            accepted in any::<bool>(),
        ) {
            let c = Creation {
                kind: kind.then_some(Kind::Vertical),
                brief: if brief { "b".to_string() } else { String::new() },
                questions: answered.iter().enumerate().map(|(i, a)| question(u32::try_from(i).unwrap_or(0), if *a { "y" } else { "" })).collect(),
                scope: if scope { "s".to_string() } else { String::new() },
                mocked: mocked.then(String::new),
                accepted,
                ..Creation::default()
            };
            let facts = Facts { design_validated: designed, kit_waiting: 0 };
            let far = c.reachable(facts);
            prop_assert!(c.may_enter(far, facts).is_ok());
            if let Some(next) = Step::ALL.iter().find(|s| **s > far) {
                prop_assert!(c.may_enter(*next, facts).is_err());
            }
        }
    }
}
