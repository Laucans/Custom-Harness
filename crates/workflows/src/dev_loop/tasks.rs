//! Ce qu'est une task pour ce round, et laquelle vient ensuite.
//!
//! La lecture que **ce workflow** fait d'une `Issue` : ses sept étiquettes, les
//! questions qu'elles permettent de poser, et les quatre règles qui choisissent
//! la task suivante. On lui passe des `Issue` déjà lues, il rend celle qui peut
//! tourner — c'est ce qui rend les règles testables sans réseau et sans double.
//!
//! Ici et pas dans `harness-core` : `harness:ready` ne veut rien dire pour une
//! issue en général, seulement pour ce round. Le core porte la forme d'une
//! issue, ce module porte ce que ce workflow-ci en fait.
//!
//! # Les quatre règles, et ce que chacune empêche
//!
//! 1. **le milestone en cours** est l'issue `harness:milestone` ouverte de plus
//!    petit numéro. Un ordre total, pour qu'un second milestone ouvert par
//!    mégarde ne rende pas le choix dépendant de l'ordre de l'API ;
//! 2. **la task suivante** est une issue `harness:agent` ouverte, sous-issue de
//!    ce milestone, portant `harness:ready`, et dont **tous** les `blocked_by`
//!    sont fermés. Un vrai parcours de dépendances, pas « l'issue N-1
//!    est-elle fermée » : le parallélisme est prévu, et il ne doit pas demander
//!    une migration de données pour arriver ;
//! 3. **une issue `harness:human` n'est pas un mécanisme à part.** Elle bloque
//!    parce qu'elle est dans les `blocked_by`, comme n'importe quelle autre
//!    dépendance — la porte humaine du round a disparu dans cette règle-ci ;
//! 4. **`harness:ready` commande tout.** Une task ouverte mais pas prête arrête
//!    le run ; elle ne le fait surtout pas basculer en rollover, qui dépense un
//!    run opus pour ouvrir un item de roadmap que personne n'a demandé.
//!
//! Métier pur : ni I/O, ni subprocess.

use harness_core::domain::Issue;

use crate::common::labels;

/// Vrai si l'issue est une task que l'agent peut faire tourner.
#[must_use]
pub fn is_agent(issue: &Issue) -> bool {
    issue.has(labels::AGENT)
}

/// Vrai si l'issue est une action que seul l'humain peut faire.
#[must_use]
pub fn is_human(issue: &Issue) -> bool {
    issue.has(labels::HUMAN)
}

/// Vrai si l'humain a coché la case.
#[must_use]
pub fn is_ready(issue: &Issue) -> bool {
    issue.has(labels::READY)
}

/// Vrai si le SPEC est déjà écrit dans le corps.
#[must_use]
pub fn spec_written(issue: &Issue) -> bool {
    issue.has(labels::SPEC_WRITTEN)
}

/// Vrai si la task est livrée et attend une fusion.
#[must_use]
pub fn waiting_merge(issue: &Issue) -> bool {
    issue.has(labels::WAITING_MERGE)
}

/// `human` ou `auto` — comment le journal nomme la sorte de task.
#[must_use]
pub fn kind(issue: &Issue) -> &'static str {
    if is_human(issue) { "human" } else { "auto" }
}

/// Les bloqueurs qui bloquent **encore**.
///
/// Une task en `waiting-merge` n'en fait pas partie : son code est sur la
/// branche d'intégration, donc la suivante peut bâtir dessus. Sans cette
/// exception la chaîne s'arrêterait après une seule task, et il faudrait une
/// fusion dans `main` par round.
///
/// Une action humaine n'a pas cet état — elle ne se livre pas sur une branche,
/// elle se ferme.
#[must_use]
pub fn blockers_pending(issue: &Issue) -> Vec<&Issue> {
    issue
        .blocked_by
        .iter()
        .filter(|blocker| blocker.is_open() && !waiting_merge(blocker))
        .collect()
}

