# Plan — agents en parallèle, contexte borné, tokens maîtrisés

## Contexte

L'architecture « agent-native » (`crates/architecture/assets/ARCHITECTURE.md`) promet
des unités de lecture qu'un agent construit seul, derrière des contrats vérifiés en
CI. Le harness ne tient pas encore cette promesse sur trois points mesurés :

- **La session voit tout.** Un clone complet par lane
  (`crates/core/src/execution/provisioning.rs`), aucune vue par tranche. Le pic de
  contexte d'une session `code` est ~168 k tokens dont ~52 k de résultats d'outils
  (31 k en 8 `cat` de fichiers entiers), pour un prompt du harness de ~12 k.
- **Le prompt fixe est gros et identique pour toutes les tâches.** SCOPE jusqu'à
  60 k caractères (`crates/core/src/domain/prompts.rs`, `scope_block`) : roadmap
  ~11 k relue à chaque tour par chaque tâche de chaque jalon, corps de jalon et de
  tâche non plafonnés. Conséquence mesurée : le harness est plus efficace sur les
  grosses tâches (26,6 lignes/tour contre 9,6), l'inverse de ce qu'on veut.
- **Aucun budget, aucune rétroaction.** Pas de `--max-turns`, pas de budget par
  tâche, le `profile` du découpage est figé à `mid`
  (`crates/workflows/src/split/config.rs`, `profile_for`). Le coût n'est lu que
  pour l'affichage. Une coupure de quota a gâché 28 % du coût de quatre tâches.

Et le merge parallèle n'est pas sûr : gates en `grep`, conformance des Concepts
désactivée, requêtes persistées non enregistrées, moteur DataGuard inexistant,
PR côté écriture mergée sans humain (`crates/launcher/src/router/mod.rs`,
`green_task_pr` → `milestone_merge`), contrairement au doc.

Décisions prises avec l'utilisateur : construire le moteur complet (DataQueue,
holds, Resolver) ; borner le contexte par **sparse checkout par tranche** ; ce
fichier est le livrable, rien n'est créé sur GitHub.

## Principe directeur

**L'unité d'architecture est l'unité de contexte et l'unité de budget.** Une
tranche (Capability, DataCapability, Micro-UI, agrégat, Concept) se construit à
partir d'une **fermeture** calculable : son dossier, la plomberie qu'elle atteint
(déjà calculée par `scripts/isolation.sh` sur `cargo metadata`), les manifestes de
ses `needs`, les Concepts qu'elle implémente, l'agrégat qu'elle cible, sa page
mock. Le harness calcule cette fermeture, la monte physiquement, la met dans le
prompt, lui attache un budget, et apprend du coût mesuré pour recouper.

Chaque phase est mesurée avant/après avec le ledger existant
(`.llocal/logs/agent-loop/costs.tsv`) sur le même jalon du banc d'essai.

## Phase 0 — Mesurer et aligner

But : une base de comparaison et un doc qui dit ce que le code fait.

- `harness costs` (launcher, lecture seule du ledger) : par tâche, stage, **unité**
  et **profil** — $ , tours, tokens entrée hors cache, cache read, lignes de PR.
  Ajouter les colonnes `unit`, `profile`, `side` à la ligne du ledger
  (`crates/core/src/domain/spend.rs`, `crates/adapters/src/store/ledger.rs`,
  `ask_and_record` dans `crates/core/src/execution/action/ask.rs`), lues depuis la
  `Declaration` de la tâche.
- Compteur de lectures hors fermeture : depuis `stream.jsonl`, les `cat`/`Read`
  dont le chemin n'est pas dans le pack (sert de métrique aux phases 2 et 3).
- Aligner doc ↔ code :
  - chaînage « par couche » annoncé, `needs`→`produces` fait
    (`crates/workflows/src/split/action/publish.rs`, `crates/workflows/ARCHITECTURE.md`
    l.257, `.claude/skills/split/SKILL.md`) ;
  - `harness:data-layer` posé sur **tous** les write-side ;
  - `review-pending` : **rétablir le merge humain côté écriture** (promesse du doc,
    condition de sûreté du parallélisme) : `green_task_pr` ne merge que
    `harness:read-side` ; une PR write-side attend `harness:approved` posé par un
    humain. Mesurer le débit perdu ;
  - `migration`/`invariant` encore cités comme unités dans les docs installés
    (`docs/PARKED.md`) ; chemins `crates/dataguard/...` vs globs `crates/*/...`
    dans `CLAUDE-rules.md`, `gates.yml`, `isolation.sh` : une seule lecture.

