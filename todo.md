# TODO — à traiter dans des sessions Claude dédiées

Issu du grilling du modèle d'issues du harness (2026-10-04) et de son
implémentation (même jour). Chaque entrée est un chantier à part, pas une
tâche de la boucle. Le modèle cible est résumé en bas.

## Fait

Implémenté et vérifié (check/clippy/fmt/tests) dans cette session :

- **Fondations** : `tokio::time` activé + échéances sur les sous-processus
  (`gh`/`git` 60 s, `claude` 30 min) ; label `harness:triggered` ; extension
  des ports `GitHub` (`create_issue`, `create_sub_issue_link`,
  `add_blocked_by`, `create_pr`, `merge_pr`, `pr_checks_green`) et `git`
  (`create_local_branch`, `stage_all`, `commit`, `push`) ; `init-repo` audite
  la CI sur `milestone/**` et crée `grill:backlog` (hors namespace
  `harness:`, sans en connaître le sens).
- **`dev_loop`** débranché du rollover ; `/planner` retiré de sa table.
- **Deux workflows neufs** : `planner` (découpe un item roadmap en
  milestones) et `split` (découpe une milestone en tâches), sur le gabarit
  de `refinement`, avec tolérance d'un essai sur le JSON
  (`common::json_reply`, partagé).
- **Le routeur** (`common::routing` + `harness watch`) : décision pure,
  boucle de polling, dispatch vers `planner`/`split`/`milestone_merge`/
  `refinement`/`dev_loop` — `refinement` n'avait jamais eu de câblage
  launcher avant cette session.
- **`milestone_merge`** : commande déterministe (pas un workflow, comme
  `init_repo`), ouvre puis merge la PR de milestone une fois ses tâches
  closes et sa CI verte ; pose `harness:waiting-merge` sur la **milestone**.
- **Branche de milestone** : `common::branching::milestone_branch` ;
  `split` la crée (`gh.create_branch`) la première fois qu'elle est
  nécessaire ; le routeur la dérive et la passe à `dev_loop` via
  `Config.integration_branch` au lieu de la branche d'intégration fixe.
  Conséquence vérifiée : le mécanisme existant (la session fait
  branche→PR→merge via le skill `/code`) cible déjà correctement la branche
  de milestone, puisque le préambule du prompt la lui communique — le flux
  à trois niveaux fonctionne de bout en bout sans la suite du chantier
  ci-dessous.

Ajouté le 2026-10-04, après le récapitulatif du flux complet :

- **`pr_review` câblé** : déclenché par `harness:to-review` sur une PR. Son
  `run::build` existait et était testé depuis l'écriture du workflow, mais
  rien ne l'appelait — le déclencheur documenté était un hook sur
  `gh pr create`, qui demanderait une URL publique. Le routeur applique les
  règles de saut du workflow lui-même (`skip_rules`) avant de monter quoi
  que ce soit, et passe la branche **que la PR cible réellement** : une PR
  de tâche cible la branche de milestone, jamais `main_agent`.
- **`pr_fix`, workflow neuf** : déclenché par `harness:pr-fix` **et** au
  moins un check conclu en échec (un check encore en cours n'est pas un
  échec — `FAILED_STATES` le dit explicitement). Lit ce qui a cassé et les
  commentaires, puis une session diagnostique, corrige, vérifie et pousse
  sur la branche de la PR. Un seul workspace en écriture du routeur
  (`router-prfix`, `force_reset`), monté sur la branche de la PR.
  **Le label est consommé avant de payer**, pas après le succès : une PR
  impossible à réparer n'achète pas une session opus par tick.
- **Bug latent corrigé au passage** : `Wanted` nommé refusait de cloner un
  workspace absent (« `--use-workspace` finds, it does not create »), donc
  le checkout partagé `router-readonly` était **inatteignable sur une
  machine neuve** — tout le chemin `planner`/`split`/`refinement` du routeur
  aurait halté au premier tick. `Wanted::create_if_missing` lève cette règle
  pour un nom choisi par le harness, et la garde reste entière pour
  `--use-workspace`.
