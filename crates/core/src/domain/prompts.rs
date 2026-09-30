//! Le préambule, le bloc de portée, et comment ils se composent.
//!
//! Ce que **toute** session reçoit, quel que soit le workflow qui la lance : le
//! bloc EXECUTION CONTEXT — là où il est dit qu'on ne peut poser aucune
//! question, comment s'arrêter, et sur quelle branche on travaille — et le bloc
//! SCOPE, qui porte le milestone et l'issue. Les consignes propres à un stage
//! s'ajoutent à ça et ne sont pas ici.
//!
//! **Ce module ne connaît aucun workflow.** La prose d'un stage lui est passée
//! en argument.
//!
//! Le préambule exige `AGENT_LOOP_OK:` en fin de réponse et
//! [`crate::domain::markers`] le relit : deux moitiés d'un même contrat, et un
//! test exige qu'elles nomment la même chaîne.
//!
//! # Une seule substitution, en une passe
//!
//! Le Python avait deux fonctions — `fill`, séquentielle, et `splice`, une
//! passe — et n'appliquait `splice` qu'au raffinage. Or le bloc SCOPE porte des
//! corps d'issue et des titres, **qui viennent de GitHub** : un `{body}` écrit
//! dans un corps de milestone se faisait substituer par la passe suivante.
//! C'est exactement ce que `splice` existait pour empêcher, appliqué au mauvais
//! endroit. Les deux sont fusionnées ici en une seule passe : identique quand
//! les valeurs sont propres, sûre quand elles ne le sont pas.

/// Le nom cité dans les blocs quand l'appelant n'en donne pas d'autre.
///
/// Volontairement générique : nommer ici la CLI d'un workflow précis serait
/// mettre une instance dans le vocabulaire, ce que cette couche n'a pas le
/// droit de porter.
pub const INJECTOR: &str = "the harness runner";

/// Ce qu'un corps vide dit, en mots.
///
/// Dit plutôt que laissé blanc : un stage qui lit une section blanche ne peut
/// pas distinguer « rien n'a été écrit » de « l'injection a cassé », et une
/// seule des deux vaut un arrêt.
pub const EMPTY_BODY: &str = "(empty — nothing has been written into this issue yet)";

const PREAMBLE: &str = "
--- EXECUTION CONTEXT (injected by @INJECTOR@) ---
You are running head-less in an unattended loop (`claude -p`). Nobody will
read this output before the run ends and nobody can answer a question.
These rules override the skill's interactive stopping points:

1. Where the skill waits for a go-ahead or a confirmation (/code steps 2
   and 3), write your findings into your reply and carry on. Reporting
   stays mandatory; waiting does not.
2. Where the skill says to ask because a choice is genuinely ambiguous, do
   not guess. Stop, change nothing further, and end your reply with the one
   line `AGENT_LOOP_STOP: <one-line reason>`. The loop halts and a human
   picks it up. Stopping is a correct outcome, not a failure.
3. The integration branch for this run is `@BRANCH@`. Wherever a skill says
   `main` as the PR base or the branch-off point, read `@BRANCH@`: branch
   off it, and `gh pr create --base @BRANCH@`. The rest of CLAUDE.md's
   Repository etiquette stands unchanged — branch -> PR ->
   `gh pr merge --rebase`, and never a direct push.
4. Stage by name, never `git add -A`. The tree may carry unrelated
   in-flight work that is not yours to commit.
5. Do not start another pipeline stage as its own process, and do not
   /clear. The loop runs one process per stage. Where this prompt names a
   second skill to continue into, that continuation is part of this same
   stage — not a new one, and not something to hand off.
6. End your reply with `AGENT_LOOP_OK: <one-line summary>` if the stage
   completed, or `AGENT_LOOP_STOP: <reason>` if it did not.
--- END EXECUTION CONTEXT ---";

const SCOPE: &str = "--- SCOPE (injected by {injector}) ---
The milestone and the issue below are the whole brief; there is no
docs/current/ any more. Both are GitHub issues: what you produce goes back
into the issue, not into a file under docs/.

MILESTONE #{milestone} — {milestone_title}
{milestone_body}

ISSUE #{num} — {title}
{body}
--- END SCOPE ---";

/// Une issue telle qu'une session doit la voir : son numéro, son titre, son
/// corps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Named {
    /// Le numéro, en texte — il n'entre que dans de la prose.
    pub number: String,
    /// Le titre.
    pub title: String,
    /// Le corps. Pour une task, le corps **est** le SPEC.
    pub body: String,
}

/// Ce sur quoi une session travaille : un milestone, et une task dedans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Le milestone.
    pub milestone: Named,
    /// La task.
    pub task: Named,
}

