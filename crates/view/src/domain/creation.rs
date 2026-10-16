//! A factory's creation, step by step: what the human wrote, what the AI
//! asked and proposed, where it stands, and the gates between the steps —
//! then the questions put to the AI and the reading of its answers.
//!
//! The record is what the register keeps beside a factory being created
//! (`Factories::creation`, opaque to `core`): its shape is decided here.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use harness_core::ports::shell::github::ISSUE_BODY_MAX;
use serde::{Deserialize, Serialize};

use super::audit::Audit;
use super::data_draft::{DataDraft, rules_print};
use super::ports_draft::PortsDraft;

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
    /// The rules the proof of concept must meet, tied to the mock-up's
    /// pages and micro-frontends, then validated; the mock-up regenerated
    /// or accepted. Once called the review.
    #[serde(alias = "review")]
    Rules,
    /// The data model the proof of concept needs: drafted by the AI off
    /// the rules, reviewed, validated.
    Data,
    /// The dependencies outside the product, each behind a port: drafted by
    /// the AI, the providers chosen by the human, validated; the proof of
    /// concept accepted.
    Ports,
    /// The factory initialized: the roadmap task opened, the watch started.
    Initialize,
}

impl Step {
    /// Every step, in order.
    pub const ALL: [Self; 11] = [
        Self::Repository,
        Self::Kind,
        Self::Brief,
        Self::Grilling,
        Self::Scope,
        Self::Design,
        Self::Mockups,
        Self::Rules,
        Self::Data,
        Self::Ports,
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
    /// The part of the product it decides; empty for the product as a
    /// whole — the page shows one card per topic.
    #[serde(default)]
    pub topic: String,
}

impl Question {
    /// The human answered it.
    #[must_use]
    pub fn answered(&self) -> bool {
        !self.answer.trim().is_empty()
    }
}

/// Where a factory's creation stands.
// Each bool is a fact of its own a gate reads (grilled, scope retouched,
// rules validated, accepted), kept as the register stores it: no state
// machine hides behind them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Creation {
    /// The factory's name.
    pub name: String,
    /// Its repository, `owner/name`.
    pub slug: String,
    /// The step the human is on.
    pub step: Step,
    /// The furthest step the human ever entered: a gate failing before it
    /// marks its step blocked.
    pub visited: Step,
    /// What kind of factory it is, once chosen.
    pub kind: Option<Kind>,
    /// What the application does, told at large: the brief is sorted out
    /// of it.
    pub description: String,
    /// What the application does, under the headings of [`BRIEF_PARTS`].
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
    /// The human validated the rules: each has its criterion and is
    /// carried by the mock-up. Any change to them takes it back.
    pub rules_validated: bool,
    /// What the rules were read off: the micro-frontends' cut as it was
    /// said then (`componentized`). Differs from it now: the cut moved.
    pub rules_for: String,
    /// What the architecture's guard found on the cut, once asked.
    pub audit: Option<Audit>,
    /// The changes to the rules the AI proposed after the mock-up moved,
    /// waiting for the human to accept or refuse each.
    pub proposals: Vec<Proposal>,
    /// The highest number a proposal was ever given.
    pub last_proposal: u32,
    /// The proposals of request pins accepted: each rule as it was and as
    /// it became, so the pin's Undo takes it back.
    pub applied: Vec<Applied>,
    /// The data model drafted for the proof of concept, once asked.
    pub data: Option<DataDraft>,
    /// The ports drafted for the proof of concept, once asked.
    pub ports: Option<PortsDraft>,
    /// The tasks opened for a human to provide each port's secrets.
    pub secrets_issues: Vec<String>,
    /// How many times the mock-ups were sent back to the brief.
    pub regenerations: u32,
    /// The mock-ups were drawn; what the session said of them.
    pub mocked: Option<String>,
    /// The scope the mock-ups were drawn from; differs from the scope now:
    /// it moved since.
    pub mocked_scope: String,
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
    /// The rules' task the initialization opened: the rules and their tests,
    /// before any development.
    pub rules_issue: String,
    /// The rules as the factory last received them — at Initialize, then at
    /// each change sent: what a change is measured against.
    pub rules_sent: Vec<Requirement>,
    /// Each change of the rules sent to the factory, in order.
    pub rules_changes: Vec<RulesSent>,
}

/// A change of the rules sent to the factory: the two tasks it opened.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulesSent {
    /// The rules' task: the changed rules' tests, ahead of the code.
    pub rules: String,
    /// The conformity task: the code brought to them.
    pub conformity: String,
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
    /// A step of a journey, or a business rule.
    pub family: Family,
    /// How to tell it is met: given…, when…, then….
    pub criterion: Criterion,
}

/// A change to the rules the AI proposes once the mock-up moved: a rule
/// rewritten, or a new one, for the human to accept or refuse.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Proposal {
    /// Its number among the proposals, never reused.
    pub key: u32,
    /// The rule it rewrites; 0 for a new rule.
    pub id: u32,
    /// The request pin that called for it; empty for none.
    pub pin: String,
    /// What moved in the mock-up and called for it.
    pub about: String,
    /// Why the rule changes, in a sentence.
    pub why: String,
    /// The rule, in a few words.
    pub title: String,
    /// What it asks for.
    pub body: String,
    /// The pages that carry it.
    pub pages: Vec<String>,
    /// The micro-frontends that carry it.
    pub components: Vec<String>,
    /// A step of a journey, or a business rule.
    pub family: Family,
    /// How to tell it is met.
    pub criterion: Criterion,
}

/// A request pin's proposal accepted: the rule as it was (`None`, it was
/// new) and as it became.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Applied {
    /// The pin whose proposal it was.
    pub pin: String,
    /// The rule as it was; `None`, the proposal added it.
    pub was: Option<Requirement>,
    /// The rule as the proposal made it.
    pub became: Requirement,
}

/// The rules of a creation as the mock-up's history keeps them: what an
/// undo of a rule's edit, acceptance or proposal writes back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulesState {
    /// The rules.
    pub requirements: Vec<Requirement>,
    /// The highest number a rule was ever given.
    pub last_requirement: u32,
    /// The rules were validated.
    pub rules_validated: bool,
    /// The proposals waiting.
    pub proposals: Vec<Proposal>,
    /// The highest number a proposal was ever given.
    pub last_proposal: u32,
    /// The proposals of request pins accepted.
    pub applied: Vec<Applied>,
}

/// What moved in the mock-up: a request drafted on a micro-frontend (or on
/// the page), then kept in its place or forked for the page it was asked on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RuleChange {
    /// The request pin it comes from; empty for none.
    pub pin: String,
    /// The micro-frontend changed; empty, the page itself.
    pub name: String,
    /// What it was saved as: itself, or its fork.
    pub saved: String,
    /// `replace`, `fork`, or empty while it is a draft.
    pub how: String,
    /// The page the change was asked on (`pages/…`).
    pub page: String,
    /// The requests, in the human's words.
    pub requests: Vec<String>,
}

impl RuleChange {
    /// The change in a sentence, as a proposal says what called for it.
    #[must_use]
    pub fn said(&self) -> String {
        match self.how.as_str() {
            "fork" => format!("{} forked as {} for {}", self.name, self.saved, self.page),
            "replace" => format!("{} changed on every page that shows it", self.name),
            _ if self.name.is_empty() => format!("the page {} drafted on a request", self.page),
            _ => format!("{} drafted on a request, on {}", self.name, self.page),
        }
    }
}

/// How a test tells a rule is met, in its three parts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Criterion {
    /// The situation before.
    pub given: String,
    /// What happens.
    pub when: String,
    /// What must then be true.
    pub then: String,
}

/// The words that open each part of a criterion written as one sentence,
/// in the languages a brief is written in.
const MARKERS: [(&str, &str, &str); 2] =
    [("given", "when", "then"), ("étant donné", "quand", "alors")];

impl Criterion {
    /// None of its parts is written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.given.trim().is_empty() && self.when.trim().is_empty() && self.then.trim().is_empty()
    }

    /// The parts not written yet, by name.
    #[must_use]
    pub fn missing(&self) -> Vec<&'static str> {
        [
            ("given", &self.given),
            ("when", &self.when),
            ("then", &self.then),
        ]
        .into_iter()
        .filter(|(_, part)| part.trim().is_empty())
        .map(|(name, _)| name)
        .collect()
    }

    /// The three parts in one line, for a prompt or a task.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "given {}; when {}; then {}",
            self.given.trim(),
            self.when.trim(),
            self.then.trim()
        )
    }

    /// The same, each part trimmed.
    fn trimmed(self) -> Self {
        Self {
            given: self.given.trim().to_string(),
            when: self.when.trim().to_string(),
            then: self.then.trim().to_string(),
        }
    }

    /// A criterion written as one sentence — "given …, when …, then …",
    /// or its French words — cut into its parts; one that does not read so
    /// is kept whole as its `given`, the rest left to write.
    #[must_use]
    pub fn from_sentence(text: &str) -> Self {
        let text = text.trim();
        let lower = text.to_lowercase();
        // Lowercase must keep every byte where it was, or the cuts drift.
        if lower.len() == text.len() {
            for (given, when, then) in MARKERS {
                let Some(start) = lower.strip_prefix(given).map(|_| given.len()) else {
                    continue;
                };
                let when_at = lower
                    .get(start..)
                    .and_then(|rest| rest.find(&format!(", {when} ")));
                let Some(when_at) = when_at.map(|i| i + start) else {
                    continue;
                };
                let after_when = when_at + 2 + when.len();
                let then_at = lower
                    .get(after_when..)
                    .and_then(|rest| rest.find(&format!(", {then} ")));
                let Some(then_at) = then_at.map(|i| i + after_when) else {
                    continue;
                };
                let part = |from: usize, to: usize| {
                    text.get(from..to).unwrap_or_default().trim().to_string()
                };
                return Self {
                    given: part(start, when_at),
                    when: part(after_when, then_at),
                    then: part(then_at + 2 + then.len(), text.len()),
                };
            }
        }
        Self {
            given: text.to_string(),
            ..Self::default()
        }
    }
}

/// A criterion as it was kept or answered: three parts, or one sentence
/// (the records before the parts, an AI that writes it so).
#[derive(Deserialize)]
#[serde(untagged)]
enum Written {
    Sentence(String),
    Parts {
        #[serde(default)]
        given: String,
        #[serde(default)]
        when: String,
        #[serde(default)]
        then: String,
    },
}

impl<'de> Deserialize<'de> for Criterion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Written::deserialize(deserializer)? {
            Written::Sentence(text) => Self::from_sentence(&text),
            Written::Parts { given, when, then } => Self { given, when, then },
        })
    }
}

/// What a rule is about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// A step of a user's journey: what the user does and sees.
    #[default]
    Journey,
    /// A business rule: an invariant, a computation, what the application
    /// keeps — the data step turns these into invariants.
    Business,
}

impl Requirement {
    /// Carried by at least one page or micro-frontend of the mock-up.
    #[must_use]
    pub const fn covered(&self) -> bool {
        !self.pages.is_empty() || !self.components.is_empty()
    }

    /// The same rule, `human` and `id` aside.
    fn same(&self, other: &Self) -> bool {
        self.title == other.title
            && self.body == other.body
            && self.pages == other.pages
            && self.components == other.components
            && self.family == other.family
            && self.criterion == other.criterion
    }
}

/// What a gate reads that the record does not hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// The design system is validated, on its branch.
    pub design_validated: bool,
    /// The kit's components ticked and not generated yet, or generating.
    pub kit_waiting: usize,
}

/// How a step's crumb reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    /// Made, and made from what is upstream now.
    Done,
    /// Made from something upstream that moved since: to review.
    Stale,
    /// The gate the human cannot pass any more to go back where they were.
    Blocked,
    /// The step to do now.
    Open,
    /// Not reachable yet.
    Locked,
}