- `Pr` porte désormais ses labels et sait dire s'il est ouvert, sans quoi un
  workflow déclenché par un label de PR ne pouvait pas vérifier son propre
  déclencheur.
- **Advisory d'`init-repo` repointé** : il conseillait de déclarer un hook
  `pr-review` dans `.claude/settings.json` — exactement le mécanisme
  abandonné (il demanderait une URL publique permanente). Un conseil qui
  envoie un humain construire la mauvaise chose est pire que pas de conseil.
  Remplacé par un constat vrai et utile : **pas de `CLAUDE.md`** dans le
  dépôt cible, donc la carte du dépôt (`common::explore`) ne lit rien et un
  run de `planner` travaille sur le seul item roadmap. Même nombre de
  lectures (`CLAUDE.md` au lieu de `.claude/settings.json`).
- **Décision prise** : `harness:to-review` et `harness:pr-fix` restent des
  **gestes humains**. Rien dans le harness ne les pose, et c'est voulu : les
  deux labels ne sont que *lus*, par le routeur. Ne pas « corriger » ça en
  les faisant poser automatiquement par `dev_loop`.
- **Deadlock corrigé** : `milestone_merge` exigeait que toutes les
  sous-issues soient `closed`, mais une tâche livrée reste **ouverte** sous
  `harness:waiting-merge` — un `Closes #n` ne ferme une issue que si la PR
  est mergée dans la branche **par défaut**, jamais dans une branche de
  milestone. La milestone ne pouvait donc jamais être mergée, quel que soit
  le travail livré. `audit::all_tasks_delivered` lit maintenant « closed
  **ou** `waiting-merge` », la même notion que `tasks::blockers_pending`
  utilisait déjà un niveau plus bas.
- **Et le mensonge que ça aurait introduit, bouché en même temps** : le
  label seul ne suffit pas. `milestone_merge` vérifie désormais la vérité
  git — `common::delivery::every_task_merged` exige, pour chaque tâche, une
  PR **mergée** portant son `Closes #n`. Sans ça, une tâche dont la PR est
  partie en CI rouge puis abandonnée aurait fait passer la milestone pour
  livrée et proposé une branche sans son code. `closes`/`first_closing` ont
  déménagé de `dev_loop::data::tasks` vers `common::delivery` (ré-exportées,
  donc aucun appelant touché) puisque les deux niveaux lisent la même
  convention.

## 1. Commit déterministe des tâches — conception arrêtée, pas construite

**La moitié « réparation CI » n'existe plus comme chantier** : `pr_fix` la
couvre. Ce qui reste est de retirer la mécanique git à la session pour les
**tâches**. État actuel inchangé : le prompt du stage `code` dit toujours à
la session de faire branche→PR→`gh pr merge --rebase` elle-même
(`dev_loop/orchestration/stages.rs`), et `dev_loop::Ports` n'a **pas** de
port `git` du tout.

Conception arrêtée le 2026-10-04 (les deux forks ont été tranchés, ne pas
les rouvrir) :

1. **Le merge se fait à un tick du routeur, pas dans le round** — choix
   explicite de l'utilisateur. Sinon le round tient le workspace exclusif et
   son verrou pendant toute la CI (jusqu'à 20 min) et bloque le routeur.
   Conséquence : le round se termine sur « PR ouverte et poussée », et la
   tâche se ferme un ou deux ticks plus tard. Il faut donc une commande
   déterministe de plus (sur le gabarit de `milestone_merge`) : *une PR de
   tâche dont la CI est verte → merge*, plus une `Route` et un bras de
   dispatch.
2. **Le message de commit est dérivé, pas extrait de la réponse.** La ligne
   `branch: <type>/<slug>` que `split` écrit dans le corps de la tâche donne
   le type ; le titre de l'issue donne le sujet → `"<type>: <titre>"`. Pour
   le stage `create_test`, le type est `test`. Rien à parser dans la réponse
   de la session, donc rien à rater — écart assumé par rapport à la
   formulation d'origine (« le message venant du texte de la session »).
3. **La PR est mémorisée dans l'état** (`Loop` gagne un `pr: Option<String>`,
   posé par l'action qui l'ouvre), ce qui évite une lecture supplémentaire :
   `MarkWaitingMerge` et la postcondition du round la lisent dans l'état au
   lieu de rechercher une PR mergée.