/// Porté par l'état d'un workflow dont les sessions travaillent sur une issue.
///
/// C'est la borne qui rend vraie, à la compilation, la règle « une session ne
/// démarre jamais sans sa portée » : une action de session qui compose un
/// prompt exige `S: Scoped`, donc elle ne peut pas être écrite sans qu'une
/// portée soit disponible. Côté Python, c'était une règle écrite dans un
/// commentaire que rien ne tenait.
pub trait Scoped {
    /// Le milestone et la task de ce round.
    fn scope(&self) -> Scope;
}

/// Remplace chaque `{nom}` connu, **en une seule passe**.
///
/// Une valeur insérée n'est jamais réexaminée : un `{body}` qui arrive *dans*
/// un corps d'issue reste littéral. Un `{nom}` inconnu reste littéral aussi —
/// ces gabarits sont de la prose écrite pour un modèle, et elle porte des
/// accolades qui ne nous appartiennent pas.
#[must_use]
pub fn splice(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
                // Poussée dans la sortie, donc hors d'atteinte des
                // remplacements suivants. C'est tout l'intérêt.
                out.push_str(value);
            } else {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
            rest = &after[end + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Le bloc EXECUTION CONTEXT, branche d'intégration substituée.
#[must_use]
pub fn preamble(branch: &str, injector: &str) -> String {
    PREAMBLE
        .replace("@INJECTOR@", injector)
        .replace("@BRANCH@", branch)
}

/// Le bloc SCOPE d'un stage : le milestone, puis la task.
#[must_use]
pub fn scope_block(scope: &Scope, injector: &str) -> String {
    let milestone_body = non_empty(&scope.milestone.body);
    let body = non_empty(&scope.task.body);
    splice(
        SCOPE,
        &[
            ("injector", injector),
            ("milestone", &scope.milestone.number),
            ("milestone_title", &scope.milestone.title),
            ("milestone_body", &milestone_body),
            ("num", &scope.task.number),
            ("title", &scope.task.title),
            ("body", &body),
        ],
    )
}

fn non_empty(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        EMPTY_BODY.to_string()
    } else {
        trimmed.to_string()
    }
}

/// L'`extra` d'un stage : ses consignes, puis la portée où il travaille.
///
/// La portée vient en dernier parce que c'est la partie longue — les consignes
/// restent là où un lecteur, et un modèle, les trouvent : en haut.
///
/// `scope` est un `Option` pour **un seul** cas, et il a une raison : le stage
/// de rollover (`/planner`) ne travaille sur aucune task, il en ouvre. Lui
/// injecter un bloc ISSUE vide lui donnerait une task à chercher. Tous les
/// autres en reçoivent une, y compris ceux qui n'ont pas de consignes propres.
#[must_use]
pub fn extra_for(instructions: &str, scope: Option<&Scope>, injector: &str) -> String {
    let Some(found) = scope else {
        return instructions.to_string();
    };
    let filled = splice(
        instructions,
        &[
            ("num", &found.task.number),
            ("title", &found.task.title),
            ("milestone", &found.milestone.number),
        ],
    );
    let block = scope_block(found, injector);
    if filled.is_empty() {
        block
    } else {
        format!("{filled}\n\n{block}")
    }
}

/// Le prompt complet d'un stage : la commande, le préambule, puis l'extra.
#[must_use]
pub fn build(lead: &str, branch: &str, extra: &str, injector: &str) -> String {
    let mut prompt = format!("{lead}\n{}", preamble(branch, injector));
    if !extra.is_empty() {
        prompt.push('\n');
        prompt.push_str(extra);
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::markers;

    fn scope() -> Scope {
        Scope {
            milestone: Named {
                number: "12".to_string(),
                title: "Le chat".to_string(),
                body: "ce que le milestone dit".to_string(),
            },
            task: Named {
                number: "34".to_string(),
                title: "La grille".to_string(),
                body: "le SPEC".to_string(),
            },
        }
    }

    // --- les deux moitiés du contrat verbal --------------------------------

    #[test]
    fn the_preamble_names_the_same_markers_the_domain_reads() {
        // Deux moitiés d'un même contrat : si l'une change de nom sans
        // l'autre, une session dirait « fini » dans une langue que le harness
        // ne lit plus.
        let said = preamble("main_agent", INJECTOR);
        assert!(said.contains(markers::OK), "le préambule doit exiger OK");
        assert!(said.contains(markers::STOP), "et offrir STOP");
    }

    #[test]
    fn the_integration_branch_replaces_every_occurrence() {
        let said = preamble("main_agent", INJECTOR);
        assert!(!said.contains("@BRANCH@"));
        // Trois mentions dans la règle 3 : la base de PR, le point de départ,
        // et la commande. En rater une enverrait une PR sur `main`.
        assert_eq!(said.matches("main_agent").count(), 3);
    }

    #[test]
    fn the_injector_is_named_rather_than_left_as_a_placeholder() {
        let said = preamble("main_agent", "harness/src/main.rs");
        assert!(said.contains("injected by harness/src/main.rs"));
        assert!(!said.contains("@INJECTOR@"));
    }

    // --- la substitution en une passe --------------------------------------

    #[test]
    fn an_untrusted_value_carrying_a_placeholder_stays_literal() {
        // Le mode de panne que ça évite : un corps d'issue qui contient
        // `{body}` se faisait remplacer par la passe suivante.
        let out = splice(
            "A={a} B={b}",
            &[("a", "ceci contient {b} littéralement"), ("b", "REMPLACÉ")],
        );
        assert_eq!(out, "A=ceci contient {b} littéralement B=REMPLACÉ");
    }

    #[test]
    fn an_unknown_placeholder_is_left_alone() {
        // La prose écrite pour un modèle porte ses propres accolades.
        assert_eq!(
            splice("garde {ceci} et mets {a}", &[("a", "ça")]),
            "garde {ceci} et mets ça"
        );
    }

    #[test]
    fn an_unclosed_brace_does_not_swallow_the_rest() {
        assert_eq!(splice("avant { après", &[("a", "x")]), "avant { après");
    }

    #[test]
    fn a_template_without_any_brace_comes_back_unchanged() {
        assert_eq!(splice("rien à faire", &[("a", "x")]), "rien à faire");
    }

    // --- le bloc de portée -------------------------------------------------

    #[test]
    fn the_scope_block_carries_both_issues_verbatim() {
        let said = scope_block(&scope(), INJECTOR);
        assert!(said.contains("MILESTONE #12 — Le chat"));
        assert!(said.contains("ce que le milestone dit"));
        assert!(said.contains("ISSUE #34 — La grille"));
        assert!(said.contains("le SPEC"));
    }

    #[test]
    fn an_empty_body_is_said_in_words_not_left_blank() {
        let mut bare = scope();
        bare.task.body = "   ".to_string();
        let said = scope_block(&bare, INJECTOR);
        assert!(said.contains(EMPTY_BODY));
    }

    #[test]
    fn a_body_that_mentions_another_field_is_not_substituted() {
        // Le cas réel : quelqu'un écrit `{body}` dans le corps du milestone.
        let mut tricky = scope();
        tricky.milestone.body = "voir {body} et {num}".to_string();
        let said = scope_block(&tricky, INJECTOR);
        assert!(said.contains("voir {body} et {num}"));
    }

    // --- l'extra -----------------------------------------------------------

    #[test]
    fn instructions_come_before_the_scope_because_scope_is_the_long_part() {
        let said = extra_for("fais ceci", Some(&scope()), INJECTOR);
        let instructions = said.find("fais ceci").expect("les consignes");
        let block = said.find("--- SCOPE").expect("la portée");
        assert!(instructions < block);
    }

    #[test]
    fn a_stage_without_instructions_still_gets_its_scope() {
        // /create-test n'a pas de consignes propres, et serait sinon la seule
        // session payée du round qui ignore quelle task elle teste.
        let said = extra_for("", Some(&scope()), INJECTOR);
        assert!(said.contains("ISSUE #34"));
        assert!(!said.starts_with('\n'));
    }

    #[test]
    fn the_rollover_stage_gets_no_issue_block_to_hunt_for() {
        // /planner n'a aucune task : lui injecter un bloc ISSUE vide lui
        // donnerait quelque chose à chercher.
        let said = extra_for("ouvre le prochain item", None, INJECTOR);
        assert_eq!(said, "ouvre le prochain item");
        assert!(!said.contains("ISSUE #"));
    }

    #[test]
    fn instruction_placeholders_are_filled_from_the_task() {
        let said = extra_for("travaille sur #{num} ({title})", Some(&scope()), INJECTOR);
        assert!(said.contains("travaille sur #34 (La grille)"));
    }

    // --- la composition ----------------------------------------------------

    #[test]
    fn the_prompt_opens_on_the_command_then_the_preamble() {
        let built = build("/code", "main_agent", "", INJECTOR);
        assert!(built.starts_with("/code\n"));
        assert!(built.contains("--- EXECUTION CONTEXT"));
    }

    #[test]
    fn the_extra_comes_after_the_preamble() {
        let built = build("/code", "main_agent", "les consignes", INJECTOR);
        let context = built.find("END EXECUTION CONTEXT").expect("le préambule");
        let extra = built.find("les consignes").expect("l'extra");
        assert!(context < extra);
    }
}