/// The steps each made from the one before it: a step to review makes
/// every later one of them to review too.
const CHAIN: [Step; 5] = [
    Step::Scope,
    Step::Mockups,
    Step::Rules,
    Step::Data,
    Step::Ports,
];

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
        Step::Design => {
            if c.scope.trim().is_empty() {
                Some("settle the proof of concept's scope")
            } else if crate::domain::drawing::screens(&c.scope).is_empty() {
                Some("list at least one screen under Screens")
            } else {
                None
            }
        }
        Step::Mockups => {
            if !facts.design_validated {
                Some("validate the design system")
            } else if facts.kit_waiting > 0 {
                Some("let the kit's components be generated")
            } else {
                None
            }
        }
        Step::Rules => c.mocked.is_none().then_some("draw the mock-ups"),
        Step::Data => (!c.rules_validated).then_some("validate the rules"),
        // Accepted, it went through every step: a later move shows as stale,
        // never as a lock.
        Step::Ports => (!c.accepted && !c.data_validated()).then_some("validate the data model"),
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

    /// Each step's mark, in order. Before the furthest step reachable, a
    /// step is done — or stale when what it was made from moved, or when a
    /// step it follows in [`CHAIN`] is stale; the furthest one is open, or
    /// blocked when the human had gone beyond it; the rest are locked.
    #[must_use]
    pub fn marks(&self, facts: Facts) -> Vec<(Step, Mark)> {
        let far = self.reachable(facts);
        let visited = self.visited.max(self.step);
        let mut upstream_stale = false;
        Step::ALL
            .iter()
            .map(|&step| {
                let own = match step {
                    Step::Scope => self.scope_stale(),
                    Step::Mockups => self.mockup_stale(),
                    Step::Rules => self.rules_stale(),
                    Step::Data => self.data_stale(),
                    Step::Ports => self.ports_stale(),
                    _ => false,
                };
                let chained = CHAIN.contains(&step);
                let stale = own || (chained && upstream_stale);
                if chained {
                    upstream_stale = stale;
                }
                let mark = if step < far || (step == Step::Initialize && !self.issue.is_empty()) {
                    if stale { Mark::Stale } else { Mark::Done }
                } else if step == far {
                    if visited > far {
                        Mark::Blocked
                    } else {
                        Mark::Open
                    }
                } else {
                    Mark::Locked
                };
                (step, mark)
            })
            .collect()
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
        self.visited = self.visited.max(step);
        Ok(())
    }

    /// The human's edits, applied: the fields they sent, the questions they
    /// kept — in that order, the others deleted — with their answers.
    pub fn edit(&mut self, edit: Edit) {
        if let Some(kind) = edit.kind {
            self.kind = Some(kind);
        }
        if let Some(description) = edit.description {
            self.description = description;
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
            let moved = kept.len() != self.requirements.len()
                || kept
                    .iter()
                    .zip(&self.requirements)
                    .any(|(a, b)| a.id != b.id || !a.same(b));
            if moved {
                self.rules_validated = false;
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

    /// The mock-ups were drawn from a scope that moved since.
    #[must_use]
    pub fn mockup_stale(&self) -> bool {
        let from = self.mocked_scope.trim();
        self.mocked.is_some() && !from.is_empty() && from != self.scope.trim()
    }

    /// The mock-ups drawn from the scope as it is now: a new drawing is not
    /// made of micro-frontends yet.
    pub fn drawn(&mut self, said: String) {
        self.mocked = Some(said);
        self.mocked_scope.clone_from(&self.scope);
        self.componentized = None;
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
        self.rules_validated = false;
        self.rules_for = self.componentized.clone().unwrap_or_default();
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
                family: r.family,
                criterion: r.criterion,
            });
        }
    }

    /// One rule read again by the AI off its supports: its number kept, the
    /// rest as read. Unknown: nothing changes.
    ///
    /// # Errors
    ///
    /// No rule has that number.
    pub fn reread(&mut self, id: u32, read: Read) -> Result<(), String> {
        let rule = self
            .requirements
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("no rule R{id}"))?;
        rule.title = read.title;
        rule.body = read.body;
        rule.pages = read.pages;
        rule.components = read.components;
        rule.family = read.family;
        rule.criterion = read.criterion;
        self.rules_validated = false;
        Ok(())
    }

    /// The rules as the mock-up's history keeps them.
    #[must_use]
    pub fn rules_state(&self) -> RulesState {
        RulesState {
            requirements: self.requirements.clone(),
            last_requirement: self.last_requirement,
            rules_validated: self.rules_validated,
            proposals: self.proposals.clone(),
            last_proposal: self.last_proposal,
            applied: self.applied.clone(),
        }
    }

    /// The rules as the history kept them, written back.
    pub fn set_rules_state(&mut self, rules: RulesState) {
        self.requirements = rules.requirements;
        self.last_requirement = rules.last_requirement;
        self.rules_validated = rules.rules_validated;
        self.proposals = rules.proposals;
        self.last_proposal = rules.last_proposal;
        self.applied = rules.applied;
    }

    /// The changes the AI proposed, kept for the human: numbered, a
    /// proposal about a rule replacing an older one about it, one about a
    /// rule that is gone left out.
    pub fn propose_rules(&mut self, about: &str, read: Vec<Proposal>) {
        for mut p in read {
            if p.title.trim().is_empty() {
                continue;
            }
            if p.id != 0 && !self.requirements.iter().any(|r| r.id == p.id) {
                continue;
            }
            if p.id != 0 {
                self.proposals.retain(|q| q.id != p.id);
            }
            self.last_proposal = self.last_proposal.saturating_add(1);
            p.key = self.last_proposal;
            p.about = about.to_string();
            self.proposals.push(p);
        }
    }

    /// A request pin's proposals kept: [`Self::propose_rules`], each tied
    /// to the pin so its validation accepts them and its Undo takes them
    /// back; its proposals still waiting from before go.
    pub fn propose_for(&mut self, change: &RuleChange, read: Vec<Proposal>) {
        // Proposed again, a pin's waiting proposals give way to the new ones.
        if !change.pin.is_empty() {
            self.proposals.retain(|p| p.pin != change.pin);
        }
        let tagged = read
            .into_iter()
            .map(|mut p| {
                p.pin.clone_from(&change.pin);
                p
            })
            .collect();
        self.propose_rules(&change.said(), tagged);
    }

    /// Every proposal of these pins still waiting, accepted: the pins'
    /// change was validated. Answers what could not be.
    pub fn accept_pins(&mut self, pins: &[String]) -> Vec<String> {
        let mut said = Vec::new();
        // A proposal leaves the list as it is settled, accepted or not.
        while let Some(key) = self
            .proposals
            .iter()
            .find(|p| pins.contains(&p.pin))
            .map(|p| p.key)
        {
            if let Err(why) = self.settle_proposal(key, true) {
                said.push(why);
            }
        }
        said
    }

    /// A micro-frontend forked for a page: the rules these pins proposed or
    /// had accepted that it carried on that page are carried by the fork.
    pub fn retie_fork(&mut self, pins: &[String], page: &str, from: &str, to: &str) {
        let swap = |components: &mut Vec<String>, pages: &[String]| {
            if pages.iter().any(|p| p == page) {
                for c in components.iter_mut().filter(|c| *c == from) {
                    *c = to.to_string();
                }
            }
        };
        for p in self.proposals.iter_mut().filter(|p| pins.contains(&p.pin)) {
            swap(&mut p.components, &p.pages);
        }
        for a in self.applied.iter_mut().filter(|a| pins.contains(&a.pin)) {
            let Some(rule) = self.requirements.iter_mut().find(|r| r.id == a.became.id) else {
                continue;
            };
            if *rule != a.became {
                continue;
            }
            let pages = rule.pages.clone();
            swap(&mut rule.components, &pages);
            a.became = rule.clone();
        }
    }

    /// What these pins did to the rules, taken back: their waiting
    /// proposals gone, the rules they had accepted as they were — unless
    /// the human retouched one since, which stays. Answers those kept.
    pub fn undo_pins(&mut self, pins: &[String]) -> Vec<String> {
        self.proposals.retain(|p| !pins.contains(&p.pin));
        let (mine, others): (Vec<Applied>, Vec<Applied>) = std::mem::take(&mut self.applied)
            .into_iter()
            .partition(|a| pins.contains(&a.pin));
        self.applied = others;
        let mut kept = Vec::new();
        for a in mine.into_iter().rev() {
            let Some(at) = self.requirements.iter().position(|r| r.id == a.became.id) else {
                continue;
            };
            if self.requirements.get(at) != Some(&a.became) {
                kept.push(format!("R{} retouched since: kept", a.became.id));
                continue;
            }
            match a.was {
                Some(rule) => {
                    if let Some(slot) = self.requirements.get_mut(at) {
                        *slot = rule;
                    }
                }
                None => {
                    self.requirements.remove(at);
                }
            }
            self.rules_validated = false;
        }
        kept
    }

    /// One proposal settled: accepted, its rule rewritten or added as the
    /// human's; refused, gone.
    ///
    /// # Errors
    ///
    /// No proposal has that number, or the rule it rewrites is gone.
    pub fn settle_proposal(&mut self, key: u32, accept: bool) -> Result<(), String> {
        let at = self
            .proposals
            .iter()
            .position(|p| p.key == key)
            .ok_or_else(|| format!("no proposal {key}"))?;
        let p = self.proposals.remove(at);
        if !accept {
            return Ok(());
        }
        let mut rule = Requirement {
            id: p.id,
            title: p.title,
            body: p.body,
            pages: p.pages,
            components: p.components,
            human: true,
            family: p.family,
            criterion: p.criterion,
        };
        let was = if rule.id == 0 {
            self.last_requirement = self.last_requirement.saturating_add(1);
            rule.id = self.last_requirement;
            self.requirements.push(rule.clone());
            None
        } else {
            let kept = self
                .requirements
                .iter_mut()
                .find(|r| r.id == rule.id)
                .ok_or_else(|| format!("R{} is gone", rule.id))?;
            Some(std::mem::replace(kept, rule.clone()))
        };
        if !p.pin.is_empty() {
            self.applied.push(Applied {
                pin: p.pin,
                was,
                became: rule,
            });
        }
        self.rules_validated = false;
        Ok(())
    }

    /// What keeps the rules from being validated, in words: a rule with no
    /// criterion, a rule nothing in the mock-up carries. Empty: they may.
    #[must_use]
    pub fn rule_gaps(&self) -> Vec<String> {
        if self.requirements.is_empty() {
            return vec!["no rule yet".to_string()];
        }
        let mut gaps = Vec::new();
        if let Some(audit) = &self.audit {
            for f in audit.blocking() {
                gaps.push(format!("architecture: {}", f.why.trim()));
            }
        }
        for r in &self.requirements {
            if r.criterion.is_empty() {
                gaps.push(format!("R{} has no criterion", r.id));
            } else if !r.criterion.missing().is_empty() {
                gaps.push(format!(
                    "R{}'s criterion has no {}",
                    r.id,
                    r.criterion.missing().join(" and no ")
                ));
            }
            if !r.covered() {
                gaps.push(format!(
                    "R{} is carried by no page and no micro-frontend",
                    r.id
                ));
            }
        }
        gaps
    }

    /// The rules' ties to what the mock-up has not — a page or a
    /// micro-frontend gone with a discard or a new drawing — in words.
    #[must_use]
    pub fn dangling(&self, pages: &[String], components: &[String]) -> Vec<String> {
        let mut found = Vec::new();
        for r in &self.requirements {
            for page in r.pages.iter().filter(|p| !pages.contains(p)) {
                found.push(format!(
                    "R{} points at {page}, which the mock-up has not",
                    r.id
                ));
            }
            for name in r.components.iter().filter(|c| !components.contains(c)) {
                found.push(format!(
                    "R{} is tied to {name}, which the mock-up has not",
                    r.id
                ));
            }
        }
        found
    }

    /// The rules validated, when nothing keeps them from it.
    ///
    /// # Errors
    ///
    /// What keeps them, first gap first.
    pub fn validate_rules(&mut self) -> Result<(), String> {
        let gaps = self.rule_gaps();
        if !gaps.is_empty() {
            return Err(gaps.join("; "));
        }
        self.rules_validated = true;
        Ok(())
    }

    /// The data model was drafted from rules that moved since.
    #[must_use]
    pub fn data_stale(&self) -> bool {
        self.data
            .as_ref()
            .is_some_and(|d| d.from != rules_print(self))
    }

    /// What keeps the data model from validation; one gap when none is
    /// drafted yet.
    #[must_use]
    pub fn data_gaps(&self) -> Vec<String> {
        self.data
            .as_ref()
            .map_or_else(|| vec!["no data model yet".to_string()], |d| d.gaps(self))
    }

    /// The data model validated, when nothing keeps it from it.
    ///
    /// # Errors
    ///
    /// What keeps it, every gap.
    pub fn validate_data(&mut self) -> Result<(), String> {
        let gaps = self.data_gaps();
        if !gaps.is_empty() {
            return Err(gaps.join("; "));
        }
        if let Some(data) = self.data.as_mut() {
            data.validated = true;
        }
        Ok(())
    }

    /// The data model validated and drafted from the rules as they are.
    #[must_use]
    pub fn data_validated(&self) -> bool {
        self.data.as_ref().is_some_and(|d| d.validated) && !self.data_stale()
    }

    /// The ports were drafted from rules or writes that moved since.
    #[must_use]
    pub fn ports_stale(&self) -> bool {
        self.ports
            .as_ref()
            .is_some_and(|p| p.from != super::ports_draft::ports_print(self))
    }

    /// The ports are validated and still drafted from what is.
    #[must_use]
    pub fn ports_validated(&self) -> bool {
        self.ports.as_ref().is_some_and(|p| p.validated) && !self.ports_stale()
    }

    /// The ports validated, when nothing keeps them: every port whole.
    ///
    /// # Errors
    ///
    /// No draft yet, or its gaps, in words.
    pub fn validate_ports(&mut self, ctx: &Context) -> Result<(), String> {
        let gaps = self.ports_gaps(ctx);
        if !gaps.is_empty() {
            return Err(gaps.join("; "));
        }
        if let Some(ports) = self.ports.as_mut() {
            ports.validated = true;
        }
        Ok(())
    }

    /// What keeps the ports from validation, in words.
    #[must_use]
    pub fn ports_gaps(&self, ctx: &Context) -> Vec<String> {
        let Some(ports) = &self.ports else {
            return vec!["no port drafted yet".to_string()];
        };
        let reads = ctx
            .components
            .iter()
            .flat_map(|m| m.needs.iter().cloned())
            .collect();
        let writes = ctx
            .components
            .iter()
            .flat_map(|m| m.actions.iter().cloned())
            .collect();
        ports.gaps(&reads, &writes)
    }

    /// The architecture's guard looked at a cut that moved since.
    #[must_use]
    pub fn audit_stale(&self) -> bool {
        self.audit.as_ref().is_some_and(|a| {
            self.componentized
                .as_deref()
                .is_some_and(|now| now != a.from)
        })
    }

    /// The rules were read off a cut into micro-frontends that moved since.
    /// Rules read before the cut was noted (`rules_for` empty) are not.
    #[must_use]
    pub fn rules_stale(&self) -> bool {
        !self.requirements.is_empty()
            && !self.rules_for.is_empty()
            && self
                .componentized
                .as_deref()
                .is_some_and(|now| now != self.rules_for)
    }

    /// The human's rules tied again to the mock-up as it stands: a rule the
    /// AI tied keeps those ties alone; the others, and every rule the AI
    /// read, are left as they are.
    pub fn retie(&mut self, ties: &[Tie]) {
        for tie in ties {
            if let Some(rule) = self
                .requirements
                .iter_mut()
                .find(|r| r.human && r.id == tie.id)
            {
                rule.pages.clone_from(&tie.pages);
                rule.components.clone_from(&tie.components);
            }
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
            let topic = self.known_topic(&asked.topic);
            self.questions.push(Question {
                id: next,
                title: asked.title,
                body: asked.body,
                recommended: asked.recommended,
                answer: String::new(),
                round: self.rounds,
                topic,
            });
        }
        self.last_id = next;
        self.grilled = round.done;
    }

    /// The topics the questions carry, each once, in the order first
    /// asked; the product as a whole (no topic) is not one.
    #[must_use]
    pub fn topics(&self) -> Vec<String> {
        let mut topics: Vec<String> = Vec::new();
        for q in &self.questions {
            if !q.topic.is_empty() && !topics.contains(&q.topic) {
                topics.push(q.topic.clone());
            }
        }
        topics
    }

    /// A topic as the AI wrote it, spelled as an earlier question already
    /// does when only the case differs: one card, not two.
    fn known_topic(&self, topic: &str) -> String {
        self.questions
            .iter()
            .map(|q| &q.topic)
            .find(|known| known.to_lowercase() == topic.to_lowercase())
            .map_or_else(|| topic.to_string(), Clone::clone)
    }

    /// Back to the brief to draw the mock-ups again: everything written is
    /// kept, the mock-ups are not.
    pub fn regenerate(&mut self) {
        self.regenerations = self.regenerations.saturating_add(1);
        self.mocked = None;
        self.mocked_scope.clear();
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
    /// What the application does, told at large.
    pub description: Option<String>,
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
    /// The part of the product it decides; empty for the whole product.
    pub topic: String,
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
    /// A step of a journey, or a business rule.
    pub family: Family,
    /// How to tell it is met.
    pub criterion: Criterion,
}

/// A human's rule tied again to the mock-up as it stands, as the AI answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Tie {
    /// The rule's number.
    pub id: u32,
    /// The pages that carry it now.
    pub pages: Vec<String>,
    /// The micro-frontends that carry it now.
    pub components: Vec<String>,
}

/// The requirements, as the AI answers them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Requirements {
    /// The requirements read.
    pub requirements: Vec<Read>,
    /// The human's rules, tied again to the mock-up as it stands.
    pub ties: Vec<Tie>,
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
        r.criterion = r.criterion.trimmed();
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

/// The human's rules tied again, out of the AI's answer: the pages and
/// micro-frontends the mock-up has, a rule with none left out. An answer
/// with none is no tie, not an error: the rules keep theirs.
#[must_use]
pub fn parse_ties(answer: &str, pages: &[String], components: &[String]) -> Vec<Tie> {
    let Some(read) = object_in(answer).and_then(|o| serde_json::from_str::<Requirements>(o).ok())
    else {
        return Vec::new();
    };
    read.ties
        .into_iter()
        .map(|mut t| {
            t.pages.retain(|p| pages.iter().any(|known| known == p));
            t.components
                .retain(|c| components.iter().any(|known| known == c));
            t
        })
        .filter(|t| !t.pages.is_empty() || !t.components.is_empty())
        .collect()
}