Pièces à écrire :

- `common::branching::task_branch(number, title)` — le repli quand le corps
  de la tâche ne porte pas de ligne `branch:`.
- `dev_loop::data::branch::wanted(body, number, title)` — lit la ligne
  `branch:`, sinon le repli.
- `dev_loop::Ports` gagne `repo: Rc<dyn Repo>` (+ le fake).
- `dev_loop::action::deliver` : `StartBranch` (crée/bascule sur la branche de
  tâche depuis la branche de milestone, idempotent à la reprise),
  `CommitWork` (`stage_all`→`commit`→`push`, sans rien faire si l'arbre est
  propre ; branchée dans le `then` de `code` **et** de `create_test`),
  `OpenPr` (crée la PR avec `Closes #n` si aucune n'existe, mémorise son ref).
- La postcondition du round : `AMergedPrClosesTheTask` devient « une PR
  portant `Closes #n` existe » (ouverte ou mergée).
- Les prompts `CODE` et `CREATE_TEST` : retirer branche→PR→merge.
- Les skills `/code` §7 et `/create-test` §7 : même retrait.

**Attention à la reprise** : `dev_loop` a un `Checkpoint`, et `StartBranch`
doit être idempotent (la branche existe déjà au second passage). Le port
`Repo` n'a aujourd'hui que `create_local_branch` (`git switch -c`, qui
échoue si la branche existe) — il manque un `switch` simple.

## 2. Revoir `grill-to-roadmap` — skill fait le 2026-10-04

Le skill est corrigé : **une session de grilling = une seule issue
`harness:roadmap`** (la version majeure sur laquelle la session a convergé),
plus **une seule issue `grill:backlog`** mise à jour et jamais recréée, avec
une section datée par session et la **raison** de chaque report. Le test à
appliquer à voix haute est écrit dans le skill : *est-ce que livrer
seulement ça laisserait le produit dans un état qui vaut la peine ?* Si la
réponse demande deux candidats, c'est un seul item roadmap ; si l'absence
d'un candidat ne se verrait pas, c'est du backlog. Le skill refuse aussi
d'ouvrir un second item roadmap tant qu'un est ouvert.

**Reste à faire, et c'est un geste humain** : reconsolider les 11 issues
roadmap de `Laucans/dnd_helper` en un item roadmap + un `grill:backlog`.
Non fait volontairement — fermer et réécrire 11 issues réelles sur un dépôt
public demande de décider *quelle* version majeure garder, ce qui est un
jugement produit, pas une transformation mécanique.

## 3. Skill humain pour `split` — fait le 2026-10-04

`.claude/skills/split/SKILL.md` créé, miroir de `planner/SKILL.md` au
niveau milestone → tâches : mêmes mécaniques `gh api`, même geste de
fermeture du robinet (`harness:ready` retiré, `harness:triggered` posé),
même conseil "`needs_human` honnête". Au passage : le prompt du workflow
`split` (`SLICE_PROMPT`, `split/orchestration/stages.rs`) citait
`.claude/skills/planner/SKILL.md`'s task-splitting rules — une référence
devenue fausse depuis que `planner/SKILL.md` a été recentré sur les
milestones (Phase G). Corrigé pour citer `split/SKILL.md` lui-même.

## 4. Le serveur à hooks — décidé, pas construit

Le déclenchement sur pose de label demanderait une URL publique joignable
en permanence. **Décision prise et implémentée** : pas de serveur —
`harness watch` pull toutes les 30 secondes. Le serveur event-driven reste
une option future si la latence du polling devient un problème réel ; rien
à faire ici sauf si ça redevient pertinent.

## Modèle cible, en une page

Trois niveaux, hiérarchie stricte :

| Niveau | Label | Équivalent | Écrit par |
|---|---|---|---|
| Vision produit | `harness:roadmap` | version majeure | l'humain, via `grill-to-roadmap` |
| Feature | `harness:milestone` | version mineure | `planner` (one-shot), étoffé par `refinement` |
| Tâche | `harness:agent` / `harness:human` | — | `split` |

- Un seul item roadmap ouvert à la fois. Le suivant naît d'un nouveau
  grilling.
- « Version majeure / mineure » est une **analogie** : aucun tag, aucune
  release, aucun CHANGELOG produit par le harness.
- Sur une **PR**, deux robinets de même nature : `harness:to-review` lance
  `pr_review`, `harness:pr-fix` lance `pr_fix` (ce dernier seulement si un
  check a réellement échoué, et il consomme son label avant de payer).
- `harness:ready` est le robinet humain côté issues, polysémique par type :
  sur un item roadmap il autorise le découpage en milestones (`planner`), sur
  une milestone il autorise le découpage en tâches (`split`), sur une tâche
  il autorise le développement (`dev_loop`). Le routage se fait sur le label
  de type — `common::routing`. Corrigé le 2026-10-04 : le skill
  `grill-to-roadmap` (alors `business-grill-with-issues`) documentait déjà
  ce geste humain
  ("`/planner`... run through the ordinary `harness` loop once a human
  checks `harness:ready` on a roadmap item") mais le code ne le vérifiait
  pas — `planner` se déclenchait dès qu'un roadmap item n'avait aucune
  milestone, sans geste humain. `routing::decide` et `planner`'s precheck
  l'exigent désormais, symétriquement aux deux autres niveaux.
- Flux de branches : tâche → branche de milestone (`milestone/<n>-<slug>`,
  créée par `split`) → `main_agent` (mergée par `milestone_merge` une fois
  les tâches closes et la CI verte, `harness:waiting-merge` posé sur la
  milestone) → `main` (geste humain).

## Ajouté le 2026-10-05, depuis le premier run réel sur `dnd_helper`

- **Refinement : prendre en compte les commentaires.** Aujourd'hui
  `issue_comments` ne sert qu'à compter les rounds (`rounds::counter`) et
  `Request.context` est toujours vide quand `harness watch` lance le
  workflow (`launcher/src/dispatch/refinement.rs`). Un commentaire humain
  n'oriente donc rien — seule l'édition du corps de l'issue compte. À faire :
  passer les commentaires utiles (hors compteur `refinement round: N`) dans
  `context`, et vérifier que `planned(.., has_context)` les route.
- **Modèles d'issues par type.** Un template GitHub (`.github/ISSUE_TEMPLATE/`)
  pour chaque type : roadmap, milestone, tâche `harness:agent`, tâche
  `harness:human`, backlog. À poser par `init-repo`, avec les labels du type
  déjà cochés.
  **Le harness doit respecter ces templates** : aujourd'hui `planner`,
  `split` et `refinement` écrivent leurs propres gabarits d'issue (sections
  de `refinement::data::sections`, corps de tâche de `split`). Les templates
  deviennent la source de vérité et les workflows les lisent au lieu d'avoir
  chacun le leur.
- **Refinement : le round 1 doit aller aussi loin que les rounds 1 + 2.**
  `rounds::planned` (`workflow/refinement/data/rounds.rs`) écrit seulement
  `keys_of_round(1)` au round 1 (business-goal, technical,
  acceptance-criteria) ; le round 2 ajoute `keys_of_round(2)` plus les
  sections manquantes (business-rules, technical-plan). Fusionner : round 1 =
  `keys_of_round(1)` + `keys_of_round(2)`, et revoir ce que devient le round 2
  (relecture ? rien ?).
- **Détecter un échec d'agent répété pour couper la boucle.** Le 2026-10-05,
  `planner` a échoué 13 fois de suite (prompt commençant par `---` lu comme
  une option par `claude`), sans rien produire : 3,77 $ d'`explore` perdus,
  le watch relançant la même run à chaque tick. À faire : un compteur
  d'échecs consécutifs par workflow et par cible (issue/PR) dans `harness
  watch`, qui suspend la route après N échecs identiques et le dit à
  l'humain. Corriger aussi la classification : le ledger a noté ces échecs
  `QUOTA` alors que `halt_for` n'avait vu aucun quota — un échec de lancement
  de `claude` ne doit pas se lire comme un quota.