## Phase 1 — Régime du prompt fixe

But : diviser par deux le texte relu à chaque tour, sans sortir la session de son
périmètre (risque asymétrique noté dans `observations.md`).

- `Cut` par stage (`crates/workflows/src/dev_loop/data/brief.rs`,
  `prompts::Brief`) :
  - `code` : **roadmap retirée** (plan déjà écrit), jalon réduit à son but +
    `## Architecture` (≤ 2 000 car), corps de tâche = `Architecture`,
    `Acceptance Criteria`, `Business Rules`, `Technical`, `TIP` ; sœurs seulement
    celles nommées dans `needs`/`produces` ;
  - `technical-refinement` : roadmap réduite à `ROADMAP_KEPT` (déjà), jalon
    plafonné ;
  - `create-test` : inchangé (`TaskOnly`).
  - Plafonds explicites : `MILESTONE_BUDGET`, `TASK_BUDGET` dans
    `crates/core/src/domain/prompts.rs`, coupe par sections entières
    (`common::sections`), jamais au milieu d'une (bug UTF-8 déjà vu : couper sur
    `chars`).
- **Carte de règles par unité** au lieu de `docs/ARCHITECTURE.md` entier :
  `harness-architecture::rules::card(Unit) -> &'static str` (~40 lignes chacune :
  layout, manifeste, ce qu'elle ne doit pas connaître, la gate qui la juge),
  injectée dans le prompt `code` ; `CLAUDE-rules.md` garde les règles communes
  seulement. Résout le conflit `CLAUDE-rules` (« lis docs/ARCHITECTURE.md ») vs
  `/code` (« ne le lis pas »).
- Signatures : budget 12 k conservé mais sélection élargie aux fichiers de la
  fermeture (phase 2), plus seulement aux chemins nommés dans le corps
  (`crates/workflows/src/dev_loop/data/signatures.rs`, `carried`).
- A/B sur deux tâches de même unité : tokens entrée hors cache par tour, tours,
  et zéro sortie de périmètre (`ArchitectureHolds` vert).

## Phase 2 — Fermeture par tranche et sparse checkout

But : la borne est physique. Ce qui n'est pas dans la lane ne peut pas être lu.

- **Calcul pur, dans `harness-architecture`** : `src/closure.rs`,
  `closure(&Tree, &Declaration, &CargoGraph) -> Closure { paths, members, missing }`.
  Règles par unité :
  - Capability / DataCapability : son dossier ; plomberie atteinte (fermeture du
    graphe, même règle qu'`isolation.sh`, portée en Rust sur `cargo metadata` lu
    via le port `Process`) ; `contracts/` ; `concepts/<C>/v<n>/` de `implements` ;
    `crates/<system>/queries/` ; pour une DataCapability `aggregate.json` cible et
    `crates/dataguard/server/{Cargo.toml,build.rs}` (ligne de dépendance exigée par
    la gate) ;
  - Micro-UI / composition : `apps/<system>/<ui>/`, `apps/shell/` (types du client,
    d.ts seulement), `mock/pages/<page>.html`, `mock/components/<ui>.*`,
    `mock/src/`, `mock/design/tokens.css`, `mock/kit/`, les manifestes des `needs`
    et `actions` (JSON seulement, pas le code) ;
  - agrégat / infrastructure : `crates/dataguard/` entier + les manifestes des
    Capabilities qui lisent ses tables (registre des requêtes) ;
  - toujours : racine (`Cargo.toml`, `package.json`, `CLAUDE.md`, `.github/`,
    `scripts/`, `data/model.json`), jamais `docs/ARCHITECTURE.md` (carte de règles
    à la place).
  - `missing` : les ids nommés dans `needs` sans manifeste → la tâche n'est pas
    démarrable (`startable_tasks`, `crates/workflows/src/dev_loop/data/tasks.rs`),
    au lieu d'une session qui cherche.
- **Montage** : port `Git` enrichi de `sparse_checkout(paths)` (déclaré dans
  `crates/core/src/ports/`, implémenté dans `crates/adapters/`) ; `provisioning`
  l'applique après le `reset --hard` de la lane. Le clone reste complet côté
  `.git`, seul l'arbre de travail est partiel : `git push` et la PR sont inchangés.
- **Contrainte produit** : `members` du workspace par glob uniquement (déjà la
  règle 2 de `CLAUDE-rules.md`) ; un membre listé explicitement entre dans toute
  fermeture. Nouvelle gate `workspace-members-by-glob` dans `gates.yml`.
- **Marqueur de manque** : le prompt dit que l'arbre est partiel et comment le
  signaler (`AGENT_LOOP_STOP: missing <path>`). Le stop est journalisé avec le
  chemin ; `harness costs` les compte. Trois manques sur une même règle de
  fermeture = la règle s'élargit, pas la session.
- Tests : `closure` sur `FakeDisk` (`crates/architecture/src/fake_disk.rs`), un
  produit de fixture par unité ; proptest sur les chemins ; intégration dans
  `crates/architecture/tests/` sur un vrai workspace (comme `isolation_script.rs`).
- Mesure : résultats d'outils par session, lectures hors fermeture (doit tendre
  vers zéro), coût par tâche à unité égale.

