//! Un `GitHub` en mémoire, partagé par les workflows qui en ont besoin pour
//! leurs tests.
//!
//! Un faux adaptateur, pas un mock : il répond depuis des `Issue`/`Pr` qu'on
//! lui a données, et les écritures se relisent. C'est ce que `CLAUDE.md`
//! demande — « inject a fake adapter; nothing mocks at the call site » — et
//! c'est ce qui permet d'exercer les règles sans réseau.

use std::cell::RefCell;
use std::path::Path;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Issue, Outcome, Pr};

/// Ce que le faux a enregistré comme écriture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wrote {
    /// Une étiquette posée.
    Label(u64, String),
    /// Une étiquette retirée.
    Unlabelled(u64, String),
    /// Un corps réécrit.
    Body(u64, String),
    /// Un commentaire posté sur une issue.
    Comment(u64, String),
    /// Une issue fermée.
    Closed(u64),
    /// Un commentaire posté sur une PR, et le fichier dont il vient.
    PrComment(String, String),
}

/// Un GitHub en mémoire.
#[derive(Default)]
pub struct FakeGitHub {
    /// Les issues qu'il connaît, milestones compris.
    pub issues: Vec<Issue>,
    /// Les sous-issues, par numéro de parent.
    pub subs: Vec<(u64, Vec<Issue>)>,
    /// Les PR mergées qu'il rendra pour `merged_prs`.
    pub merged: Vec<Issue>,
    /// Les étiquettes que le dépôt porte.
    pub labels: Vec<String>,
    /// Les commentaires d'issue qu'il rendra pour `issue_comments`.
    pub issue_comments: Vec<String>,
    /// Les PR que `pr()` sait lire, par numéro ou URL demandé.
    pub prs: Vec<(String, Pr)>,
    /// Ce que `pr_comments` rend, par numéro de PR.
    pub pr_comment_bodies: Vec<(String, String)>,
    /// Ce qu'il a écrit.
    pub wrote: RefCell<Vec<Wrote>>,
    /// Quand c'est rempli, **toute** lecture échoue avec cet arrêt.
    pub broken: Option<Halt>,
}

impl FakeGitHub {
    fn ok(&self) -> Outcome<()> {
        self.broken
            .as_ref()
            .map_or(Ok(()), |halt| Err(halt.clone()))
    }

    /// Les écritures, dans l'ordre.
    pub fn writes(&self) -> Vec<Wrote> {
        self.wrote.borrow().clone()
    }
}

#[async_trait(?Send)]
impl GitHub for FakeGitHub {
    async fn authenticated(&self) -> Outcome<bool> {
        self.ok()?;
        Ok(true)
    }

    async fn repo(&self) -> Outcome<String> {
        self.ok()?;
        Ok("owner/repo".to_string())
    }

    async fn labels(&self) -> Outcome<Vec<String>> {
        self.ok()?;
        Ok(self.labels.clone())
    }

    async fn issue(&self, number: u64) -> Outcome<Issue> {
        self.ok()?;
        self.issues
            .iter()
            .find(|issue| issue.number == number)
            .cloned()
            .ok_or_else(|| Halt::Unreadable(format!("pas d'issue #{number}")))
    }

    async fn issues_labelled(&self, label: &str, _state: &str) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(self
            .issues
            .iter()
            .filter(|issue| issue.has(label))
            .cloned()
            .collect())
    }

    async fn sub_issues(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(self
            .subs
            .iter()
            .find(|(parent, _)| *parent == number)
            .map(|(_, subs)| subs.clone())
            .unwrap_or_default())
    }

    async fn blocked_by(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.ok()?;
        // Les bloqueurs sont déjà portés par l'issue dans le faux : ce
        // qu'on veut exercer est la règle, pas la forme de l'API.
        Ok(self
            .subs
            .iter()
            .flat_map(|(_, subs)| subs.iter())
            .find(|issue| issue.number == number)
            .map(|issue| issue.blocked_by.clone())
            .unwrap_or_default())
    }

    async fn with_blockers(&self, tasks: Vec<Issue>) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(tasks)
    }

    async fn merged_prs(&self, _base: &str) -> Outcome<Vec<Issue>> {
        self.ok()?;
        Ok(self.merged.clone())
    }

    async fn issue_comments(&self, _number: u64) -> Outcome<Vec<String>> {
        self.ok()?;
        Ok(self.issue_comments.clone())
    }

    async fn add_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Label(number, label.to_string()));
        Ok(())
    }

    async fn remove_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Unlabelled(number, label.to_string()));
        Ok(())
    }

    async fn set_body(&self, number: u64, body: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Body(number, body.to_string()));
        Ok(())
    }

    async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()> {
        self.wrote
            .borrow_mut()
            .push(Wrote::Comment(number, body.to_string()));
        Ok(())
    }

    async fn close_issue(&self, number: u64) -> Outcome<()> {
        self.wrote.borrow_mut().push(Wrote::Closed(number));
        Ok(())
    }

    async fn pr(&self, pr_ref: &str) -> Outcome<Pr> {
        self.ok()?;
        self.prs
            .iter()
            .find(|(asked, _)| asked == pr_ref)
            .map(|(_, pr)| pr.clone())
            .ok_or_else(|| Halt::Failed(format!("pas de PR {pr_ref}")))
    }

    async fn pr_comments(&self, num: &str) -> Outcome<String> {
        self.ok()?;
        Ok(self
            .pr_comment_bodies
            .iter()
            .find(|(pr, _)| pr == num)
            .map(|(_, body)| body.clone())
            .unwrap_or_default())
    }

    async fn post_pr_comment(&self, num: &str, body_file: &Path) -> Outcome<()> {
        self.ok()?;
        self.wrote.borrow_mut().push(Wrote::PrComment(
            num.to_string(),
            body_file.display().to_string(),
        ));
        Ok(())
    }
}