/// The proposals, as the AI answers them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct Proposals {
    proposals: Vec<Proposal>,
}

/// The changes to the rules out of the AI's answer: those with a title,
/// their pages and micro-frontends kept to the ones the mock-up has. None
/// is an answer: the change leaves the rules as they are.
///
/// # Errors
///
/// The answer carries no proposals.
pub fn parse_proposals(
    answer: &str,
    pages: &[String],
    components: &[String],
) -> Result<Vec<Proposal>, String> {
    let object = object_in(answer).ok_or("the AI answered no proposals")?;
    let read: Proposals = serde_json::from_str(object)
        .map_err(|e| format!("the AI's proposals are not readable: {e}"))?;
    Ok(read
        .proposals
        .into_iter()
        .filter(|p| !p.title.trim().is_empty())
        .map(|mut p| {
            p.pages.retain(|x| pages.iter().any(|known| known == x));
            p.components
                .retain(|x| components.iter().any(|known| known == x));
            p
        })
        .collect())
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
        q.topic = q.topic.trim().to_string();
    }
    // A round that asks something is not the last, whatever it says.
    round.done = round.done && round.questions.is_empty();
    Ok(round)
}

/// One part of the brief: the question it answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BriefPart {
    /// Its key, in the AI's answer and the page's fields.
    pub id: &'static str,
    /// The question, and the brief's heading.
    pub title: &'static str,
    /// What an answer covers.
    pub hint: &'static str,
}

/// The brief's parts, in order: the page asks them, the AI sorts the
/// description into them.
pub const BRIEF_PARTS: [BriefPart; 4] = [
    BriefPart {
        id: "what",
        title: "What is the application, and what problem does it solve?",
        hint: "The idea in a few sentences: the need, and what goes wrong today without it.",
    },
    BriefPart {
        id: "who",
        title: "Who uses it?",
        hint: "The kinds of users, their roles, what each may see or do.",
    },
    BriefPart {
        id: "journeys",
        title: "What do they do with it?",
        hint: "The main journeys, start to finish: what a user comes to do, and what they get out of it.",
    },
    BriefPart {
        id: "data",
        title: "What does it keep, and what does it connect to?",
        hint: "The information it stores, the tools or services it talks to, the constraints (devices, languages, offline…).",
    },
];

/// The question that sorts the human's description into the brief's parts.
#[must_use]
pub fn decompose_prompt(description: &str) -> String {
    let mut text = String::from(
        "A human described an application they want a development factory to build. Sort what \
         they wrote into the parts of a brief below. Keep their words, their details and their \
         examples; reword only to make a part read on its own; invent nothing — a part the \
         description says nothing about is an empty string. Answer with one JSON object and \
         nothing else, one key per part, each value Markdown text: {",
    );
    let keys: Vec<String> = BRIEF_PARTS
        .iter()
        .map(|p| format!("\"{}\": \"…\"", p.id))
        .collect();
    text.push_str(&keys.join(", "));
    text.push_str("}. Write in the language of the description.\n\nThe parts:\n\n");
    for p in &BRIEF_PARTS {
        let _ = writeln!(text, "- `{}` — {} {}", p.id, p.title, p.hint);
    }
    let _ = write!(text, "\nThe description:\n\n{}\n", description.trim());
    text
}

/// The brief out of the AI's answer: each part under its heading, the empty
/// ones left out — the text the page splits back into its fields.
///
/// # Errors
///
/// The answer carries no object, or every part is empty.
pub fn parse_brief(answer: &str) -> Result<String, String> {
    let object = object_in(answer).ok_or("the AI answered no brief")?;
    let parts: HashMap<String, String> =
        serde_json::from_str(object).map_err(|e| format!("the AI's brief is not readable: {e}"))?;
    let brief = BRIEF_PARTS
        .iter()
        .filter_map(|p| {
            let text = parts.get(p.id).map_or("", |t| t.trim());
            (!text.is_empty()).then(|| format!("### {}\n\n{text}", p.title))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if brief.is_empty() {
        return Err("the AI found nothing to sort in the description".to_string());
    }
    Ok(brief)
}

/// One section of the proof of concept's scope: its own field on the page,
/// its own key in the AI's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ScopeSection {
    /// Its key, in the AI's answer and the page's fields.
    pub id: &'static str,
    /// Its heading in the scope, `## <title>`: fixed, the text is split on it.
    pub title: &'static str,
    /// What it holds, in one line.
    pub hint: &'static str,
    /// What it is for and who reads it: the page's `?`.
    pub help: &'static str,
    /// A scope without it cannot be drawn.
    pub required: bool,
}

/// The id of the section the drawing reads its screens from.
pub const SCREENS: &str = "screens";

/// The id of the free section, where any other heading goes.
pub const NOTES: &str = "notes";

/// The scope's sections, in order: the AI answers them, the page shows
/// them, the scope keeps them under their headings.
pub const SCOPE_SECTIONS: [ScopeSection; 9] = [
    ScopeSection {
        id: "goal",
        title: "Goal",
        hint: "One paragraph: what the proof of concept proves.",
        help: "The one thing the proof of concept must show works. Every later step reads it: the mock-up, the rules, the data and the ports.",
        required: false,
    },
    ScopeSection {
        id: "users",
        title: "Users & roles",
        hint: "Who uses it, and what each kind of user may see or do.",
        help: "The kinds of users and their rights. The rules step reads it for who may do what, the ports step for the sign-in it needs.",
        required: false,
    },
    ScopeSection {
        id: SCREENS,
        title: "Screens",
        hint: "One bullet per screen: its name in bold, then what the user sees and does there.",
        help: "Read by the factory itself: one page of the mock-up per bullet, named after the bold name. The first screen is drawn first, alone — the reference whose look every other page follows. At least one is needed to draw.",
        required: true,
    },
    ScopeSection {
        id: "journeys",
        title: "Journeys",
        hint: "The user journeys it carries, step by step.",
        help: "What a user comes to do, start to finish, across the screens. The rules step turns each step into a journey rule with its test.",
        required: false,
    },
    ScopeSection {
        id: "data",
        title: "Data",
        hint: "The things it keeps, with their main fields.",
        help: "The information the proof of concept stores. The data step drafts the data model from it.",
        required: false,
    },
    ScopeSection {
        id: "integrations",
        title: "Integrations",
        hint: "The services outside it talks to: payment, mail, an API, a sign-in.",
        help: "Every dependency outside the product. The ports step lists them, a provider picked for each.",
        required: false,
    },
    ScopeSection {
        id: "left_out",
        title: "Left out",
        hint: "What waits for later, and why.",
        help: "What the proof of concept does not do. It keeps the drawing and the rules from growing past the scope.",
        required: false,
    },
    ScopeSection {
        id: "proof",
        title: "Proof",
        hint: "How the human will know it works.",
        help: "What you will check to call the proof of concept a success. The rules read it for their criteria.",
        required: false,
    },
    ScopeSection {
        id: NOTES,
        title: "Notes",
        hint: "Anything else, in sub-sections: `### Constraints`, `### Languages`…",
        help: "Free: whatever the other sections do not hold — constraints, devices, languages, tone. Start each sub-section with `### Title`. Every later step reads it.",
        required: false,
    },
];

/// The scope out of the AI's answer: each section under its heading, in
/// order, the empty ones left out — the text the page splits back into its
/// fields.
///
/// # Errors
///
/// The answer carries no object, or no screen.
pub fn parse_scope(answer: &str) -> Result<String, String> {
    let object = object_in(answer).ok_or("the AI answered no scope")?;
    let sections: HashMap<String, serde_json::Value> =
        serde_json::from_str(object).map_err(|e| format!("the AI's scope is not readable: {e}"))?;
    let mut said = Vec::new();
    for s in &SCOPE_SECTIONS {
        // A key the scope does not know is left; one it knows is text.
        let text = match sections.get(s.id) {
            None | Some(serde_json::Value::Null) => "",
            Some(serde_json::Value::String(text)) => text.trim(),
            Some(_) => {
                return Err(format!(
                    "the AI's scope is not readable: `{}` is not text",
                    s.id
                ));
            }
        };
        if !text.is_empty() {
            said.push(format!("## {}\n\n{text}", s.title));
        }
    }
    let scope = said.join("\n\n");
    if crate::domain::drawing::screens(&scope).is_empty() {
        return Err("the AI's scope lists no screen".to_string());
    }
    Ok(scope)
}

/// What the human wants built: the kind, the brief, the decisions taken.
pub fn product(c: &Creation) -> String {
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
            let _ = writeln!(
                text,
                "- {}**{}** — {}",
                in_topic(q),
                q.title,
                q.answer.trim()
            );
        }
    }
    text
}

/// A question's topic as a prefix, `[Combat] `; nothing for the product as
/// a whole.
fn in_topic(q: &Question) -> String {
    if q.topic.is_empty() {
        String::new()
    } else {
        format!("[{}] ", q.topic)
    }
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
         File each question under a topic: the part of the product it decides (the human sees \
         one card per topic), a short name of a few words. A decision about the product as a \
         whole takes the empty topic, \"\", the general card. Reuse a topic already in use, \
         spelled the same, rather than a near-synonym; open a new topic when a round reaches a \
         part of the product no topic covers yet. One decision is asked once, under one \
         topic: never the same decision, or one overlapping it, under two topics — the topics \
         sort the questions, they do not split the interview.\n\n\
         Answer with one JSON object and nothing else: \
         {\"questions\": [{\"topic\": \"the part of the product, or empty\", \"title\": \"the \
         decision in a few words\", \"body\": \"the question, the choices it offers\", \
         \"recommended\": \"your recommended answer\"}], \"done\": false}. \
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
            let _ = writeln!(text, "- {}{}", in_topic(q), q.title);
        }
    }
    let topics = c.topics();
    if !topics.is_empty() {
        let _ = write!(text, "\nTopics in use: {}.\n", topics.join(", "));
    }
    text
}

/// The question that proposes the proof of concept's scope, one key per
/// section of [`SCOPE_SECTIONS`].
#[must_use]
pub fn scope_prompt(c: &Creation) -> String {
    let mut text = String::from(
        "You scope the proof of concept of an application a development factory will build. \
         The proof of concept is the smallest thing that proves the application's value to the \
         human: a few screens, one or two user journeys end to end, the data they need — nothing \
         that can wait. Answer with one JSON object and nothing else, one key per section below, \
         each value Markdown text without its heading: {",
    );
    let keys: Vec<String> = SCOPE_SECTIONS
        .iter()
        .map(|s| format!("\"{}\": \"…\"", s.id))
        .collect();
    text.push_str(&keys.join(", "));
    text.push_str(
        "}. A section the proof of concept has nothing for is an empty string, but `screens` \
         never is. Write in the language of the human's brief.\n\nThe sections:\n\n",
    );
    for s in &SCOPE_SECTIONS {
        let _ = writeln!(text, "- `{}` — {}: {}", s.id, s.title, s.hint);
    }
    text.push('\n');
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

/// What a mock-up session is told of the design system it builds on: it
/// lives on its own branch and is served beside the pages, its variables
/// are the only ones to use, and its folders are never written — a file
/// there hides the design system's own.
#[must_use]
pub fn design_brief(tokens_css: &str) -> String {
    let mut text = String::from(
        "The design system lives on its own branch and is served beside the pages: \
         `../design/tokens.css` (its variables), `../kit/kit.css` and its kit's components \
         (`data-kit`). Never write anything under `mock/design/` or `mock/kit/`: a file there \
         hides the design system's own. Link `../design/tokens.css` and `../kit/kit.css` in \
         every page, and style everything through the variables below and no other — never a \
         colour, a font or a size written by hand.\n\n",
    );
    if tokens_css.trim().is_empty() {
        text.push_str("The design system has no variable yet: style with none.\n\n");
    } else {
        let _ = write!(
            text,
            "Its variables:\n\n```css\n{}\n```\n\n",
            tokens_css.trim()
        );
    }
    text
}

/// What a mock-up session is told of the interface examples lent beside
/// the pages (`mock/design/examples/`): each image and what to take from it,
/// and to say in its last message what it took. Empty when there is none.
#[must_use]
pub fn examples_part(examples: &[harness_architecture::kit::Example]) -> String {
    if examples.is_empty() {
        return String::new();
    }
    let mut text = String::from("\n");
    text.push_str(&super::design::examples_brief(examples, "mock/design/"));
    text.push_str(
        "They are read where they are: never copy one, never link one from a page. In your \
         last message, say what you took from each, one line per example: \
         `example <file>: <what you took>`.\n",
    );
    text
}

/// The lines of a session's last message that say what it took from an
/// interface example, `example <file>: <what>`, without their list marks.
#[must_use]
pub fn examples_taken(said: &str) -> Vec<String> {
    said.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['-', '*', ' '])
                .replace('`', "")
        })
        .filter(|l| {
            l.get(..8)
                .is_some_and(|w| w.eq_ignore_ascii_case("example "))
                && l.contains(':')
        })
        .collect()
}

/// The variables a stylesheet declares, `--name`, in their order.
#[must_use]
pub fn token_names(css: &str) -> Vec<String> {
    css.split([';', '{', '\n'])
        .filter_map(|decl| decl.trim().split_once(':'))
        .map(|(name, _)| name.trim())
        .filter(|name| name.starts_with("--"))
        .map(str::to_string)
        .collect()
}

/// The variables a text uses that the design system has not — those the
/// text itself declares (`--cols: 3`) aside.
#[must_use]
pub fn unknown_tokens(text: &str, tokens: &[String]) -> Vec<String> {
    let declared = token_names(text);
    let mut unknown = std::collections::BTreeSet::new();
    for (at, _) in text.match_indices("var(--") {
        let name: String = text
            .get(at + 4..)
            .unwrap_or_default()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !tokens.contains(&name) && !declared.contains(&name) {
            unknown.insert(name);
        }
    }
    unknown.into_iter().collect()
}

/// The session that repairs a `mock/pages.json` the view cannot read:
/// `rules` are the mock-up's own (the manifest's shape among them), `why`
/// what the reading said.
#[must_use]
pub fn manifest_repair_prompt(rules: &str, why: &str) -> String {
    let mut text = rules.to_string();
    let _ = write!(
        text,
        "\nThe factory view cannot read the mock-up: `{}`. Repair `mock/pages.json` so it reads \
         as the shape above — keep every page and micro-frontend it lists; where a field holds \
         several values and the shape wants one, keep the main one. Change no other file. Say in \
         one line what you repaired.\n",
        why.trim()
    );
    text
}

/// The session sent back to what the cut's gates refuse on the mock-up it
/// left: each gap with the file and the units it is about.
#[must_use]
pub fn cut_repair_prompt(rules: &str, gaps: &[String]) -> String {
    let mut text = rules.to_string();
    text.push_str(
        "\nThe factory's gates refuse the mock-up as you left it; the rules cannot be validated \
         while one of these stands:\n\n",
    );
    for gap in gaps {
        let _ = writeln!(text, "- {}", gap.trim());
    }
    text.push_str(
        "\nFix every one in the files under `mock/`. Keep every page, every micro-frontend's name \
         and what the human sees. Before you finish, read again each fragment you touched and \
         check each of its `actions` has an element carrying `data-writes=\"<DataCapability>\"` \
         and each of its `needs` one carrying `data-reads=\"<Capability>\"` — on the element \
         itself, a `data-kit` one included. Say in one line what you fixed.\n",
    );
    text
}