## Phase 3 — Budget par tâche et rétroaction vers le découpage

But : un plafond par session et un découpage qui apprend du coût mesuré.

- `Session` (port) prend un `turn_cap` ; `ClaudeCli::argv`
  (`crates/adapters/src/agent/claude_cli.rs`) ajoute `--max-turns`. Valeur par
  stage et par profil (`strong`/`mid`/`small`), dans la table des stages
  (`crates/workflows/src/dev_loop/orchestration/stages.rs`).
- Check `BudgetHasRoom` (à côté de `QuotaHasRoom`,
  `crates/workflows/src/dev_loop/checks/preflight.rs`) : refuse un stage quand le
  cumul de la tâche (`total_cost_usd`, cumulatif depuis CLI 2.1.277) dépasse le
  budget de son unité ; verdict `Continue` quand il ne s'applique pas (leçon
  `--ignore-quota`).
- Port `CostHistory` dans `core::ports::store` (médiane et p75 du coût par unité
  et profil), adapter launcher sur `Ledger`. Budget d'une tâche = p75 × 1,5 de son
  unité ; à défaut, une constante par unité.
- `profile_for` déduit du modèle **et** de l'historique : si le p75 d'une unité
  dépasse le budget, le découpage passe de `mid` à `small` pour cette unité (coupe
  `PJ` → `PJ.invariants` → `PJ.migration`, déjà décrite dans `profile_block`).
  Branché dans la route du watch (`crates/launcher/src/router/routes.rs`), là où
  le modèle est connu (`docs/PARKED.md`).
- `split` reçoit, dans l'inventaire (`inventory_block`,
  `crates/workflows/src/split/action/actions.rs`), le coût médian par unité :
  « une Capability coûte X, un agrégat Y » — le découpage voit ce qu'il coûte.
- Reprise après coupure : un run coupé par le quota reprend sur le même
  `--resume` (checkpoint existe) au lieu de repayer le stage.

## Phase 4 — Rendre le merge parallèle sûr

But : les gates jugent la structure, pas du texte. Sans quoi le parallélisme
côté lecture merge du code sans humain sur la foi d'un `grep`.

