//! Le tableau : le milestone en cours, ses tasks, et laquelle vient ensuite.
//!
//! Le côté **lecture** du modèle en issues, à un seul endroit. Le round en a
//! besoin pour choisir, le préflight pour vérifier que le tableau est
//! atteignable, un rapport d'état pour dire où on en est. Plusieurs lecteurs,
//! une seule façon de lire.
//!
//! Ici et pas dans `harness-core::adapters` : composer « le milestone ouvert de
//! plus petit numéro, puis ses sous-issues, puis leurs bloqueurs » est une
//! politique, et un adaptateur ne décide de rien.
//!
//! **Le `Board` est de la donnée pure**, à la différence du Python, où il
//! portait le client `gh` pour éviter d'en rouvrir un. En Rust le client se
//! passe, et un tableau sans client se teste en le construisant à la main.

use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Issue, Outcome};

use crate::common::labels;
use crate::dev_loop::tasks;

/// Ce que la boucle voit du tableau, à un instant donné.
#[derive(Debug, Clone, Default)]
pub struct Board {
    /// Le milestone en cours.
    pub milestone: Issue,
    /// Ses sous-issues, bloqueurs compris.
    pub tasks: Vec<Issue>,
    /// La task que le point de reprise désigne, quand il y en a une.
    ///
    /// Elle **prime** sur le choix du tableau, et c'est ce qui permet de finir
    /// un round dont le `/code` a déjà mergé : l'issue est fermée, donc plus
    /// rien ne l'offrirait, et `/create-test` serait perdu.
    pub resuming: Option<Issue>,
}

impl Board {
    /// Les tasks d'agent encore ouvertes — ce qui décide du rollover.
    #[must_use]
    pub fn open_agents(&self) -> Vec<&Issue> {
        tasks::open_agent_tasks(&self.tasks)
    }

    /// La task suivante selon les quatre règles, s'il y en a une.
    #[must_use]
    pub fn next(&self) -> Option<&Issue> {
        tasks::next_task(&self.tasks)
    }

    /// La sous-issue de ce milestone qui porte ce numéro, **ouverte ou non**.
    ///
    /// Fermée comprise, à dessein : un round interrompu après le merge de
    /// `/code` doit pouvoir se terminer, et l'issue qu'il finit est déjà
    /// fermée.
    #[must_use]
    pub fn find(&self, key: &str) -> Option<&Issue> {
        let number: u64 = key.parse().ok()?;
        self.tasks.iter().find(|task| task.number == number)
    }

    /// Le message d'arrêt quand il reste des tasks mais aucune jouable.
    ///
    /// Ce cas n'est **pas** un rollover, et les confondre coûte un run opus :
    /// le milestone n'est pas fini, il attend un humain. Reste à dire **lequel**
    /// des gestes il attend, parce qu'ils n'ont rien à voir — tout livrer et
    /// attendre une fusion n'a pas la même réponse qu'une case `harness:ready`
    /// que personne n'a cochée.
    #[must_use]
    pub fn stuck(&self) -> String {
        let open = self.open_agents();
        let head = format!(
            "milestone {} has {} open task(s) but none can run:\n{}\n",
            self.milestone.reference(),
            open.len(),
            tasks::stuck_report(&self.tasks)
        );
        if !open.is_empty() && open.iter().all(|task| tasks::waiting_merge(task)) {
            return head
                + "Everything is delivered on the integration branch and \
                   waiting for you to merge it and close these issues. Nothing \
                   here is the harness's to do.";
        }
        head + &format!(
            "Add {} to the one to work on next, or close what blocks it, then \
             re-run.",
            labels::READY
        )
    }
}

/// Le tableau, lu maintenant.
///
/// # Errors
///
/// - [`Halt::Halted`] s'il n'y a aucun milestone ouvert : il n'y a rien sur
///   quoi travailler, et le dire nomme le geste qui débloque ;
/// - l'échec de l'adaptateur, propagé tel quel. Une lecture qui n'aboutit pas
///   ne se rend **jamais** en tableau vide, qui se lirait « milestone terminé »
///   et déclencherait `/planner`.
pub async fn read(gh: &dyn GitHub) -> Outcome<Board> {
    let found = gh.issues_labelled(labels::MILESTONE, "open").await?;
    let Some(milestone) = tasks::current_milestone(&found) else {
        return Err(Halt::Halted(format!(
            "no open {} issue — there is nothing to work from. Open one (or let \
             /planner open one from a {} issue) before running the harness.",
            labels::MILESTONE,
            labels::ROADMAP
        )));
    };
    let milestone = milestone.clone();
    let subs = gh.sub_issues(milestone.number).await?;
    let held = gh.with_blockers(subs).await?;
    Ok(Board {
        milestone,
        tasks: held,
        resuming: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    #[tokio::test]
    async fn the_board_reads_the_lowest_open_milestone_and_its_tasks() {
        let gh = FakeGitHub {
            issues: vec![
                issue(9, &[labels::MILESTONE]),
                issue(4, &[labels::MILESTONE]),
            ],
            subs: vec![(4, vec![task(11), task(12)])],
            ..FakeGitHub::default()
        };
        let board = read(&gh).await.expect("un tableau");
        assert_eq!(board.milestone.number, 4);
        assert_eq!(board.next().expect("une task").number, 11);
    }

    #[tokio::test]
    async fn no_open_milestone_halts_and_names_the_gesture() {
        let gh = FakeGitHub::default();
        let err = read(&gh).await.expect_err("doit s'arrêter");
        let Halt::Halted(said) = err else {
            panic!("un arrêt volontaire, pas une panne");
        };
        assert!(said.contains(labels::MILESTONE));
        assert!(said.contains("/planner"), "dire quoi faire");
    }

    #[tokio::test]
    async fn an_unreadable_read_propagates_and_never_becomes_an_empty_board() {
        // Le mode de panne que ça évite : un tableau vide se lit « milestone
        // terminé » et déclenche un /planner payant.
        let gh = FakeGitHub {
            broken: Some(Halt::Unreadable("jeton expiré".to_string())),
            ..FakeGitHub::default()
        };
        let err = read(&gh).await.expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn find_returns_a_closed_task_so_an_interrupted_round_can_finish() {
        let mut done = task(11);
        done.state = "closed".to_string();
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![done],
            resuming: None,
        };
        // Le round repris travaille sur une issue déjà fermée par le merge de
        // /code : l'exclure ferait perdre /create-test.
        assert_eq!(board.find("11").expect("trouvée").number, 11);
        assert!(board.find("99").is_none());
        assert!(board.find("pas un nombre").is_none());
    }

    #[test]
    fn stuck_tells_you_to_merge_when_everything_is_delivered() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])],
            resuming: None,
        };
        let said = board.stuck();
        assert!(said.contains("waiting for you to merge"));
        assert!(!said.contains(labels::READY), "ce n'est pas le geste ici");
    }

    #[test]
    fn stuck_tells_you_what_to_tick_when_nothing_is_ready() {
        let board = Board {
            milestone: issue(4, &[labels::MILESTONE]),
            tasks: vec![issue(11, &[labels::AGENT])],
            resuming: None,
        };
        let said = board.stuck();
        assert!(said.contains(labels::READY));
        assert!(!said.contains("waiting for you to merge"));
    }
}