/// The session that makes the accepted mock-up again of micro-frontends,
/// keeping what the human saw: `rules` are the mock-up's own, `kit` the
/// design system's components.
#[must_use]
pub fn componentize_prompt(c: &Creation, rules: &str, kit: &str, tokens_css: &str) -> String {
    let mut text = rules.to_string();
    text.push_str(
        "\nThis proof of concept's mock-up was drawn fast: its pages may carry \
         their markup inline. Make it again of **micro-frontends**, as the product's architecture \
         wants it, without changing what the human sees. Read every page listed in \
         `mock/pages.json`; cut each into the Micro-UIs it is made of — a unit a user recognises \
         (an invoice list, an upload form, a balance), named in PascalCase, its system the bounded \
         context it belongs to — and for each one: write its fragment \
         `mock/components/<Name>.html` (its markup, its style on the design system's variables, its \
         script finding its root as `document.currentScript.closest('[data-component]')`, the \
         kit's components kept as `data-kit` elements), and list it under `components` of \
         `mock/pages.json` (name, system, description, fragment, fixtures, props; `needs` and \
         `actions` named as the Capabilities and DataCapabilities it would read and trigger, \
         `<system>.<verb><Thing>`). **Its data comes from one fixture per Capability it needs**, \
         `mock/fixtures/<Capability>.json` — what the mock server answers that Capability with, \
         what the rules' tests start from, shared by every micro-frontend that reads it — fetched \
         as `fetch(new URL('../fixtures/<Capability>.json', location.href))` and listed under its \
         `fixtures`. **Name its parts** (contract D): every actuator carries \
         `data-writes=\"<DataCapability>\"`, every part that shows a read \
         `data-reads=\"<Capability>\"`, each of its `actions` and `needs` at least once. A \
         micro-frontend shared by several pages is written once. Then rewrite each page as layout only: its regions, and in them \
         `<div data-component=\"<Name>\" data-props='{…}'></div>` for each micro-frontend, with \
         `<script src=\"../mock.js\"></script>` — no business markup left inline. Keep every `id` a page carries on its region. Keep each page's \
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
    text.push_str(&design_brief(tokens_css));
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
    /// Its fragment, relative to `mock/`.
    pub fragment: String,
    /// Its fixtures, one per Capability it needs, relative to `mock/`.
    pub fixtures: Vec<String>,
    /// The props it takes, `name: type`.
    pub props: BTreeMap<String, String>,
    /// Its fragment's markup as read when the context was made; empty when
    /// it was not read. Never written in the record.
    #[serde(skip)]
    pub markup: String,
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
/// The data model drafted at creation, beside the mock-up (contract M):
/// the write side installs it at `data/model.json`.
pub const DATA_MODEL: &str = "data/model.json";
/// Where the aggregates' integrity rules drafted at creation lie, one
/// `<Aggregate>.aggregate.json` each, as contract I has it: the task that
/// builds an aggregate installs its own at
/// `crates/dataguard/aggregates/<Aggregate>/aggregate.json`.
pub const DATA_AGGREGATES_DIR: &str = "data/aggregates";

/// The draft of one aggregate, under [`DATA_AGGREGATES_DIR`].
#[must_use]
pub fn aggregate_draft_path(aggregate: &str) -> String {
    format!("{DATA_AGGREGATES_DIR}/{aggregate}.aggregate.json")
}

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
        writes: Vec<WriteRef>,
        aggregates: Vec<&'a str>,
        ports: Option<&'a PortsDraft>,
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
        writes: writes_of(c, ctx),
        aggregates: c
            .data
            .iter()
            .flat_map(|d| d.aggregates.iter().map(|a| a.name.as_str()))
            .collect(),
        ports: c.ports.as_ref(),
    })
    .map_err(|e| e.to_string())
}

/// One `DataCapability` as the creation decided it: what contract F's
/// manifest declares when the task builds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WriteRef {
    /// Its identifier, `<system>.<verbThing>`.
    #[serde(rename = "dataCapability")]
    pub data_capability: String,
    /// What it does to the data, as the guard read it; `None` when no audit
    /// was made.
    pub effect: Option<super::audit::Effect>,
    /// The fields it changes, `table.field`, as the data step drafted them.
    pub touches: Vec<String>,
}

/// Every `DataCapability` a micro-frontend triggers, with its effect and
/// the fields it changes, by identifier.
#[must_use]
pub fn writes_of(c: &Creation, ctx: &Context) -> Vec<WriteRef> {
    let mut ids: Vec<&str> = ctx
        .components
        .iter()
        .flat_map(|m| m.actions.iter().map(String::as_str))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| WriteRef {
            data_capability: id.to_string(),
            effect: c.audit.as_ref().and_then(|a| a.effects.get(id).copied()),
            touches: c
                .data
                .as_ref()
                .and_then(|d| d.writes.get(id).cloned())
                .unwrap_or_default(),
        })
        .collect()
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

/// The proof of concept as decided, whole — the brief, the decisions, the
/// scope, every rule — with the pages that mock it, the micro-frontends and
/// the Capabilities they name, the design system, and the pins left on the
/// mock-up: what `mock/proof-of-concept.md` keeps in the repository.
#[must_use]
pub fn roadmap_record(c: &Creation, ctx: &Context) -> String {
    roadmap_body(c, ctx, &rules_section(c))
}

/// The roadmap task the factory starts on: the record, lighter — the rules
/// only counted, since the rules' task carries them and each milestone
/// names the ones it turns green — and fit under what GitHub takes, the
/// longest sections pointing at the record when it is still too long.
#[must_use]
pub fn roadmap_issue(c: &Creation, ctx: &Context) -> (String, String) {
    let title = format!("Proof of concept — {}", c.name);
    let whole = format!("`mock/{RECORD_MD}` {}", mockup_place(c));
    let body = roadmap_body(c, ctx, &rules_pointer(c));
    (title, fit_issue_body(&body, ISSUE_BUDGET, &whole))
}

/// What a body is fit under: GitHub's limit, less a margin for the
/// characters it counts twice.
const ISSUE_BUDGET: usize = ISSUE_BODY_MAX / 10 * 9;

/// `body` as it is when it fits in `max` characters; otherwise its longest
/// `## ` sections, one after the other, folded to their heading and a line
/// saying where they lie whole (`whole`), and as a last resort the end cut.
#[must_use]
pub fn fit_issue_body(body: &str, max: usize, whole: &str) -> String {
    let count = |text: &str| text.chars().count();
    if count(body) <= max {
        return body.to_string();
    }
    // The preamble, then one part per `## ` section.
    let mut parts = vec![String::new()];
    for line in body.split_inclusive('\n') {
        if line.starts_with("## ") {
            parts.push(String::new());
        }
        if let Some(last) = parts.last_mut() {
            last.push_str(line);
        }
    }
    let mut folded = vec![false; parts.len()];
    while parts.iter().map(|p| count(p)).sum::<usize>() > max {
        let Some(i) = (1..parts.len())
            .filter(|&i| !folded[i])
            .max_by_key(|&i| count(&parts[i]))
        else {
            break;
        };
        let heading = parts[i].lines().next().unwrap_or_default().to_string();
        parts[i] = format!(
            "{heading}\n\nToo long to stand in this issue ({} characters): read it whole in {whole}.\n\n",
            count(&parts[i])
        );
        folded[i] = true;
    }
    let fitted = parts.concat();
    if count(&fitted) <= max {
        return fitted;
    }
    let tail = format!("\n\n… cut here: read it whole in {whole}.\n");
    let kept: String = fitted
        .chars()
        .take(max.saturating_sub(count(&tail)))
        .collect();
    kept + &tail
}

/// The record's body, its rules section as given.
fn roadmap_body(c: &Creation, ctx: &Context, rules: &str) -> String {
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
        body.push_str("\n## Decisions\n");
        let topics = c.topics();
        // The product as a whole first, then each topic as first asked.
        for topic in std::iter::once(String::new()).chain(topics.iter().cloned()) {
            let under: Vec<&&Question> = answered.iter().filter(|q| q.topic == topic).collect();
            if under.is_empty() {
                continue;
            }
            if !topics.is_empty() {
                let heading = if topic.is_empty() { "General" } else { &topic };
                let _ = write!(body, "\n### {heading}\n");
            }
            body.push('\n');
            for q in under {
                let _ = writeln!(body, "- **{}** — {}", q.title, q.answer.trim());
                if !q.body.trim().is_empty() {
                    let _ = writeln!(body, "  (asked: {})", q.body.trim().replace('\n', " "));
                }
            }
        }
    }
    body.push_str(rules);
    body.push_str(&data_section(c));
    body.push_str(&ports_section(c));
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
             they read (`needs`, each answered by `mock/fixtures/<Capability>.json`) and the \
             DataCapabilities they trigger (`actions`). Every write's effect and the fields it \
             touches are in `mock/proof-of-concept.json` (`writes`).\n\n",
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
    body
}

/// The ports, as the roadmap task names them: each dependency outside the
/// product, its provider, who uses it. Empty when none was drafted.
fn ports_section(c: &Creation) -> String {
    let mut body = String::new();
    let Some(p) = &c.ports else {
        return body;
    };
    body.push_str(
        "\n## Ports\n\nEverything outside the product, each behind a port (`ports.json`, \
         contract Q): its adapter under `crates/ports/<port>/<provider>/`, its fake and contract \
         tests in `crates/ports/<port>/`, its secrets by name — a human provides them.\n\n",
    );
    for port in &p.ports {
        let _ = writeln!(
            body,
            "- **{}** — {} ({} {}){}: {}{}",
            port.id,
            port.provider,
            port.tech_kind,
            port.tech_name,
            if port.stateful { ", keeps state" } else { "" },
            port.purpose.trim(),
            if port.used_by.is_empty() {
                String::new()
            } else {
                format!(" — used by {}", port.used_by.join(", "))
            }
        );
    }
    body
}

/// The data model, as the roadmap task names it: where it lies, its tables
/// and aggregates, the invariants. Empty when none was drafted.
fn data_section(c: &Creation) -> String {
    let mut body = String::new();
    let Some(d) = &c.data else {
        return body;
    };
    let invariants: usize = d.aggregates.iter().map(|a| a.invariants.len()).sum();
    let _ = write!(
        body,
        "\n## Data\n\nDrafted and validated at creation, beside the mock-up: `mock/{DATA_MODEL}` \
         (contract M, {} tables) and one `mock/{DATA_AGGREGATES_DIR}/<Aggregate>.aggregate.json` \
         per aggregate (contract I, {} aggregates, {invariants} invariants). The task that \
         builds an aggregate installs its draft at \
         `crates/dataguard/aggregates/<Aggregate>/aggregate.json` and moves its tables into \
         `data/model.json` with its migration.\n\n",
        d.tables.len(),
        d.aggregates.len()
    );
    for a in &d.aggregates {
        let _ = writeln!(body, "- **{}** — {}", a.name, a.tables.join(", "));
        for i in &a.invariants {
            let _ = writeln!(body, "  - {} — {}", i.id, i.rule.trim());
        }
    }
    body
}

/// The rules as contract P has them (`rules.json`): `R<id>`, with its
/// family and criterion; those in `conform` marked so — changed, their
/// tests ahead of the code. A rule whose criterion is not whole cannot be
/// tested, and is answered apart rather than written.
#[must_use]
pub fn rules_contract(c: &Creation, conform: &[u32]) -> (serde_json::Value, Vec<u32>) {
    let mut rules = Vec::new();
    let mut untestable = Vec::new();
    for r in &c.requirements {
        let k = &r.criterion;
        if [&k.given, &k.when, &k.then]
            .iter()
            .any(|part| part.trim().is_empty())
            || r.title.trim().is_empty()
        {
            untestable.push(r.id);
            continue;
        }
        let mut rule = serde_json::json!({
            "id": format!("R{}", r.id),
            "title": r.title.trim(),
            "family": r.family,
            "criterion": {
                "given": k.given.trim(),
                "when": k.when.trim(),
                "then": k.then.trim(),
            },
        });
        if !r.body.trim().is_empty() {
            rule["body"] = serde_json::json!(r.body.trim());
        }
        if !r.pages.is_empty() {
            rule["pages"] = serde_json::json!(r.pages);
        }
        if !r.components.is_empty() {
            rule["components"] = serde_json::json!(r.components);
        }
        if conform.contains(&r.id) {
            rule["state"] = serde_json::json!("conform");
        }
        rules.push(rule);
    }
    (
        serde_json::json!({ "version": 1, "rules": rules }),
        untestable,
    )
}

/// What moved in the rules since the factory last received them: the rules
/// new or whose title, family or criterion changed — what their tests
/// prove — and the rules removed, by id.
#[must_use]
pub fn rules_moved(c: &Creation) -> (Vec<u32>, Vec<u32>) {
    let proved = |r: &Requirement| {
        (
            r.title.trim().to_string(),
            r.family,
            r.criterion.given.trim().to_string(),
            r.criterion.when.trim().to_string(),
            r.criterion.then.trim().to_string(),
        )
    };
    let changed = c
        .requirements
        .iter()
        .filter(|r| {
            c.rules_sent
                .iter()
                .find(|sent| sent.id == r.id)
                .is_none_or(|sent| proved(sent) != proved(r))
        })
        .map(|r| r.id)
        .collect();
    let removed = c
        .rules_sent
        .iter()
        .filter(|sent| !c.requirements.iter().any(|r| r.id == sent.id))
        .map(|sent| sent.id)
        .collect();
    (changed, removed)
}