- **Lecture seule par les types** : la plomberie de chaque système expose un crate
  `<system>-read` (connexion `ReadOnly` sans API d'écriture, requêtes par hash).
  Une Capability ne dépend ni de `sqlx` ni de `tokio-postgres` directement :
  règle ajoutée à `isolation.sh` et à `tree::violations` (dépendances interdites),
  gate `read-only` ramenée à ce test de graphe. Le rôle Postgres read-only reste
  (défense en profondeur), provisionné par `docker/postgres/compose.yml`.
- **Registre réel des requêtes** : `queries/registry.json` généré par un script
  installé (`scripts/queries.sh`) depuis les `.graphql`/`.sql` des Capabilities,
  hash par fichier ; gate `queries-registered` : chaque `reads:` d'un manifeste
  existe et son hash correspond ; `<system>-read` n'accepte qu'un hash enregistré.
- **Runner de conformance** : crate installé `crates/platform/conformance/`
  (lit `concepts/<C>/v<n>/fixtures.json`, appelle la Capability par son id) ; gate
  `semantic-conformance` activée.
- `invariants-tested` : l'id doit apparaître dans un `#[test]` **de la crate de
  l'agrégat**, pas n'importe où ; `write-actions` vérifie aussi `permissions`.
- `ArchitectureHolds` (`crates/workflows/src/dev_loop/checks/architecture.rs`)
  juge le **diff du round** et non l'arbre : la tâche n'a touché que sa fermeture
  (sauf `Cargo.lock`, la ligne de dépendance du serveur, `mock/pages.json`).
- Doubles règles (`tree::violations` ↔ `gates.yml`) : un test de
  `crates/architecture/tests/` fait tourner `gates.yml` (les étapes `run:`)
  sur les fixtures de `violations` et exige le même verdict.

## Phase 5 — Moteur DataGuard installable, mock fidèle

But : le côté écriture existe une fois, les produits l'utilisent, les agents ne
le lisent jamais (il est hors de leur fermeture, seule son API publique y est).

- Nouveau crate `harness-dataguard` (lib, `crates/dataguard/`), **hors du graphe
  du harness** comme `harness-view-render` : ne dépend d'aucun crate du workspace,
  Postgres via `tokio-postgres` (fait : `sqlx` verrouille un `libsqlite3-sys` qui
  entre en conflit avec celui de `rusqlite` dans `harness-adapters`),
  `#[async_trait(?Send)]` non requis (runtime produit, multi-thread). Les produits le prennent par `git = …, tag = …` ; `init-repo` installe
  le squelette `crates/dataguard/server/` (build script qui découvre
  `data-capabilities/*`, déjà attendu par la gate `data-capability-registered`).
  `scripts/dead-code.sh` et `cargo machete` l'excluent explicitement, avec le motif.
- Modules, chacun un contrat :
  - `queue` (G) : table `data_queue`, une partition par agrégat,
    `FOR UPDATE SKIP LOCKED`, position fixe, états du contrat ;
  - `guard` (F, H, I) : permissions, classification statique puis dynamique de
    l'effet, invariants (`trait Invariant { fn check(&State) -> Result<(), Id> }`
    enregistrés par agrégat), double vérification (projeté / réel) ;
  - `resolver` (J) : plan d'impact depuis le registre des requêtes et
    `relations` d'`aggregate.json`, politiques `restrict/cascade/nullify/archive` ;
  - `applier` : un seul, `dataVersion` séquence globale, journal des commandes,
    soft delete + filtrage des tombstones dans `<system>-read` ;
  - `holds`, `parked`, `relative` : TTL 1 min / 24 h, commandes relatives sans
    confirmation ;
  - `messages` (L) : `LISTEN/NOTIFY` → SSE, un abonnement par auteur ;
  - `reconcile` (K) : proposition IA = appel sortant derrière un trait, décision
    humaine par rôle approbateur ;
  - `registry` (C) : les hash de requêtes servis à `<system>-read`.
  - Protocole de fil : **celui que `mock/src/server.js` définit déjà**
    (`POST /capabilities/<id>`, `POST /commands`, `GET /commands/<id>`, flux
    `dataVersion`) : le même client shell parle aux deux.
- Ordre de livraison, chacun une tranche testable seule : `registry` → `guard`
  synchrone (fast path) → `applier` + journal → `queue` + `parked` → `holds` +
  `relative` → `resolver` → `messages` → `reconcile`.