/// Ouverte, prête, pas déjà livrée, et plus rien qui la bloque.
///
/// `waiting_merge` est ce qui empêche de rejouer une task finie : elle reste
/// ouverte jusqu'à la fusion, donc sans ce test le round suivant la choisirait
/// et repayerait ses trois stages.
#[must_use]
pub fn runnable(issue: &Issue) -> bool {
    issue.is_open()
        && is_agent(issue)
        && is_ready(issue)
        && !waiting_merge(issue)
        && blockers_pending(issue).is_empty()
}

/// Pourquoi cette task ne peut pas tourner, dit à un humain.
///
/// Un run qui s'arrête doit nommer le geste qui le débloque. « aucune task
/// prête » n'en nomme aucun ; « #12 attend #11 (ouverte) » en nomme un.
#[must_use]
pub fn why_not(issue: &Issue) -> String {
    if issue.is_closed() {
        return format!("{} is closed", issue.reference());
    }
    if waiting_merge(issue) {
        return format!(
            "{} {}: delivered on the integration branch, waiting for the merge \
             that closes it",
            issue.reference(),
            issue.title
        );
    }
    let mut reasons = Vec::new();
    if !is_ready(issue) {
        reasons.push(format!("no {} label", labels::READY));
    }
    for blocker in blockers_pending(issue) {
        let said = if is_human(blocker) {
            "human action"
        } else {
            "task"
        };
        reasons.push(format!(
            "blocked by {} ({said}, still open)",
            blocker.reference()
        ));
    }
    if reasons.is_empty() {
        reasons.push("ready".to_string());
    }
    format!(
        "{} {}: {}",
        issue.reference(),
        issue.title,
        reasons.join(", ")
    )
}

/// Règle 1 : le `harness:milestone` ouvert de plus petit numéro.
#[must_use]
pub fn current_milestone(issues: &[Issue]) -> Option<&Issue> {
    issues
        .iter()
        .filter(|issue| issue.is_open() && issue.has(labels::MILESTONE))
        .min_by_key(|issue| issue.number)
}

/// Les sous-issues qui sont des tasks d'agent, dans l'ordre des numéros.
#[must_use]
pub fn agent_tasks(issues: &[Issue]) -> Vec<&Issue> {
    let mut found: Vec<&Issue> = issues.iter().filter(|i| is_agent(i)).collect();
    found.sort_by_key(|issue| issue.number);
    found
}

/// Les tasks d'agent encore ouvertes — ce qui décide du rollover.
#[must_use]
pub fn open_agent_tasks(issues: &[Issue]) -> Vec<&Issue> {
    agent_tasks(issues)
        .into_iter()
        .filter(|issue| issue.is_open())
        .collect()
}

/// Règle 2 : la plus petite task ouverte, prête et débloquée.
#[must_use]
pub fn next_task(issues: &[Issue]) -> Option<&Issue> {
    open_agent_tasks(issues)
        .into_iter()
        .filter(|issue| runnable(issue))
        .min_by_key(|issue| issue.number)
}