/// The ids, as the rules name them: `R3, R5`.
fn named(ids: &[u32]) -> String {
    ids.iter()
        .map(|id| format!("R{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The rules' task a change of the rules opens, once the factory runs:
/// every rule again, the changed ones marked `conform` — their tests are
/// rewritten ahead of the code.
///
/// # Errors
///
/// Nothing moved, or the rules do not keep contract P.
pub fn rules_change_issue(c: &Creation) -> Result<(String, String), String> {
    let (changed, removed) = rules_moved(c);
    if changed.is_empty() && removed.is_empty() {
        return Err("no rule changed since the factory received them".to_string());
    }
    let (rules, untestable) = rules_contract(c, &changed);
    let mut moved = Vec::new();
    if !changed.is_empty() {
        moved.push(format!("changed or new: {}", named(&changed)));
    }
    if !removed.is_empty() {
        moved.push(format!("removed: {}", named(&removed)));
    }
    let mut intro = format!(
        "The rules of `{}` changed in the factory's design after it was initialized — {}. \
         Their tests are rewritten ahead of the code, the changed rules marked `conform`; \
         a removed rule's tests are removed. A conformity task brings the code to them. \
         No development task starts while this issue is open.",
        c.name,
        moved.join("; ")
    );
    if !untestable.is_empty() {
        let _ = write!(
            intro,
            "\n\nLeft out, their criterion not whole: {}.",
            named(&untestable)
        );
    }
    let mut ids = changed;
    ids.extend(removed);
    let title = format!(
        "{} {}",
        harness_workflows::rules::data::request::CHANGED_TITLE,
        named(&ids)
    );
    let body = harness_workflows::rules::data::request::body(&intro, &rules)?;
    Ok((title, body))
}

/// The conformity task a change of the rules opens beside its rules'
/// task: the code brought to the changed rules' tests. Its title and body.
#[must_use]
pub fn conformity_issue(c: &Creation, rules_issue: &str) -> (String, String) {
    let (changed, removed) = rules_moved(c);
    let title = format!("Conform to the changed rules: {}", named(&changed));
    let mut body = format!(
        "The rules {} of `{}` changed in the factory's design; the rules' task ({rules_issue}) \
         rewrites their tests, marked `conform` in `rules.json`, ahead of the code.\n\n\
         Bring the product to them: their tests under `tests/rules/` pass. Never change \
         `rules.json` or `tests/rules/` — a test that cannot hold is contested with \
         `CONTEST.md`.\n",
        named(&changed),
        c.name
    );
    if !removed.is_empty() {
        let _ = write!(
            body,
            "\nRemoved: {} — remove what only they asked for.\n",
            named(&removed)
        );
    }
    body.push_str("\nThe rules as they stand:\n");
    for r in c.requirements.iter().filter(|r| changed.contains(&r.id)) {
        let _ = write!(
            body,
            "\n- **R{} {}** — given {}, when {}, then {}",
            r.id,
            r.title.trim(),
            r.criterion.given.trim(),
            r.criterion.when.trim(),
            r.criterion.then.trim()
        );
    }
    body.push('\n');
    (title, body)
}

/// The rules' task the initialization opens, first of the roadmap: every
/// rule, to be written as `rules.json` and proved by tests before any
/// development.
///
/// # Errors
///
/// The rules do not keep contract P.
pub fn rules_issue(c: &Creation) -> Result<(String, String), String> {
    let (rules, untestable) = rules_contract(c, &[]);
    let mut intro = format!(
        "The first task of `{}`'s roadmap: the rules validated in the factory's Rules step, \
         written as `rules.json` (contract P) and proved by tests before any development. \
         No development task starts while this issue is open.",
        c.name
    );
    if !untestable.is_empty() {
        let ids: Vec<String> = untestable.iter().map(|id| format!("R{id}")).collect();
        let _ = write!(
            intro,
            "\n\nLeft out, their criterion not whole: {}.",
            ids.join(", ")
        );
    }
    let body = harness_workflows::rules::data::request::body(&intro, &rules)?;
    Ok((
        harness_workflows::rules::data::request::FIRST_TITLE.to_string(),
        body,
    ))
}

/// The rules, as the roadmap task lists them: each with its family, what it
/// asks for, its criterion and what carries it. Empty when there is none.
/// The rules, as the roadmap task names them: counted, where they are.
fn rules_pointer(c: &Creation) -> String {
    if c.requirements.is_empty() {
        return String::new();
    }
    let journeys = c
        .requirements
        .iter()
        .filter(|r| r.family == Family::Journey)
        .count();
    let tasked = if c.rules_issue.is_empty() {
        "the rules' task".to_string()
    } else {
        format!("the rules' task ({})", c.rules_issue)
    };
    format!(
        "\n## Rules\n\n{} rule{} ({journeys} journey, {} business), carried by {tasked}, which \
         writes them in `rules.json` with their tests: each milestone names the ones it turns \
         green. Every rule in full: `mock/{RECORD_MD}`.\n",
        c.requirements.len(),
        if c.requirements.len() == 1 { "" } else { "s" },
        c.requirements.len() - journeys,
    )
}

fn rules_section(c: &Creation) -> String {
    let mut body = String::new();
    if c.requirements.is_empty() {
        return body;
    }
    body.push_str("\n## Rules\n\n");
    for r in &c.requirements {
        let family = match r.family {
            Family::Journey => "journey",
            Family::Business => "business",
        };
        let _ = write!(body, "- **R{} {}** ({family})", r.id, r.title.trim());
        if !r.body.trim().is_empty() {
            let _ = write!(body, " — {}", r.body.trim().replace('\n', " "));
        }
        if !r.criterion.is_empty() {
            let _ = write!(
                body,
                " — criterion: {}",
                r.criterion.line().replace('\n', " ")
            );
        }
        let mut on = Vec::new();
        on.extend(r.pages.iter().map(|p| format!("`mock/{p}`")));
        on.extend(r.components.iter().map(|m| format!("`{m}`")));
        if !on.is_empty() {
            let _ = write!(body, " — on {}", on.join(", "));
        }
        body.push('\n');
    }
    body
}

/// The question that reads the requirements off the scope and the mock-up:
/// each tied to the pages and the micro-frontends that carry it.
#[must_use]
pub fn requirements_prompt(c: &Creation, ctx: &Context) -> String {
    let mut text = String::from(
        "You read the requirements of a proof of concept off its scope and its mock-up, for \
         the human to retouch before the development starts. A requirement is one thing the \
         application must do or keep: a user journey's step, a rule it enforces, a thing it \
         keeps, a proof the human expects — never a page or a component as such. Each is of a \
         family: `journey` (a step of a user's journey: what the user does and sees) or \
         `business` (an invariant, a computation, a thing the application keeps — what the data \
         must hold true). Each carries its criterion: how a test tells it is met, in three \
         parts — `given` the situation before, `when` what happens, `then` what must then be \
         true — each a clause, none left empty. Write each in the language of the human's \
         brief, and tie it to the pages and the micro-frontends of the mock-up that carry it, \
         by the exact paths and names listed below. Answer with one JSON object and nothing \
         else: {\"requirements\": [{\"title\": \"the requirement in a few words\", \"body\": \
         \"what it asks for, one or two sentences\", \"family\": \"journey\", \"criterion\": \
         {\"given\": \"…\", \"when\": \"…\", \"then\": \"…\"}, \"pages\": [\"pages/….html\"], \
         \"components\": [\"Name\"]}]}. Between eight and twenty-five requirements, the \
         scope's order.\n\n",
    );
    text.push_str(&mockup_context(c, ctx));
    if c.requirements.iter().any(|r| r.human) {
        text.push_str(
            "\nThe human wrote these already; they are kept, do not repeat them. Tie each again \
             to the pages and the micro-frontends listed above that carry it now, beside the \
             requirements: \"ties\": [{\"id\": <its number>, \"pages\": [\"pages/….html\"], \
             \"components\": [\"Name\"]}].\n\n",
        );
        for r in c.requirements.iter().filter(|r| r.human) {
            let _ = writeln!(text, "- R{}: {}", r.id, r.title.trim());
        }
    }
    text
}

/// The question that keeps the rules in step with a change of the mock-up:
/// the rules it makes wrong or incomplete rewritten, those it calls for
/// added — each a proposal the human accepts or refuses.
#[must_use]
pub fn rules_change_prompt(c: &Creation, ctx: &Context, change: &RuleChange) -> String {
    let mut text = String::from(
        "You keep the requirements of a proof of concept in step with its mock-up. The human \
         just changed the mock-up; propose the changes to the requirements it calls for: a \
         requirement it makes wrong or incomplete, rewritten whole under its number; a \
         requirement it calls for that none covers, with the number 0. Leave out every \
         requirement it does not touch. A requirement is of a family, `journey` or `business`, \
         and carries its criterion in three parts — `given`, `when`, `then` — none left empty. \
         Write in the language of the human's brief, and tie each to the pages and the \
         micro-frontends that carry it, by the exact paths and names listed below; a fork \
         carries, on its page, what the micro-frontend it was forked from carried there. Answer \
         with one JSON object and nothing else: {\"proposals\": [{\"id\": 0, \"why\": \"why it \
         changes, one sentence\", \"title\": \"…\", \"body\": \"…\", \"family\": \"journey\", \
         \"criterion\": {\"given\": \"…\", \"when\": \"…\", \"then\": \"…\"}, \"pages\": \
         [\"pages/….html\"], \"components\": [\"Name\"]}]}, with an empty list when nothing \
         changes.\n\n",
    );
    text.push_str(&mockup_context(c, ctx));
    let _ = write!(text, "\nWhat changed: {}.\n", change.said());
    if !change.requests.is_empty() {
        text.push_str("The human asked for it in these words:\n\n");
        for r in &change.requests {
            let _ = writeln!(text, "- {}", r.trim().replace('\n', " "));
        }
    }
    text.push_str(
        "\nThe requirements as they stand (number — family — title — what it asks — criterion \
         — pages — micro-frontends), those tied to what changed marked *:\n\n",
    );
    for r in &c.requirements {
        let tied = r.components.contains(&change.name)
            || (!change.page.is_empty() && r.pages.contains(&change.page));
        let _ = writeln!(
            text,
            "- {}R{} — {} — {} — {} — given {}, when {}, then {} — {} — {}",
            if tied { "*" } else { "" },
            r.id,
            serde_json::to_string(&r.family)
                .unwrap_or_default()
                .trim_matches('"'),
            r.title.trim(),
            r.body.trim().replace('\n', " "),
            r.criterion.given.trim(),
            r.criterion.when.trim(),
            r.criterion.then.trim(),
            r.pages.join(", "),
            r.components.join(", ")
        );
    }
    text
}

/// What the AI is told of the product and its mock-up: the product, the
/// scope, the pages and the micro-frontends.
fn mockup_context(c: &Creation, ctx: &Context) -> String {
    let mut text = product(c);
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
    text
}

/// The question that reads one rule again off the scope and the mock-up as
/// it stands: the same reading, for that rule alone.
#[must_use]
pub fn rule_prompt(c: &Creation, ctx: &Context, rule: &Requirement) -> String {
    let mut text = requirements_prompt(c, ctx);
    let _ = write!(
        text,
        "\nRead again this one requirement only, and answer it as the only entry of \
         `requirements`: keep its meaning, tie it to what carries it in the mock-up as it stands, \
         settle its family and write its criterion.\n\n- title: {}\n- what it asks for: {}\n",
        rule.title.trim(),
        rule.body.trim().replace('\n', " ")
    );
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

    /// A scope with the one section a drawing needs.
    const SCOPE: &str = "## Goal\n\n## Screens\n\n- **Home** — the ledger";

    fn ready() -> Creation {
        Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            questions: vec![question(1, "yes")],
            scope: SCOPE.to_string(),
            mocked: Some("two pages".to_string()),
            rules_validated: true,
            accepted: true,
            ..Creation::new("app", "me/app")
        }
    }

    fn mark(c: &Creation, step: Step) -> Mark {
        c.marks(DESIGNED)
            .into_iter()
            .find(|(s, _)| *s == step)
            .map(|(_, m)| m)
            .expect("every step has a mark")
    }

    #[test]
    fn the_steps_before_the_furthest_are_done_the_furthest_open_the_rest_locked() {
        let mut c = ready();
        c.accepted = false;
        let marks = c.marks(DESIGNED);
        assert_eq!(marks.len(), Step::ALL.len());
        assert_eq!(mark(&c, Step::Kind), Mark::Done);
        assert_eq!(mark(&c, Step::Mockups), Mark::Done);
        assert_eq!(mark(&c, Step::Rules), Mark::Done);
        assert_eq!(mark(&c, Step::Data), Mark::Open);
        assert_eq!(mark(&c, Step::Initialize), Mark::Locked);
        c.rules_validated = false;
        assert_eq!(mark(&c, Step::Rules), Mark::Open);
        assert_eq!(mark(&c, Step::Data), Mark::Locked);
        c.rules_validated = true;
        c.accepted = true;
        assert_eq!(mark(&c, Step::Initialize), Mark::Open);
        c.issue = "https://github.com/me/app/issues/1".to_string();
        assert_eq!(mark(&c, Step::Initialize), Mark::Done);
    }

    #[test]
    fn a_stale_scope_makes_every_step_drawn_from_it_stale_and_no_other() {
        let mut c = ready();
        c.accepted = false;
        c.rules_validated = false;
        c.mocked_scope.clone_from(&c.scope);
        c.scope_from = c.fingerprint();
        assert_eq!(mark(&c, Step::Scope), Mark::Done);
        c.questions[0].answer = "no".to_string();
        assert!(c.scope_stale());
        assert_eq!(mark(&c, Step::Scope), Mark::Stale);
        assert_eq!(mark(&c, Step::Mockups), Mark::Stale);
        assert_eq!(
            mark(&c, Step::Design),
            Mark::Done,
            "not drawn from the scope"
        );
        assert_eq!(mark(&c, Step::Grilling), Mark::Done);
        assert_eq!(
            mark(&c, Step::Rules),
            Mark::Open,
            "not made yet: nothing to review"
        );
    }

    #[test]
    fn a_scope_moved_since_the_drawing_makes_the_mock_ups_stale_not_the_scope() {
        let mut c = ready();
        c.mocked_scope = "## Older goal".to_string();
        assert_eq!(mark(&c, Step::Scope), Mark::Done);
        assert_eq!(mark(&c, Step::Mockups), Mark::Stale);
        assert_eq!(mark(&c, Step::Rules), Mark::Stale);
    }

    #[test]
    fn a_gate_failing_before_where_the_human_went_blocks_its_step() {
        let mut c = ready();
        c.go(Step::Initialize, DESIGNED).expect("open");
        assert_eq!(c.visited, Step::Initialize);
        c.go(Step::Brief, DESIGNED).expect("back is open");
        assert_eq!(
            c.visited,
            Step::Initialize,
            "going back keeps how far it went"
        );
        c.accepted = false;
        c.mocked = None;
        assert_eq!(mark(&c, Step::Mockups), Mark::Blocked);
        assert_eq!(mark(&c, Step::Rules), Mark::Locked);
        let mut fresh = ready();
        fresh.mocked = None;
        assert_eq!(
            mark(&fresh, Step::Mockups),
            Mark::Open,
            "never went beyond it"
        );
    }

    /// A complete criterion opening on `given`; none when it is empty.
    fn crit(given: &str) -> Criterion {
        if given.is_empty() {
            return Criterion::default();
        }
        Criterion {
            given: given.to_string(),
            when: "it is used".to_string(),
            then: "it holds".to_string(),
        }
    }

    fn rule(id: u32, criterion: &str, pages: &[&str]) -> Requirement {
        Requirement {
            id,
            title: format!("rule {id}"),
            criterion: crit(criterion),
            pages: pages.iter().map(ToString::to_string).collect(),
            ..Requirement::default()
        }
    }

    #[test]
    fn the_rules_task_carries_every_testable_rule_as_contract_p() {
        let mut business = rule(
            3,
            "given lines, when opened, then all show",
            &["pages/a.html"],
        );
        business.family = Family::Business;
        business.body = "  every line ".to_string();
        business.components = vec!["Lines".to_string()];
        let c = Creation {
            name: "app".to_string(),
            requirements: vec![
                business,
                rule(5, "", &[]),
                rule(7, "given a, when b, then c", &[]),
            ],
            ..Creation::default()
        };
        let (rules, untestable) = rules_contract(&c, &[7]);
        assert_eq!(untestable, [5], "a criterion not whole cannot be tested");
        assert_eq!(rules["version"], 1);
        assert_eq!(rules["rules"][0]["id"], "R3");
        assert_eq!(rules["rules"][0]["family"], "business");
        assert_eq!(rules["rules"][0]["body"], "every line");
        assert_eq!(rules["rules"][0]["pages"][0], "pages/a.html");
        assert_eq!(rules["rules"][0]["components"][0], "Lines");
        assert_eq!(
            rules["rules"][0]["criterion"]["when"],
            crit("given lines, when opened, then all show")
                .when
                .as_str()
        );
        assert!(rules["rules"][0].get("state").is_none());
        assert_eq!(rules["rules"][1]["id"], "R7");
        assert_eq!(rules["rules"][1]["state"], "conform");
        assert!(rules["rules"][1].get("body").is_none());

        let (title, body) = rules_issue(&c).expect("contract P");
        assert_eq!(title, harness_workflows::rules::data::request::FIRST_TITLE);
        assert!(
            body.contains("`app`") && body.contains("Left out, their criterion not whole: R5.")
        );
        let read = harness_workflows::rules::data::request::read(&title, &body);
        assert!(
            matches!(read, Ok(harness_workflows::rules::data::request::Ask::Write(r)) if r.ids == ["R3", "R7"] && r.conform.is_empty())
        );
    }

    #[test]
    fn a_rule_changed_after_initialize_opens_its_rules_task_and_its_conformity() {
        let sent = vec![
            rule(1, "given a, when b, then c", &[]),
            rule(2, "given a, when b, then c", &[]),
            rule(3, "given a, when b, then c", &[]),
        ];
        let mut c = Creation {
            name: "app".to_string(),
            requirements: sent.clone(),
            rules_sent: sent,
            ..Creation::default()
        };
        assert_eq!(rules_moved(&c), (vec![], vec![]));
        assert!(rules_change_issue(&c).is_err(), "nothing to send");
        // A page moved: nothing its tests prove.
        c.requirements[0].pages = vec!["pages/b.html".to_string()];
        assert_eq!(rules_moved(&c), (vec![], vec![]));
        c.requirements[1].criterion = crit("given a, when b, then d");
        c.requirements.remove(2);
        c.requirements.push(rule(4, "given x, when y, then z", &[]));
        assert_eq!(rules_moved(&c), (vec![2, 4], vec![3]));

        let (title, body) = rules_change_issue(&c).expect("sent");
        assert_eq!(title, "Rules changed: R2, R4, R3");
        assert!(
            body.contains("changed or new: R2, R4; removed: R3"),
            "{body}"
        );
        let Ok(harness_workflows::rules::data::request::Ask::Write(read)) =
            harness_workflows::rules::data::request::read(&title, &body)
        else {
            panic!("the rules' task reads it");
        };
        assert_eq!(read.ids, ["R1", "R2", "R4"]);
        assert_eq!(read.conform, ["R2", "R4"]);

        let (title, body) = conformity_issue(&c, "https://x/issues/9");
        assert_eq!(title, "Conform to the changed rules: R2, R4");
        assert!(body.contains("https://x/issues/9"));
        assert!(body.contains("Removed: R3"));
        assert!(body.contains("CONTEST.md") && body.contains("Never change"));
        assert!(body.contains("**R4 rule 4**"), "{body}");
    }

    #[test]
    fn a_change_of_the_mock_up_proposes_rules_the_human_accepts_or_refuses() {
        let mut c = Creation {
            requirements: vec![rule(3, "x", &["pages/a.html"]), rule(5, "x", &[])],
            last_requirement: 5,
            rules_validated: true,
            ..Creation::default()
        };
        let pages = ["pages/a.html".to_string()];
        let components = ["Ai".to_string(), "Ai2".to_string()];
        let read = parse_proposals(
            r#"Here: {"proposals": [
                {"id": 3, "why": "two modes now", "title": "Connect", "body": "by URL", "family": "business",
                 "criterion": {"given": "a URL", "when": "tested", "then": "the kind is found"},
                 "pages": ["pages/a.html", "pages/gone.html"], "components": ["Ai2", "Gone"]},
                {"id": 0, "title": "Binary", "criterion": {"given": "g", "when": "w", "then": "t"}},
                {"id": 9, "title": "Unknown rule"},
                {"id": 0, "title": "  "}
            ]}"#,
            &pages,
            &components,
        )
        .expect("read");
        assert_eq!(read.len(), 3, "a proposal with no title is none");
        assert_eq!(read[0].pages, ["pages/a.html"]);
        assert_eq!(read[0].components, ["Ai2"]);
        assert_eq!(
            parse_proposals(r#"{"proposals": []}"#, &pages, &components),
            Ok(vec![])
        );
        assert!(parse_proposals("no json", &pages, &components).is_err());

        let change = RuleChange {
            pin: String::new(),
            name: "Ai".to_string(),
            saved: "Ai2".to_string(),
            how: "fork".to_string(),
            page: "pages/a.html".to_string(),
            requests: vec!["two modes".to_string()],
        };
        c.propose_rules(&change.said(), read.clone());
        assert_eq!(
            c.proposals
                .iter()
                .map(|p| (p.key, p.id))
                .collect::<Vec<_>>(),
            [(1, 3), (2, 0)],
            "numbered; one about a rule that is not there left out"
        );
        assert_eq!(c.proposals[0].about, "Ai forked as Ai2 for pages/a.html");
        // A newer proposal about a rule replaces the older one.
        c.propose_rules("again", vec![read[0].clone()]);
        assert_eq!(
            c.proposals
                .iter()
                .map(|p| (p.key, p.id))
                .collect::<Vec<_>>(),
            [(2, 0), (3, 3)]
        );
        assert!(c.rules_validated, "proposing changes no rule");

        c.settle_proposal(3, true).expect("accepted");
        let r3 = c.requirements.iter().find(|r| r.id == 3).expect("R3");
        assert_eq!((r3.title.as_str(), r3.human), ("Connect", true));
        assert_eq!(r3.criterion.then, "the kind is found");
        assert!(
            !c.rules_validated,
            "a rule changed takes the validation back"
        );
        c.settle_proposal(2, true).expect("added");
        assert_eq!(
            c.requirements.last().map(|r| (r.id, r.title.as_str())),
            Some((6, "Binary"))
        );
        let seen = &c.proposals;
        assert!(seen.is_empty(), "{seen:?}");
        assert!(c.settle_proposal(2, true).is_err(), "settled once");

        c.propose_rules("x", vec![read[0].clone()]);
        c.requirements.retain(|r| r.id != 3);
        assert_eq!(c.settle_proposal(4, true), Err("R3 is gone".to_string()));
        c.propose_rules("x", vec![read[1].clone()]);
        c.settle_proposal(5, false).expect("refused");
        assert_eq!(c.requirements.len(), 2, "refused: nothing added");

        assert_eq!(
            RuleChange {
                how: "replace".to_string(),
                ..change.clone()
            }
            .said(),
            "Ai changed on every page that shows it"
        );
        let prompt = rules_change_prompt(
            &Creation {
                requirements: vec![rule(3, "x", &["pages/a.html"]), rule(5, "x", &[])],
                ..Creation::default()
            },
            &Context::default(),
            &change,
        );
        assert!(prompt.contains("What changed: Ai forked as Ai2 for pages/a.html."));
        assert!(prompt.contains("- two modes"));
        assert!(prompt.contains("- *R3 — "), "tied to the page: marked");
        assert!(prompt.contains("- R5 — "), "not tied: listed, unmarked");
    }

    #[test]
    fn a_pin_s_proposals_are_accepted_with_it_retied_to_its_fork_and_taken_back_by_its_undo() {
        let mut c = Creation {
            requirements: vec![rule(3, "x", &["pages/a.html"]), rule(5, "x", &[])],
            last_requirement: 5,
            ..Creation::default()
        };
        let proposal = |id: u32, title: &str| Proposal {
            id,
            title: title.to_string(),
            pages: vec!["pages/a.html".to_string()],
            components: vec!["Ai".to_string()],
            ..Proposal::default()
        };
        let drafted = |pin: &str| RuleChange {
            pin: pin.to_string(),
            name: "Ai".to_string(),
            page: "pages/a.html".to_string(),
            ..RuleChange::default()
        };
        assert_eq!(
            drafted("p").said(),
            "Ai drafted on a request, on pages/a.html"
        );
        assert_eq!(
            RuleChange {
                name: String::new(),
                ..drafted("p")
            }
            .said(),
            "the page pages/a.html drafted on a request"
        );
        c.propose_for(
            &drafted("p"),
            vec![proposal(3, "Connect"), proposal(0, "Binary")],
        );
        c.propose_for(&drafted("q"), vec![proposal(5, "Other pin")]);
        // Proposed again: the pin's waiting proposals give way to the new ones.
        c.propose_for(&drafted("q"), vec![proposal(0, "Other pin, again")]);
        assert_eq!(
            c.proposals
                .iter()
                .filter(|p| p.pin == "q")
                .map(|p| p.title.as_str())
                .collect::<Vec<_>>(),
            ["Other pin, again"]
        );
        assert!(c.proposals.iter().take(2).all(|p| p.pin == "p"));
        let pins = ["p".to_string()];

        // Forked for the page, then validated: retied to the fork, accepted.
        c.retie_fork(&pins, "pages/a.html", "Ai", "Ai2");
        assert_eq!(c.proposals[0].components, ["Ai2"]);
        assert_eq!(
            c.proposals[2].components,
            ["Ai"],
            "another pin's: untouched"
        );
        let seen = &c.accept_pins(&pins);
        assert!(seen.is_empty(), "{seen:?}");
        assert_eq!(c.proposals.len(), 1, "the other pin's still waits");
        assert_eq!(c.applied.len(), 2);
        let titles = |c: &Creation| {
            c.requirements
                .iter()
                .map(|r| (r.id, r.title.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(&c),
            [
                (3, "Connect".to_string()),
                (5, "rule 5".to_string()),
                (6, "Binary".to_string())
            ]
        );

        // The human retouches the new rule; the pin's Undo leaves it.
        c.requirements[2].body = "mine now".to_string();
        let kept = c.undo_pins(&pins);
        assert_eq!(kept, ["R6 retouched since: kept"]);
        assert_eq!(
            titles(&c),
            [
                (3, "rule 3".to_string()),
                (5, "rule 5".to_string()),
                (6, "Binary".to_string())
            ]
        );
        let seen = &c.applied;
        assert!(seen.is_empty(), "{seen:?}");
        assert_eq!(c.proposals.len(), 1, "another pin's proposal stays");

        // A proposal of a new rule accepted then undone: the rule goes.
        c.propose_for(&drafted("r"), vec![proposal(0, "Fresh")]);
        c.accept_pins(&["r".to_string()]);
        assert!(c.requirements.iter().any(|r| r.title == "Fresh"));
        let seen = &c.undo_pins(&["r".to_string()]);
        assert!(seen.is_empty(), "{seen:?}");
        assert!(!c.requirements.iter().any(|r| r.title == "Fresh"));
        // Waiting proposals of a pin go with its undo.
        c.undo_pins(&["q".to_string()]);
        let seen = &c.proposals;
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_criterion_written_as_one_sentence_is_cut_into_its_three_parts() {
        let english = Criterion::from_sentence(
            " Given a debt, when the client pays, then it is settled, and kept ",
        );
        assert_eq!(
            (
                english.given.as_str(),
                english.when.as_str(),
                english.then.as_str()
            ),
            ("a debt", "the client pays", "it is settled, and kept")
        );
        let french = Criterion::from_sentence(
            "Étant donné une campagne exportée alors qu'elle existe, quand l'import se termine, alors une copie est créée",
        );
        assert_eq!(
            (
                french.given.as_str(),
                french.when.as_str(),
                french.then.as_str()
            ),
            (
                "une campagne exportée alors qu'elle existe",
                "l'import se termine",
                "une copie est créée"
            )
        );
        for loose in [
            "given a debt",
            "given a debt, when paid",
            "when paid, then settled",
            "a debt, when paid, then settled",
        ] {
            let kept = Criterion::from_sentence(loose);
            assert_eq!(
                (kept.given.as_str(), kept.when.as_str(), kept.then.as_str()),
                (loose, "", ""),
                "{loose}"
            );
        }
        assert!(Criterion::from_sentence("  ").is_empty());
        let old: Criterion =
            serde_json::from_str(r#""given a, when b, then c""#).expect("a sentence reads");
        assert_eq!(old.line(), "given a; when b; then c");
        let parts: Criterion = serde_json::from_str(r#"{"given": "a"}"#).expect("parts read");
        assert_eq!(parts.missing(), vec!["when", "then"]);
        assert!(!parts.is_empty());
    }

    #[test]
    fn a_criterion_with_a_part_missing_keeps_the_rules_from_validation() {
        let mut c = ready();
        c.requirements = vec![rule(1, "y", &["pages/a.html"])];
        c.requirements[0].criterion.when.clear();
        assert_eq!(
            c.rule_gaps(),
            vec!["R1's criterion has no when".to_string()]
        );
        c.requirements[0].criterion.then = " ".to_string();
        assert_eq!(
            c.rule_gaps(),
            vec!["R1's criterion has no when and no then".to_string()]
        );
    }

    #[test]
    fn rules_are_validated_only_with_a_criterion_and_a_support_each() {
        let mut c = ready();
        c.rules_validated = false;
        assert_eq!(c.rule_gaps(), vec!["no rule yet".to_string()]);
        c.requirements = vec![rule(1, "", &["pages/a.html"]), rule(2, "given x", &[])];
        assert_eq!(
            c.rule_gaps(),
            vec![
                "R1 has no criterion".to_string(),
                "R2 is carried by no page and no micro-frontend".to_string()
            ]
        );
        assert!(c.validate_rules().is_err());
        assert!(!c.rules_validated);
        c.requirements[0].criterion = crit("y");
        c.requirements[1].components = vec!["Card".to_string()];
        assert!(c.requirements[1].covered());
        c.validate_rules().expect("nothing keeps them");
        assert!(c.rules_validated);
    }

    #[test]
    fn a_write_passed_off_as_a_read_keeps_the_rules_from_validation_until_fixed() {
        use super::super::audit::{Check, Finding};
        let mut c = ready();
        c.requirements = vec![rule(1, "given y", &["pages/a.html"])];
        c.componentized = Some("cut one".to_string());
        c.audit = Some(Audit {
            findings: vec![
                Finding {
                    id: 1,
                    check: Some(Check::Naming),
                    why: "a name".to_string(),
                    ..Finding::default()
                },
                Finding {
                    id: 2,
                    check: Some(Check::WriteAsRead),
                    why: " it creates ".to_string(),
                    ..Finding::default()
                },
            ],
            from: "cut one".to_string(),
            ..Audit::default()
        });
        assert_eq!(c.rule_gaps(), vec!["architecture: it creates".to_string()]);
        assert!(c.validate_rules().is_err());
        assert!(!c.audit_stale());
        c.componentized = Some("cut two".to_string());
        assert!(c.audit_stale());
        if let Some(a) = c.audit.as_mut() {
            a.resolve(2, true).expect("fixed");
        }
        c.validate_rules().expect("only a warning left");
        c.audit = None;
        assert!(!c.audit_stale());
    }

    #[test]
    fn a_data_model_drafted_from_rules_that_moved_is_stale_and_not_validated() {
        use super::super::data_draft::{DataDraft, Table};
        let mut c = ready();
        c.requirements = vec![rule(1, "given y", &["pages/a.html"])];
        c.data = Some(DataDraft {
            tables: vec![Table {
                name: "line".to_string(),
                ..Table::default()
            }],
            from: rules_print(&c),
            ..DataDraft::default()
        });
        assert!(!c.data_stale());
        assert!(!c.data_validated(), "drafted, not validated");
        c.validate_data().expect("no gap");
        assert!(c.data_validated());
        assert_eq!(mark(&c, Step::Data), Mark::Done);
        c.requirements[0].criterion = crit("z");
        assert!(c.data_stale());
        assert!(!c.data_validated(), "validated, but from other rules");
        assert_eq!(
            mark(&c, Step::Rules),
            Mark::Done,
            "the rules themselves are not stale"
        );
        assert_eq!(mark(&c, Step::Data), Mark::Stale);
        c.data = None;
        assert!(!c.data_stale() && !c.data_validated());
        assert_eq!(c.data_gaps(), vec!["no data model yet".to_string()]);
    }

    #[test]
    fn the_roadmap_issue_lists_the_ports_and_who_uses_them() {
        use crate::domain::ports_draft::{PortDraft, PortsDraft, database};
        let mut c = ready();
        assert_eq!(ports_section(&c), "", "no ports drafted");
        c.ports = Some(PortsDraft {
            ports: vec![
                database(),
                PortDraft {
                    id: "mail".to_string(),
                    purpose: " Receipts ".to_string(),
                    provider: "Postmark".to_string(),
                    tech_kind: "rest".to_string(),
                    tech_name: "postmark".to_string(),
                    used_by: vec!["billing.send".to_string(), "billing.remind".to_string()],
                    ..PortDraft::default()
                },
            ],
            ..PortsDraft::default()
        });
        let section = ports_section(&c);
        assert!(section.starts_with("\n## Ports\n\nEverything outside the product"));
        assert!(
            section.contains("\n- **mail** — Postmark (rest postmark): Receipts — used by billing.send, billing.remind\n"),
            "{section}"
        );
        assert!(
            section.contains("- **database** — PostgreSQL 16 (sql postgres), keeps state: "),
            "{section}"
        );
    }

    #[test]
    fn the_roadmap_issue_names_the_data_model_and_its_invariants() {
        use super::super::data_draft::{Aggregate, DataDraft, Invariant, Table};
        let mut c = ready();
        assert!(!roadmap_issue(&c, &Context::default()).1.contains("## Data"));
        c.data = Some(DataDraft {
            tables: vec![Table::default(), Table::default()],
            aggregates: vec![Aggregate {
                name: "Line".to_string(),
                tables: vec!["line".to_string(), "total".to_string()],
                invariants: vec![Invariant {
                    id: "positive".to_string(),
                    rule: " above zero ".to_string(),
                    ..Invariant::default()
                }],
                ..Aggregate::default()
            }],
            ..DataDraft::default()
        });
        let (_, body) = roadmap_issue(&c, &Context::default());
        assert!(body.contains("\n## Data\n\nDrafted and validated at creation, beside the mock-up: `mock/data/model.json` (contract M, 2 tables) and one `mock/data/aggregates/<Aggregate>.aggregate.json` per aggregate (contract I, 1 aggregates, 1 invariants)"), "{body}");
        assert!(
            body.contains("- **Line** — line, total\n  - positive — above zero\n"),
            "{body}"
        );
    }

    #[test]
    fn a_tie_to_what_the_mock_up_has_not_is_named() {
        let mut c = ready();
        c.requirements = vec![rule(1, "y", &["pages/a.html", "pages/gone.html"])];
        c.requirements[0].components = vec!["Card".to_string(), "Ghost".to_string()];
        let pages = ["pages/a.html".to_string()];
        let components = ["Card".to_string()];
        assert_eq!(
            c.dangling(&pages, &components),
            vec![
                "R1 points at pages/gone.html, which the mock-up has not".to_string(),
                "R1 is tied to Ghost, which the mock-up has not".to_string()
            ]
        );
        c.requirements[0].pages.pop();
        c.requirements[0].components.pop();
        let seen = &c.dangling(&pages, &components);
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_session_is_told_the_design_system_s_variables_and_never_to_write_its_folders() {
        let css = ":root {\n  --color-text: #fff;\n  --space-1: 4px;\n}";
        let brief = design_brief(css);
        assert!(brief.contains("Never write anything under `mock/design/` or `mock/kit/`"));
        assert!(brief.contains("```css\n:root {\n  --color-text: #fff;"));
        assert!(design_brief(" ").contains("has no variable yet"));
        let seen = &examples_part(&[]);
        assert!(seen.is_empty(), "{seen:?}");
        let lent = [harness_architecture::kit::Example {
            file: "examples/a.png".into(),
            note: "the side nav".into(),
            scope: harness_architecture::kit::ExampleScope::App,
        }];
        let part = examples_part(&lent);
        assert!(part.contains(
            "- `mock/design/examples/a.png` — an example of the whole product: the side nav"
        ));
        assert!(part.contains("`example <file>: <what you took>`"));
        assert_eq!(
            examples_taken(
                "Done.\n- `example examples/a.png: the side nav`\nExample b.png: nothing\nexamples are fine\n* example c.png no colon"
            ),
            [
                "example examples/a.png: the side nav",
                "Example b.png: nothing"
            ]
        );
        let c = ready();
        assert!(
            componentize_prompt(&c, "R", "", css)
                .contains("Never write anything under `mock/design/`")
        );
        assert_eq!(
            token_names(css),
            vec!["--color-text".to_string(), "--space-1".to_string()]
        );
        let tokens = token_names(css);
        let page = "body { color: var(--color-text); background: var(--color-bg); } .g { --cols: 2; width: var(--cols); margin: var(--space-9) }";
        assert_eq!(
            unknown_tokens(page, &tokens),
            vec!["--color-bg".to_string(), "--space-9".to_string()]
        );
        let seen = &unknown_tokens("a { color: var(--color-text) }", &tokens);
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_page_s_regions_carry_ids_the_cut_keeps() {
        let c = ready();
        assert!(
            componentize_prompt(&c, "R", "", "")
                .contains("Keep every `id` a page carries on its region")
        );
    }

    #[test]
    fn a_rule_changed_takes_the_validation_back_the_same_rules_do_not() {
        let mut c = ready();
        c.requirements = vec![rule(1, "given y", &["pages/a.html"])];
        c.validate_rules().expect("valid");
        let same = c.requirements.clone();
        c.edit(Edit {
            requirements: Some(same.clone()),
            ..Edit::default()
        });
        assert!(c.rules_validated, "sent back unchanged");
        let mut changed = same;
        changed[0].family = Family::Business;
        c.edit(Edit {
            requirements: Some(changed),
            ..Edit::default()
        });
        assert!(!c.rules_validated);
        c.validate_rules().expect("valid");
        c.edit(Edit {
            requirements: Some(Vec::new()),
            ..Edit::default()
        });
        assert!(!c.rules_validated, "one deleted");
    }

    #[test]
    fn rules_read_off_a_cut_that_moved_are_stale() {
        let mut c = ready();
        c.componentized = Some("cut one".to_string());
        c.validate_rules().expect_err("no rule");
        c.add_requirements(vec![Read {
            title: "Pay".to_string(),
            family: Family::Business,
            criterion: crit("given"),
            ..Read::default()
        }]);
        assert_eq!(c.rules_for, "cut one");
        assert_eq!(c.requirements[0].family, Family::Business);
        assert_eq!(c.requirements[0].criterion, crit("given"));
        assert!(!c.rules_stale());
        c.componentized = Some("cut two".to_string());
        assert!(c.rules_stale());
        assert!(!c.rules_validated, "read again: to validate again");
        assert_eq!(mark(&c, Step::Rules), Mark::Open);
        c.rules_validated = true;
        assert_eq!(mark(&c, Step::Rules), Mark::Stale);
        assert_eq!(mark(&c, Step::Data), Mark::Stale, "drawn from the rules");
        c.rules_for.clear();
        assert!(!c.rules_stale(), "read before the cut was noted");
        c.rules_for = "cut one".to_string();
        c.requirements.clear();
        assert!(!c.rules_stale(), "nothing read: nothing to review");
    }

    #[test]
    fn a_rule_read_again_keeps_its_number_and_takes_the_reading() {
        let mut c = ready();
        c.requirements = vec![rule(4, "", &[]), rule(5, "kept", &[])];
        c.rules_validated = true;
        let read = Read {
            title: "Pay".to_string(),
            body: "pays".to_string(),
            pages: vec!["pages/a.html".to_string()],
            components: vec!["Card".to_string()],
            family: Family::Business,
            criterion: crit("a debt"),
        };
        c.reread(4, read).expect("known");
        let r = &c.requirements[0];
        assert_eq!(
            (
                r.id,
                r.title.as_str(),
                r.body.as_str(),
                r.criterion.given.as_str()
            ),
            (4, "Pay", "pays", "a debt")
        );
        assert_eq!(
            (r.pages.len(), r.components.len(), r.family),
            (1, 1, Family::Business)
        );
        assert_eq!(c.requirements[1].criterion, crit("kept"));
        assert!(!c.rules_validated);
        assert!(c.reread(9, Read::default()).is_err());
    }

    #[test]
    fn the_rules_prompt_asks_a_family_and_a_criterion_and_one_rule_alone() {
        let c = ready();
        let ctx = Context::default();
        let all = requirements_prompt(&c, &ctx);
        assert!(
            all.contains("`journey`")
                && all.contains("`business`")
                && all.contains("\"criterion\"")
        );
        let one = rule_prompt(
            &c,
            &ctx,
            &Requirement {
                title: "Pay an invoice".to_string(),
                body: "from\nthe list".to_string(),
                ..Requirement::default()
            },
        );
        assert!(one.starts_with(&all));
        assert!(one.contains("- title: Pay an invoice\n- what it asks for: from the list"));
    }

    #[test]
    fn a_rule_s_family_and_criterion_are_read_off_the_answer() {
        let answer = r#"{"requirements": [{"title": " Pay ", "family": "business", "criterion": " given a debt ", "pages": ["pages/a.html"]}]}"#;
        let read = parse_requirements(answer, &["pages/a.html".to_string()], &[]).expect("read");
        assert_eq!(read[0].family, Family::Business);
        assert_eq!(
            read[0].criterion.given, "given a debt",
            "a sentence that does not cut stays whole"
        );
        let parts = r#"{"requirements": [{"title": "Pay", "criterion": {"given": " a debt ", "when": "paid", "then": "settled"}}]}"#;
        let read = parse_requirements(parts, &[], &[]).expect("read");
        assert_eq!(
            read[0].criterion,
            Criterion {
                given: "a debt".to_string(),
                when: "paid".to_string(),
                then: "settled".to_string()
            }
        );
        let plain =
            parse_requirements(r#"{"requirements": [{"title": "Pay"}]}"#, &[], &[]).expect("read");
        assert_eq!(plain[0].family, Family::Journey, "a journey unless said");
    }

    #[test]
    fn ports_listed_from_rules_that_moved_are_stale_and_not_validated() {
        use crate::domain::ports_draft::{PortsDraft, database, ports_print};
        let mut c = ready();
        c.accepted = false;
        c.requirements = vec![Requirement {
            id: 1,
            title: "Pay".to_string(),
            ..Requirement::default()
        }];
        c.data = Some(DataDraft {
            from: rules_print(&c),
            validated: true,
            ..DataDraft::default()
        });
        assert_eq!(mark(&c, Step::Ports), Mark::Open);
        assert!(!c.ports_stale() && !c.ports_validated(), "none listed");
        c.ports = Some(PortsDraft {
            ports: vec![database()],
            from: ports_print(&c),
            validated: true,
        });
        assert!(c.ports_validated());
        c.accepted = true;
        assert_eq!(mark(&c, Step::Ports), Mark::Done);
        if let Some(data) = c.data.as_mut() {
            data.writes
                .insert("billing.pay".to_string(), vec!["line.total".to_string()]);
        }
        assert!(c.ports_stale());
        assert!(!c.ports_validated(), "validated, but from other writes");
        assert_eq!(
            mark(&c, Step::Data),
            Mark::Done,
            "the data model is not stale"
        );
        assert_eq!(mark(&c, Step::Ports), Mark::Stale);
        c.requirements[0].title = "Pay twice".to_string();
        assert!(c.ports_stale(), "the rules moved too");
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
        let cases: [(Spoil, Step, &str); 9] = [
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
            (
                |c| c.scope = "## Goal\n\nprove it".to_string(),
                Step::Design,
                "list at least one screen under Screens",
            ),
            (|c| c.mocked = None, Step::Rules, "draw the mock-ups"),
            (
                |c| c.rules_validated = false,
                Step::Data,
                "validate the rules",
            ),
            (
                |c| c.accepted = false,
                Step::Ports,
                "validate the data model",
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
        let mut modelled = ready();
        modelled.accepted = false;
        modelled.data = Some(DataDraft {
            from: rules_print(&modelled),
            validated: true,
            ..DataDraft::default()
        });
        assert_eq!(
            modelled.may_enter(Step::Initialize, DESIGNED),
            Err("accept the proof of concept".to_string())
        );
        assert_eq!(modelled.reachable(DESIGNED), Step::Ports);
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
            description: Some("told".to_string()),
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
        assert_eq!(c.description, "told");
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
        c.step = Step::Rules;
        c.regenerate();
        assert_eq!(c.step, Step::Brief);
        assert_eq!(
            (c.mocked.clone(), c.accepted, c.grilled),
            (None, false, false)
        );
        assert_eq!(c.regenerations, 1);
        assert_eq!(
            (c.brief.as_str(), c.scope.as_str(), c.questions.len()),
            ("a ledger", SCOPE, 1)
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
    fn the_decompose_prompt_carries_the_description_and_every_part() {
        let prompt = decompose_prompt("  a ledger for a bakery  ");
        assert!(prompt.ends_with("The description:\n\na ledger for a bakery\n"));
        assert!(prompt.contains(r#"{"what": "…", "who": "…", "journeys": "…", "data": "…"}"#));
        for p in &BRIEF_PARTS {
            assert!(prompt.contains(&format!("- `{}` — {} {}", p.id, p.title, p.hint)));
        }
    }

    #[test]
    fn a_brief_is_sorted_under_its_headings_the_empty_parts_left_out() {
        let answer = "Sorted:\n```json\n{\"who\": \" bakers \", \"what\": \"a ledger\", \"journeys\": \"  \", \"other\": \"x\"}\n```";
        assert_eq!(
            parse_brief(answer),
            Ok(
                "### What is the application, and what problem does it solve?\n\na ledger\n\n\
                ### Who uses it?\n\nbakers"
                    .to_string()
            )
        );
        assert_eq!(
            parse_brief(r#"{"what": " ", "data": ""}"#),
            Err("the AI found nothing to sort in the description".to_string())
        );
        assert_eq!(
            parse_brief("nothing"),
            Err("the AI answered no brief".to_string())
        );
        assert!(parse_brief(r#"{"what": 3}"#).is_err());
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
    fn the_scope_prompt_asks_each_section_by_its_key() {
        let prompt = scope_prompt(&ready());
        for s in &SCOPE_SECTIONS {
            assert!(prompt.contains(&format!("\"{}\": \"…\"", s.id)), "{}", s.id);
            assert!(prompt.contains(&format!("- `{}` — {}: {}", s.id, s.title, s.hint)));
        }
        assert!(prompt.contains("`screens` never is"));
        assert_eq!(
            SCOPE_SECTIONS
                .iter()
                .filter(|s| s.required)
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            [SCREENS]
        );
        assert_eq!(SCOPE_SECTIONS.last().map(|s| s.id), Some(NOTES));
    }

    #[test]
    fn a_scope_is_read_off_the_ai_s_sections_in_order() {
        let answer = r#"Here: {"proof": "it pays", "goal": "Pay", "screens": "- **Home** — the ledger\n- **Pay** — the form", "data": "  ", "notes": "plain\n### Tone\nshort", "ghost": "x"}"#;
        let scope = parse_scope(answer).expect("read");
        assert_eq!(
            scope,
            "## Goal\n\nPay\n\n## Screens\n\n- **Home** — the ledger\n- **Pay** — the form\n\n## Proof\n\nit pays\n\n## Notes\n\nplain\n### Tone\nshort"
        );
        let names: Vec<String> = crate::domain::drawing::screens(&scope)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["Home", "Pay"], "the drawing reads the screens back");
        assert_eq!(
            parse_scope(r#"{"goal": "Pay", "screens": " "}"#),
            Err("the AI's scope lists no screen".to_string())
        );
        assert_eq!(
            parse_scope("no json"),
            Err("the AI answered no scope".to_string())
        );
        assert_eq!(
            parse_scope(r#"{"goal": 1, "screens": "- **A**"}"#),
            Err("the AI's scope is not readable: `goal` is not text".to_string())
        );
        assert!(
            parse_scope("{oops}")
                .is_err_and(|why| why.starts_with("the AI's scope is not readable"))
        );
        assert!(parse_scope(r#"{"goal": null, "extra": [1], "screens": "- **A**"}"#).is_ok());
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
    fn a_body_that_fits_is_kept_and_a_long_one_folds_its_longest_sections_first() {
        let body = format!(
            "intro\n\n## Short\n\nshort\n\n## Long\n\n{}\n\n## Middle\n\n{}\n",
            "l".repeat(400),
            "m".repeat(200)
        );
        assert_eq!(fit_issue_body(&body, 10_000, "`x.md`"), body);
        let fitted = fit_issue_body(&body, 400, "`x.md`");
        assert!(fitted.chars().count() <= 400, "{fitted}");
        assert!(fitted.starts_with("intro\n\n## Short\n\nshort\n\n## Long\n\nToo long to stand in this issue (411 characters): read it whole in `x.md`."), "{fitted}");
        assert!(
            fitted.contains(&"m".repeat(200)),
            "the next longest stays while it fits"
        );
        // A preamble alone too long: the end is cut, the pointer kept.
        let cut = fit_issue_body(&"é".repeat(500), 100, "`x.md`");
        assert_eq!(cut.chars().count(), 100);
        assert!(cut.ends_with("read it whole in `x.md`.\n"), "{cut}");
    }

    proptest::proptest! {
        #[test]
        fn a_fitted_body_never_passes_the_limit(
            sections in proptest::collection::vec(("[a-z]{1,8}", "[a-zé \n]{0,300}"), 0..12),
            max in 120usize..2_000,
        ) {
            let body = sections.iter().fold(String::new(), |mut body, (h, t)| {
                let _ = write!(body, "## {h}\n\n{t}\n");
                body
            });
            let fitted = fit_issue_body(&body, max, "`mock/proof-of-concept.md`");
            proptest::prop_assert!(fitted.chars().count() <= max);
            if body.chars().count() <= max {
                proptest::prop_assert_eq!(fitted, body);
            }
        }
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
            family: Family::Business,
            criterion: Criterion::from_sentence(
                "given an invoice due, when the client pays, then it is settled",
            ),
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
                ..ComponentRef::default()
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
        let rule = "- **R3 Pay an invoice** (business) — The client pays from its list — criterion: given an invoice due; when the client pays; then it is settled — on `mock/pages/home.html`, `InvoiceList`";
        assert!(
            roadmap_record(&c, &ctx).contains(rule) && !body.contains(rule),
            "the record keeps every rule, the issue counts them"
        );
        assert!(body.contains("\n## Rules\n\n1 rule (0 journey, 1 business), carried by the rules' task, which writes them in `rules.json`"), "{body}");
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
        assert!(!bare.contains("## Rules") && !bare.contains("## Requests left"));
        let json = record_json(&c, &ctx).expect("written");
        let read: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(read["requirements"][0]["title"], "Pay an invoice");
        assert_eq!(read["components"][0]["needs"][0], "billing.listInvoices");
        assert_eq!(read["decisions"][0]["answer"], "yes");
        assert_eq!(read["mockup_at"], "mockup/proof-of-concept");
        // Every write, with what the guard and the data step decided of it.
        assert_eq!(read["writes"][0]["dataCapability"], "billing.payInvoice");
        assert!(read["writes"][0]["effect"].is_null(), "no audit made");
        let mut audited = c.clone();
        audited.audit = Some(crate::domain::audit::Audit {
            effects: std::collections::BTreeMap::from([(
                "billing.payInvoice".to_string(),
                crate::domain::audit::Effect::Update,
            )]),
            ..crate::domain::audit::Audit::default()
        });
        audited.data = Some(crate::domain::data_draft::DataDraft {
            writes: std::collections::BTreeMap::from([(
                "billing.payInvoice".to_string(),
                vec!["invoice.paid_at".to_string()],
            )]),
            ..crate::domain::data_draft::DataDraft::default()
        });
        let writes = writes_of(&audited, &ctx);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].effect, Some(crate::domain::audit::Effect::Update));
        assert_eq!(writes[0].touches, ["invoice.paid_at"]);
        assert!(
            read["components"][0].get("markup").is_none(),
            "never written"
        );
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
                ..ComponentRef::default()
            }],
            ..Context::default()
        };
        let prompt = requirements_prompt(&c, &ctx);
        assert!(prompt.contains("- `pages/home.html` — Home — see the invoices"));
        assert!(prompt.contains(
            "- `InvoiceList` — billing — the invoices — billing.listInvoices — billing.payInvoice"
        ));
        assert!(prompt.contains("\"ties\": [{\"id\": <its number>"));
        assert!(prompt.contains(".\n\n- R1: Mine already\n"));
        assert!(!prompt.contains("- The AI's"), "only the human's are kept");
        let bare = requirements_prompt(&ready(), &ctx);
        assert!(!bare.contains("do not repeat them"));
    }

    #[test]
    fn the_human_s_rules_are_tied_again_to_the_mock_up_as_it_stands() {
        let pages = vec!["pages/home.html".to_string()];
        let components = vec!["CampaignList".to_string()];
        let answer = r#"{"requirements": [], "ties": [
            {"id": 1, "pages": ["pages/home.html", "pages/gone.html"], "components": ["CampaignList", "Gone"]},
            {"id": 2, "pages": ["pages/gone.html"], "components": []},
            {"id": 3, "components": ["CampaignList"]}]}"#;
        let ties = parse_ties(answer, &pages, &components);
        assert_eq!(ties.len(), 2, "a tie to nothing the mock-up has is none");
        assert_eq!(ties[0].pages, pages);
        assert_eq!(ties[0].components, components);
        assert_eq!(ties[1].id, 3);
        let seen = &parse_ties("no object", &pages, &components);
        assert!(seen.is_empty(), "{seen:?}");
        let mut c = ready();
        let mut mine = rule(1, "given", &["pages/old.html"]);
        mine.human = true;
        mine.components = vec!["OldList".to_string()];
        let ais = rule(3, "given", &["pages/old.html"]);
        c.requirements = vec![mine, ais];
        c.retie(&ties);
        assert_eq!(c.requirements[0].pages, pages);
        assert_eq!(c.requirements[0].components, components);
        assert_eq!(
            c.requirements[1].pages,
            ["pages/old.html"],
            "only the human's are tied again"
        );
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
    fn a_manifest_repair_names_what_does_not_read_and_touches_nothing_else() {
        let text = manifest_repair_prompt("RULES", " mock/pages.json: invalid type \n");
        assert!(
            text.starts_with("RULES"),
            "the rules first: the shape is in them"
        );
        assert!(text.contains("`mock/pages.json: invalid type`"), "{text}");
        assert!(text.contains("Change no other file"), "{text}");
    }

    #[test]
    fn a_drawing_keeps_its_scope_and_is_stale_once_the_scope_moves() {
        let mut c = Creation {
            scope: "## Goal\nfirst".to_string(),
            componentized: Some("earlier".to_string()),
            ..Creation::default()
        };
        assert!(!c.mockup_stale(), "nothing drawn: nothing stale");
        c.drawn("two pages".to_string());
        assert_eq!(c.mocked.as_deref(), Some("two pages"));
        assert_eq!(c.componentized, None, "a new drawing is not cut yet");
        assert!(!c.mockup_stale());
        c.scope = "  ## Goal\nfirst\n".to_string();
        assert!(!c.mockup_stale(), "spaces around do not move it");
        c.scope = "## Goal\nsecond".to_string();
        assert!(c.mockup_stale());
        c.mocked_scope.clear();
        assert!(
            !c.mockup_stale(),
            "a record drawn before the scope was kept"
        );
        c.mocked_scope = "## Goal\nfirst".to_string();
        c.regenerate();
        assert!(!c.mockup_stale() && c.mocked_scope.is_empty());
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
    fn the_cut_repair_prompt_lists_each_gap_after_the_rules() {
        let text = cut_repair_prompt(
            "RULES",
            &[
                " `Home` has no fragment ".to_string(),
                "in `mock/x`".to_string(),
            ],
        );
        assert!(text.starts_with("RULES\nThe factory's gates refuse"));
        assert!(
            text.contains("\n- `Home` has no fragment\n- in `mock/x`\n"),
            "{text}"
        );
        assert!(text.contains("data-writes=\"<DataCapability>\""));
    }

    #[test]
    fn the_componentize_prompt_keeps_the_look_and_carries_the_kit_and_the_brief() {
        let c = ready();
        let text = componentize_prompt(
            &c,
            "RULES",
            "Button: a button",
            ":root { --color-text: #fff; }",
        );
        assert!(text.starts_with("RULES"));
        assert!(text.contains("without changing what the human sees"));
        assert!(text.contains("Button: a button"));
        assert!(text.contains("a ledger"));
        assert!(!componentize_prompt(&c, "RULES", "  ", "").contains("The kit's components"));
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

    fn asked(topic: &str, title: &str) -> Asked {
        Asked {
            topic: topic.to_string(),
            title: title.to_string(),
            ..Asked::default()
        }
    }

    #[test]
    fn a_round_files_its_questions_under_topics_one_spelling_each() {
        let answer = "{\"questions\": [{\"topic\": \" Combat \", \"title\": \"Initiative\"}, \
                      {\"title\": \"One-shots\"}]}";
        let round = parse_round(answer).expect("read");
        let topics: Vec<&str> = round.questions.iter().map(|q| q.topic.as_str()).collect();
        assert_eq!(topics, ["Combat", ""]);
        let mut c = ready();
        c.add_round(round);
        c.add_round(Round {
            questions: vec![asked("combat", "Tokens"), asked("Campaign", "Calendar")],
            done: false,
        });
        let topics: Vec<&str> = c.questions.iter().map(|q| q.topic.as_str()).collect();
        assert_eq!(topics, ["", "Combat", "", "Combat", "Campaign"]);
        assert_eq!(c.topics(), ["Combat", "Campaign"]);
    }

    #[test]
    fn a_question_kept_before_topics_reads_as_the_general_one() {
        let q: Question = serde_json::from_str("{\"id\": 4, \"title\": \"Who\"}").expect("read");
        assert_eq!(q.topic, "");
        let seen = &ready().topics();
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn the_grilling_prompt_names_the_topics_in_use_and_forbids_overlaps() {
        let mut c = ready();
        assert!(!grill_prompt(&c).contains("Topics in use"));
        c.questions[0].topic = "Combat".to_string();
        c.questions.push(Question {
            topic: "Campaign".to_string(),
            ..question(2, "")
        });
        let grill = grill_prompt(&c);
        assert!(grill.contains("- [Combat] **q1** — yes"));
        assert!(grill.contains("do not ask them again:\n\n- [Campaign] q2"));
        assert!(grill.contains("Topics in use: Combat, Campaign."));
        assert!(grill.contains("open a new topic when a round reaches"));
        assert!(grill.contains("never the same decision, or one overlapping it, under two topics"));
        assert!(grill.contains("\"topic\""));
    }

    #[test]
    fn the_roadmap_issue_sorts_the_decisions_by_topic_the_general_ones_first() {
        let mut c = ready();
        c.questions.push(Question {
            topic: "Combat".to_string(),
            ..question(2, "no")
        });
        c.questions.push(question(3, "solo"));
        c.questions.push(Question {
            topic: "Campaign".to_string(),
            ..question(4, " ")
        });
        let (_, body) = roadmap_issue(&c, &Context::default());
        assert!(body.contains(
            "## Decisions\n\n### General\n\n- **q1** — yes\n- **q3** — solo\n\n### Combat\n\n- **q2** — no\n"
        ));
        assert!(
            !body.contains("### Campaign"),
            "a topic with no decision taken"
        );
        let (_, flat) = roadmap_issue(&ready(), &Context::default());
        assert!(flat.contains("## Decisions\n\n- **q1** — yes\n"));
        assert!(!flat.contains("###"));
    }

    proptest! {
        #[test]
        fn proposals_read_off_any_answer_never_panic(answer in ".{0,200}") {
            let _ = parse_proposals(&answer, &[], &[]);
        }

        #[test]
        fn a_scope_keeps_every_section_said_in_order(texts in proptest::collection::vec("[a-z ]{0,20}", 9)) {
            let mut object: HashMap<&str, String> =
                SCOPE_SECTIONS.iter().map(|s| s.id).zip(texts.iter().cloned()).collect();
            object.insert(SCREENS, "- **Home** — the ledger".to_string());
            let scope = parse_scope(&serde_json::to_string(&object).expect("json")).expect("read");
            let said: Vec<String> = SCOPE_SECTIONS
                .iter()
                .filter_map(|s| {
                    let text = object.get(s.id).map_or("", |t| t.trim());
                    (!text.is_empty()).then(|| format!("## {}\n\n{text}", s.title))
                })
                .collect();
            prop_assert_eq!(scope, said.join("\n\n"));
        }

        #[test]
        fn a_sorted_brief_keeps_every_part_said_in_order(parts in proptest::collection::vec(".{0,40}", 4)) {
            let object: HashMap<&str, &String> = BRIEF_PARTS.iter().map(|p| p.id).zip(&parts).collect();
            let brief = parse_brief(&serde_json::to_string(&object).expect("json"));
            let said: Vec<String> = BRIEF_PARTS
                .iter()
                .zip(&parts)
                .filter(|(_, t)| !t.trim().is_empty())
                .map(|(p, t)| format!("### {}\n\n{}", p.title, t.trim()))
                .collect();
            if said.is_empty() {
                prop_assert!(brief.is_err());
            } else {
                prop_assert_eq!(brief, Ok(said.join("\n\n")));
            }
        }

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
        fn a_topic_reads_back_trimmed(topic in "[a-zA-Z ]{0,20}") {
            let answer = format!(
                "{{\"questions\": [{{\"topic\": {}, \"title\": \"q\"}}]}}",
                serde_json::to_string(&topic).expect("json")
            );
            let round = parse_round(&answer).expect("read");
            prop_assert_eq!(round.questions.first().map(|q| q.topic.clone()), Some(topic.trim().to_string()));
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