- Gates `queries-valid` et `schema-impact` activées (fait : `queries-valid`
  monte un Postgres de service, applique les migrations du commit et `PREPARE`
  chaque requête SQL du registre ; `schema-impact` lit les colonnes qu'une
  migration du diff supprime, renomme ou retype et exige que la même PR porte
  la requête de chaque lecteur du registre).
- **Mock fidèle** : `mock/src/server.js` prend les états `queued`,
  `awaiting_confirmation`, `parked`, les messages `ValueDeclaredAhead` et
  `CommandRejected` ; `commands.js` juge avec les invariants d'`aggregate.json`
  (lus en fixture) et non du JS à la main ; une DataCapability absente est
  **refusée**, plus appliquée telle quelle.

## Phase 6 — Concept v2

But : un document par lecteur, réinstallable, cohérent avec ce que le harness
impose.

- `ARCHITECTURE.md` coupé en deux documents installés : **« Layout et contrats »**
  (ce que les gates vérifient, ce qu'un agent doit savoir : principes, unités,
  contrats A–F, I, M, N, O, gates) et **« Intégrité des données à l'exécution »**
  (queue, holds, confirmations, Resolver, messages : G, H, J, K, L — le manuel du
  moteur, lu par l'agent d'infrastructure seulement).
- Cartes de règles par unité (phase 1) tenues à jour avec les contrats : un test
  d'`assets.rs` vérifie que chaque `Unit` a sa carte et que chaque gate de
  `gates.yml` est citée par une carte.
- Hôtes sans serveur (plugin, app statique) : une section qui dit ce qui vaut
  (lecture, Concepts, mock, gates) et ce qui ne vaut pas (queue, credentials) ;
  un `profile` d'architecture `hostless` dans la `Declaration` pour que
  `violations` ne réclame pas un serveur.
- Réinstallation versionnée : marqueur `harness:architecture v2`,
  `init_repo::data::install` remplace les blocs marqués, n'ajoute jamais deux fois.
- Contradiction résolue dans le texte : le principe 10 (« jamais rejetée pour
  concurrence ») est reformulé — jamais rejetée **pour concurrence**, rejetée en
  tête sur invariant du vrai état.

## Vérification de bout en bout

- Banc d'essai : le produit de test déjà utilisé (`dnd_helper`, jalons 15–17),
  même jalon rejoué avant/après chaque phase, `harness costs` comme juge :
  - tokens entrée hors cache par tour (phases 1–2), lectures hors fermeture
    (phase 2, cible 0), tours par stage et $ par unité (phase 3) ;
  - lignes/tour sur petites tâches : doit remonter vers celui des grosses.
- Parallélisme : `harness watch --parallel 4` sur un jalon de quatre Capabilities
  de systèmes différents ; quatre PR mergées sans conflit, aucune lecture hors
  fermeture, `ArchitectureHolds` vert sur le diff.
- Sûreté : un produit de fixture par violation (Capability qui importe `sqlx`,
  requête non enregistrée, Concept sans fixtures, DataCapability qui touche un
  champ non déclaré) ; `gates.yml` rouge et `violations` identique.
- Moteur : tests d'intégration sur Postgres Docker (`scripts/postgres.sh`) par
  module, scénario complet du contrat G (deux auteurs, une valeur déclarée devant,
  confirmation, parked, expiration).
- Harness lui-même, à chaque phase : `cargo check/clippy/fmt/test`, `cargo machete`,
  `bash scripts/dead-code.sh`, `cargo mutants --in-diff` (CLAUDE.md).

## Risques et hors périmètre

- Sparse checkout : une fermeture trop étroite coûte un stop et un tour, pas un
  bug ; une fermeture trop large coûte des tokens. Le compteur de manques arbitre.
- Merge humain côté écriture : le débit baisse ; c'est le prix de la sûreté que
  le doc promet. Le moteur (phase 5) réduit ensuite le nombre de tâches write-side.
- Le moteur est un produit à part entière : à livrer tranche par tranche avec le
  harness lui-même, en `harness:write-side`.
- Non traité ici : composition d'écran par IA à l'exécution (contrat E « at run
  time »), coût des boucles bash des gates (acceptable à l'échelle actuelle).