/// Pourquoi aucune task ne peut tourner, task par task.
///
/// Le cas que ce texte existe pour ne pas confondre avec le rollover : il reste
/// des tasks ouvertes, donc le milestone n'est pas fini, donc appeler
/// `/planner` serait payer un run opus pour ouvrir un milestone de plus.
#[must_use]
pub fn stuck_report(issues: &[Issue]) -> String {
    open_agent_tasks(issues)
        .into_iter()
        .map(|issue| format!("  - {}", why_not(issue)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Le corps de PR déclare-t-il fermer `#number` ?
///
/// **Exige la ligne entière**, alors que GitHub accepte le mot-clé n'importe où
/// dans le corps. Volontairement plus strict, et dans le bon sens : `/code` a
/// pour consigne de le mettre sur sa propre ligne, et lire plus large ferait
/// fermer une issue sur une phrase qui la mentionne. Rater une fermeture que
/// GitHub aurait faite est sans conséquence — l'issue est alors déjà fermée, et
/// on n'arrive jamais jusqu'ici.
///
/// Écrit à la main plutôt qu'en regex : trois mots-clés et un numéro ne valent
/// pas une dépendance, et la lecture ligne à ligne dit exactement la règle.
#[must_use]
pub fn closes(body: &str, number: u64) -> bool {
    body.lines()
        .any(|line| closes_on_its_own_line(line, number))
}

fn closes_on_its_own_line(line: &str, number: u64) -> bool {
    let trimmed = line.trim_matches(|c| c == ' ' || c == '\t');
    let lowered = trimmed.to_lowercase();
    for keyword in ["closes", "fixes", "resolves"] {
        let Some(rest) = lowered.strip_prefix(keyword) else {
            continue;
        };
        // Au moins une espace ou tabulation entre le mot-clé et le `#`.
        let rest = rest.trim_start_matches([' ', '\t']);
        if rest.len() == lowered.len() - keyword.len() {
            continue;
        }
        let Some(digits) = rest.strip_prefix('#') else {
            continue;
        };
        if digits.parse::<u64>() == Ok(number) {
            return true;
        }
    }
    false
}

/// La première PR de la liste qui déclare fermer `#number`.
///
/// Ici et non dans l'adaptateur : `closes` est la convention donnée à `/code`,
/// pas une propriété de l'API GitHub. L'adaptateur rend les PR mergées, ce
/// module décide laquelle vaut preuve de livraison.
#[must_use]
pub fn first_closing(prs: &[Issue], number: u64) -> Option<&Issue> {
    prs.iter().find(|pr| closes(&pr.body, number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    fn closed(mut issue: Issue) -> Issue {
        issue.state = "closed".to_string();
        issue
    }

    // --- règle 1 : le milestone --------------------------------------------

    #[test]
    fn the_current_milestone_is_the_lowest_numbered_open_one() {
        // Un ordre total : un second milestone ouvert par mégarde ne doit pas
        // rendre le choix dépendant de l'ordre de l'API.
        let issues = vec![
            issue(9, &[labels::MILESTONE]),
            issue(4, &[labels::MILESTONE]),
            issue(7, &[labels::MILESTONE]),
        ];
        assert_eq!(current_milestone(&issues).expect("un milestone").number, 4);
    }

    #[test]
    fn a_closed_milestone_is_not_the_current_one() {
        let issues = vec![
            closed(issue(4, &[labels::MILESTONE])),
            issue(9, &[labels::MILESTONE]),
        ];
        assert_eq!(current_milestone(&issues).expect("un milestone").number, 9);
    }

    #[test]
    fn no_milestone_at_all_is_none_rather_than_a_guess() {
        assert!(current_milestone(&[issue(1, &[labels::AGENT])]).is_none());
    }

    // --- règle 2 : la task suivante ----------------------------------------

    #[test]
    fn the_next_task_is_the_lowest_numbered_runnable_one() {
        let issues = vec![task(12), task(7), task(20)];
        assert_eq!(next_task(&issues).expect("une task").number, 7);
    }

    #[test]
    fn a_task_blocked_by_an_open_issue_is_not_runnable() {
        let mut blocked = task(12);
        blocked.blocked_by = vec![issue(11, &[labels::AGENT])];
        assert!(!runnable(&blocked));
        assert!(next_task(&[blocked]).is_none());
    }

    #[test]
    fn a_task_whose_blockers_are_all_closed_is_runnable() {
        let mut freed = task(12);
        freed.blocked_by = vec![closed(issue(11, &[labels::AGENT]))];
        assert!(runnable(&freed));
    }

    #[test]
    fn a_dependency_chain_is_walked_not_assumed_from_numbering() {
        // Le parallélisme est prévu : #20 peut partir avant #12 si c'est #12
        // qui est bloquée. « l'issue N-1 est-elle fermée » donnerait #12.
        let mut blocked = task(12);
        blocked.blocked_by = vec![issue(11, &[labels::AGENT])];
        let issues = vec![blocked, task(20)];
        assert_eq!(next_task(&issues).expect("une task").number, 20);
    }

    // --- règle 3 : l'humain bloque par dépendance --------------------------

    #[test]
    fn a_human_issue_blocks_through_the_dependency_not_a_mechanism_of_its_own() {
        let mut waiting = task(12);
        waiting.blocked_by = vec![issue(11, &[labels::HUMAN])];
        assert!(!runnable(&waiting));
        assert!(why_not(&waiting).contains("human action"));
    }

    #[test]
    fn a_closed_human_issue_stops_blocking_like_any_other() {
        let mut freed = task(12);
        freed.blocked_by = vec![closed(issue(11, &[labels::HUMAN]))];
        assert!(runnable(&freed));
    }

    // --- règle 4 : ready commande tout -------------------------------------

    #[test]
    fn an_open_task_without_ready_is_not_runnable() {
        let not_ready = issue(12, &[labels::AGENT]);
        assert!(!runnable(&not_ready));
        assert!(why_not(&not_ready).contains(labels::READY));
    }

    #[test]
    fn a_ready_label_alone_is_not_enough_without_the_agent_label() {
        assert!(!runnable(&issue(12, &[labels::READY])));
    }

    // --- waiting-merge : le troisième état ---------------------------------

    #[test]
    fn a_delivered_task_is_never_picked_again() {
        // Sans ce test, le round suivant la choisirait et repayerait ses
        // trois stages.
        let delivered = issue(12, &[labels::AGENT, labels::READY, labels::WAITING_MERGE]);
        assert!(!runnable(&delivered));
        assert!(next_task(&[delivered]).is_none());
    }

    #[test]
    fn a_delivered_blocker_stops_blocking_so_the_chain_continues() {
        // Son code est sur la branche d'intégration : la suivante peut bâtir
        // dessus. Sinon il faudrait une fusion dans `main` par round.
        let mut next = task(12);
        next.blocked_by = vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])];
        assert!(runnable(&next));
    }

    #[test]
    fn a_delivered_task_says_what_it_is_waiting_for() {
        let delivered = issue(12, &[labels::AGENT, labels::WAITING_MERGE]);
        assert!(why_not(&delivered).contains("waiting for the merge"));
    }

    // --- le rapport de blocage ---------------------------------------------

    #[test]
    fn the_stuck_report_names_a_gesture_for_every_open_task() {
        let mut blocked = task(12);
        blocked.blocked_by = vec![issue(11, &[labels::HUMAN])];
        let report = stuck_report(&[blocked, issue(20, &[labels::AGENT])]);
        assert!(report.contains("#12"));
        assert!(report.contains("#20"));
        assert!(report.contains(labels::READY), "dire quoi cocher");
        assert_eq!(report.lines().count(), 2);
    }

    // --- la preuve de livraison --------------------------------------------

    #[test]
    fn closes_needs_the_keyword_on_its_own_line() {
        assert!(closes("du texte\nCloses #42\nencore", 42));
        assert!(closes("fixes #42", 42));
        assert!(closes("  resolves\t#42  ", 42));
    }

    #[test]
    fn a_mention_inside_a_sentence_does_not_count_as_closing() {
        // Plus strict que GitHub, et dans le bon sens : lire plus large
        // fermerait une issue sur une phrase qui la mentionne.
        assert!(!closes("this PR closes #42 among other things", 42));
        assert!(!closes("see closes #42 below", 42));
    }

    #[test]
    fn closing_another_issue_is_not_closing_this_one() {
        assert!(!closes("Closes #420", 42));
        assert!(!closes("Closes #4", 42));
    }

    #[test]
    fn the_first_pr_that_closes_the_task_is_the_proof() {
        let prs = vec![
            Issue {
                number: 1,
                body: "rien à voir".to_string(),
                ..Issue::default()
            },
            Issue {
                number: 2,
                body: "Closes #42".to_string(),
                ..Issue::default()
            },
        ];
        assert_eq!(first_closing(&prs, 42).expect("une PR").number, 2);
        assert!(first_closing(&prs, 99).is_none());
    }
}
