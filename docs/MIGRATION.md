# Migration du pipeline vers un harness Rust

Document de travail, écrit au fur et à mesure. Il porte les décisions prises
et ce qu'elles impliquent — pas un récit de la migration.

La source : `event_assistant/pipeline/` (Python, 121 modules, 667 cas de test).
Sa carte : `pipeline/ARCHITECTURE.md` et `pipeline/src/pipeline/core/ARCHITECTURE.md`.

## Objet

Réécrire le runner en Rust, sous le nom **harness** — pas « pipeline » : il ne
fait pas que déplacer de la donnée par étapes, il pilote des sessions d'agent
au travers de portes de vérification, ce qui est précisément ce que le mot
désigne dans l'écosystème des agents de code (boucle, observation, actions,
vérification, persistance d'état). Et profiter du passage pour corriger trois
choses que le Python a laissées s'installer :

- le vocabulaire d'exécution est implicite — il n'y a pas de type `Round`, ni
  de type `Stage` exécutable ; ce sont des fermetures passées à des `shape`s ;
- l'état d'exécution vit dans trois objets qui se chevauchent (`Ctx`,
  `RoundCtx(Ctx)`, `RoundState`) ;
- « vérifier » et « faire » sont mélangés : deux gardes écrivent sur GitHub.

## Les trois crates

L'arborescence est conservée. Elle devient un graphe de crates, dans un
workspace Cargo :

```
harness-launcher (bin)  ->  harness-workflows (lib)  ->  harness-core (lib)
     ce qui trigge              la définition des             le framework
                                    workflows
```

**Ce que ça remplace** : `tests/test_layering.py` et sa table `ALLOWED`.
« Le framework ignore ses utilisateurs » cesse d'être un test qui parcourt des
AST — un `use harness_workflows::` dans `harness-core` est une dépendance
cyclique, et Cargo refuse de compiler. L'invariant n°1 du projet devient une
erreur de compilation. Vérifié en pratique à l'étape 4 : le nom de crate ne
résout même pas (`E0432`), avant même qu'une règle de couches ait à
s'appliquer.

### Dans `harness-core`

| Module | Porte | Note |
| --- | --- | --- |
| `domain/` | `Halt`, `Verdict`, `ActionResult`, `Workspace` | pur, aucun I/O |
| `traces/` | `Logbook`, `Tally`, métriques | ex-`runtime/monitoring/` |
| `adapters/` | agent, git, gh, notify, store | l'extérieur, emballé |
| `execution/` | `Context<S>`, les deux traits, `Gate`, la séquence | |

- `adapters/` reste un emballage entre le code et les appels externes. Une
  porte décide, elle n'appelle jamais `gh`/`git`/`which` elle-même.
- `runtime/` disparaît comme nom. Son `monitoring/` devient `traces/`.

## Décisions verrouillées

| # | Décision | Pourquoi |
| --- | --- | --- |
| 1 | **Deux traits** : `Executable` (`&mut Context`), `Verification` (`&Context`) | le borrow checker rend vraie la règle « Action fait, Verification juge » |
| 2 | **`Context<S>` générique** sur un `State` déclaré par le workflow | aucun downcast ; `core` reste ignorant des workflows, vérifié à la compilation |
| 3 | **Async, tokio `current_thread`**, `#[async_trait(?Send)]` | l'agent est async ; le harness pilote une session à la fois, donc pas de borne `Send` à payer |
| 4 | **Trois crates dans un workspace** | la règle de couches devient une erreur de compilation |
| 5 | **`.execute()` rend du contrôle, pas de la donnée** | découle de « le Context transporte l'état » + « l'exécuteur porte les résultats d'action » |
| 6 | **`Settings` immuable** | construits une fois, jamais réécrits en place |
| 7 | **Une gate entoure un exécutable**, à tous les niveaux | valider unitairement (autour d'une action) ou intégré (autour d'une stage, d'un round) |
| 8 | **La session payante reste une `Stage`**, qui porte un objet `Session` | plusieurs actions interopèrent avec la même session ouverte |
| 9 | **La `Stage` est le point de variation** : session, ou locale | le Round tient strictement des Stages ; la hiérarchie du diagramme est tenue par les types |
| 10 | **Premier workflow : la boucle de dev** | elle exerce Round + budget + reprise, donc le plus de core |
| 11 | **Le round est du code, la séquence est une table** (variante B) | le dépôt a déjà retiré un moteur de graphe parce que la couche déclarative mentait sur l'exécution |
| 12 | **Le core compose le prompt** depuis une borne `Scoped` sur l'état | « une session ne démarre jamais sans sa portée » devient une garantie de type |
| 13 | **Le `Workflow` compte les tours**, le `Round` reçoit son numéro | le budget est un `Settings` ; un round n'a pas à savoir qu'il est le 3ᵉ de 5 |
| 14 | **`Context` porte un port de reprise** ; `ctx.checkpoint()` y délègue | le core n'écrit jamais sur le disque lui-même |

### Sur l'async

`async fn` en trait est stable depuis Rust 1.75 mais **n'est pas
dyn-compatible** : la taille du futur rendu est inconnue. Un Stage qui tient
`Vec<Box<dyn Executable<S>>>` en a besoin, donc `#[async_trait]`, qui désugare
en `Pin<Box<dyn Future>>`. Une allocation par `execute()` — négligeable quand
une action est une session de plusieurs minutes.

`?Send` + `current_thread` parce que rien n'est parallèle : ça retire les
`Arc<Mutex<_>>` du Context et laisse le heartbeat en `spawn_local`. Le jour où
une vraie parallélisation apparaît, c'est là qu'on paie `Send`.

## La forme du core

### `Halt` — les arrêts sont des valeurs

```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum Halt {
    #[error("{0}")] Halted(String),      // arrêt volontaire
    #[error("{0}")] Unreadable(String),  // un magasin n'a pas répondu
    #[error("{0}")] Failed(String),      // rien d'utilisable rendu
    #[error("{0}")] Quota(String),       // fenêtre d'abonnement épuisée
}

pub type Outcome<T> = Result<T, Halt>;
```

- Les quatre variantes, leurs codes de sortie, leurs préfixes de journal et
  leurs niveaux sont **inchangés** : un ordonnanceur extérieur les lit.
- `Halted` et `Unreadable` restent séparés. `[]` se lit « plus rien à faire »,
  et le confondre avec « illisible » ferait payer un `/planner` pour un jeton
  expiré.
- **Piège** : `Err` en Rust se lit « erreur », et un `Halted` est un résultat
  *correct* (code 1, niveau info). Le rapport ne doit pas appeler ça un échec.

### `Verdict`

```rust
pub enum Verdict {
    Continue,            // j'ai fait mon travail, enchaîne
    Skip(String),        // je n'avais rien à faire ici, enchaîne
    NothingLeft(String), // il n'y a plus rien à faire du tout, arrête proprement
}
```

- `Err(halt)` — s'arrêter. **`Stop` n'est pas une variante** : c'est `Err`, et
  `?` le propage.
- `Continue` et `Skip` absorbent le `skip` trivalué et les `before`/`after`
  bivalués du Python.
- **`NothingLeft` est né de la décision n°13.** Demander qui compte les tours a
  révélé le trou : `Repeat` porte aujourd'hui l'écart entre « budget épuisé » et
  « plus rien à faire » dans un `Result[bool]`, et ce second cas est un **succès**
  (code 0), pas un arrêt. Sans cette variante il n'avait nulle part où aller —
  ni `Continue`, ni `Skip`, ni `Err`. Seul un `Round` l'émet ; la répétition
  s'arrête dessus, journalise la raison, et rend un succès.

### `Context<S>`

```rust
pub struct Context<S> {
    settings: Settings,   // immuable après construction
    state: S,             // ce que le workflow déclare
    traces: Logbook,
    tally: Tally,
}
```

- `Settings` et `state` portent chacun leur donnée. Le Context les transporte ;
  une `Verification` les atteint par `&Context`.
- **`Settings` devient immuable.** Aujourd'hui `provisioning.mount()` réécrit
  `cfg.workspace` en place. Ici le montage rend les `Settings` définitifs
  *avant* que le Context existe : `mount(settings) -> Outcome<(Settings, Mount)>`.
- Remplace `Ctx` + `RoundCtx(Ctx)` + `RoundState`. L'héritage du Python n'a pas
  d'équivalent ; le paramètre `S` le remplace sans cast.

### Les deux traits

```rust
#[async_trait(?Send)]
pub trait Executable<S> {                    // Workflow, Round, Stage, Action
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

#[async_trait(?Send)]
pub trait Verification<S> {                  // Gate, Verification
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict>;
}
```

- Une `Verification` reçoit `&Context` : **écrire ne compile pas.** C'est la
  table des responsabilités, tenue par le compilateur.
- Les paramètres de type ne cassent pas la dyn-compatibilité (seuls les `async
  fn` nus et les méthodes génériques le font), donc `Box<dyn Executable<S>>`
  reste possible.

### `Gate` est une `Verification`

```rust
pub struct Gate<S> {
    pub name: &'static str,
    pub checks: Vec<Box<dyn Verification<S>>>,
}

#[async_trait(?Send)]
impl<S> Verification<S> for Gate<S> {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        for check in &self.checks {
            if let Verdict::Skip(why) = check.verify(ctx).await? {
                return Ok(Verdict::Skip(why));
            }
        }
        Ok(Verdict::Continue)
    }
}
```

- Une Gate implémente ce qu'elle orchestre : elle se compose donc d'elle-même.
- `?` donne « la première qui échoue gagne ». C'est `verify_all()` en six
  lignes, et comme objet plutôt que fonction libre — donc journalisable et
  testable d'une pièce.

### Les gates entourent un exécutable

Une gate ne vit pas *dans* la séquence : elle entoure un exécutable, à
n'importe quel niveau. Autour d'une action, elle valide unitairement ; autour
d'une stage ou d'un round, elle valide l'intégré.

```rust
pub trait Executable<S> {
    fn gates(&self) -> Gates<'_, S> { Gates::none() }
    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

// Blanket impl : non implémentable à la main, donc non contournable.
#[async_trait(?Send)]
impl<S, E: Executable<S> + ?Sized> Guarded<S> for E {
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        // pré → perform → post. Écrit ici, et nulle part ailleurs.
    }
}
```

- Le séquenceur n'appelle que `execute()`. Un niveau ne peut pas livrer sans
  ses gates : une impl spécifique entrerait en conflit avec le blanket impl.
- C'est ce que `contract/workflow.py` décrit comme « quinze lignes, jamais
  réécrites » — tenu par le compilateur au lieu de la discipline.
- Une pré-gate qui rend `Skip(why)` journalise et **absorbe** : le corps ne
  tourne pas, le parent reçoit `Continue` et enchaîne. C'est le `skip`
  d'aujourd'hui.
- **Ce que ça supprime** : les callables `skip`/`before`/`after`/`excluded`/
  `tolerate` de `run_sequence`, et le protocole `Step` qui les portait. Le
  séquenceur redevient une boucle sur des exécutables.

### La stage et sa session

Une `Stage` a deux formes, et c'est le seul point de variation de la
hiérarchie. Le `Round` tient strictement des `Stage`.

```rust
pub struct Stage<S> {
    pub name: String,
    pub pre: Option<Gate<S>>,     // les gates au niveau de la stage,
    pub post: Option<Gate<S>>,    // quelle que soit sa forme
    pub body: StageBody<S>,
}

pub enum StageBody<S> {
    Session { spec: SessionSpec, actions: Vec<Box<dyn SessionAction<S>>> },
    Local   { actions: Vec<Box<dyn Action<S>>> },
}
```

- Les gates sont sur `Stage`, pas sur le corps : une gate entoure un
  exécutable, et l'exécutable est la Stage.
- Un ensemble fermé à deux variantes, à dessein : le match est exhaustif, et
  un workflow ne peut pas inventer une troisième façon de faire tourner une
  stage.
- `pick_task` devient une `StageBody::Local` à une action.

**Une session ouverte, plusieurs actions.** C'est la différence de fond avec le
Python : une stage payante n'est plus un prompt et un résultat, c'est une
session tenue ouverte contre laquelle plusieurs actions dialoguent.

```rust
// Ce qu'une action de session reçoit. Deref vers Context<S>.
pub struct Open<'a, S> {
    ctx: &'a mut Context<S>,
    session: &'a mut Session,
}

pub trait SessionAction<S> {
    async fn perform(&self, open: &mut Open<S>) -> Outcome<Verdict>;
}
```

- **La `Session` ne quitte jamais sa Stage.** Elle n'est pas dans le Context,
  donc rien en dehors d'une stage ne peut l'atteindre — garanti par les types,
  pas par une convention.
- **Ce que ça supprime** : `StageSpec.lead`. Le champ existait pour cramer
  `/tech-analyst` et `/code` dans un seul prompt « parce que le plan du
  tech-analyst n'est écrit dans aucun fichier — un processus neuf le
  jetterait ». Deux actions de session dans une stage sont ce que `lead`
  imitait.
- **Ce que ça préserve** : la session restant au niveau de la stage, la
  comptabilité aussi. `costs.tsv` garde sa ligne par stage, format gelé
  compris ; les tours s'y additionnent.

## Ce que Rust supprime — à ne pas migrer

La part de `core/` qui n'existe que pour compenser Python.

| Ce qui disparaît | Lignes | Remplacé par |
| --- | --- | --- |
| `Result[T]`, `Status`, `recast()`, `map()`, `but()` | 165 | `Result<T, Halt>` et `?` ; `map` est dans la stdlib |
| `steps.run_sequence` et ses 7 callables optionnels | 103 | une boucle `for` avec `?` |
| Le protocole `Step` et ses trois gardes | — | les gates entourent l'exécutable |
| `shapes.perform` — « le seul point qui distingue `StageSpec` de `Action` » | — | une Stage est une Stage, une Action une Action : les types les distinguent |
| `StageSpec.lead` | — | deux actions de session dans une stage |
| `tests/test_layering.py` + table `ALLOWED` | — | le graphe de crates |
| Le chemin rapide entier | — | un binaire compilé démarre |

**Le chemin rapide était une contrainte Python.** Les ~130 ms de `--status`, les
imports tardifs (`_round`, `provisioning`), les `if TYPE_CHECKING:` de `shapes/`
et `design/`, `test_declaring_a_workflow_does_not_load_the_engine`, et la
`Route` écrite à la main pour éviter 7 ms de `inspect` : **rien de tout ça ne se
migre.** Ces contraintes ont façonné `design/` ; elles n'ont plus d'objet.

## Ce que « deux traits » impose : l'inventaire des gardes

Les sept gardes du round, triées. Cinq passent telles quelles.

| Garde | Ce qu'elle fait | Devient |
| --- | --- | --- |
| `spec_already_written` | lit `st.spec_written` | `Verification` |
| `code_already_delivered` | lit issue + PR mergées | `Verification` |
| `already_delivered` | lit issue + PR mergées | `Verification` |
| `code_has_a_spec` | lit `st.task_body` | `Verification` |
| `planner_opened_a_task` | relit le tableau | `Verification` |
| `spec_is_in_the_issue` | lit, **pose `spec-written`**, **mute l'état** | à scinder |
| `task_is_delivered` | lit, **pose `waiting-merge`** | à scinder |

Plus `Once.precheck`, qui remplit l'état par contrat explicite — donc une
`Action`, pas une vérification.

Trois points à scinder. Le coût de la décision n°1 est borné et connu.

## Ce qui reste à trancher

### Ce qui porte une `Session` (étape 4)

`Session` est une interface dans `adapters/agent/`. Ce qui la porte est derrière
la couture, donc ce choix ne perturbe ni le core ni les workflows. Trois
candidats, et l'arbitrage est entre fidélité du résultat et agnosticisme.

| Porteur | Gagne | Coûte |
| --- | --- | --- |
| **tmux** — un pane par session, `send-keys` / `capture-pane` | l'agent devient interchangeable (claude, opencode, …) ; une stage est attachable en cours de run ; aucun protocole à écrire | `capture-pane` rend un terminal : ANSI, spinners, lignes re-rendues. La fin d'un tour est fragile à détecter, et **le coût et l'usage ne sont pas récupérables** |
| **stream-json** — `claude -p --input-format stream-json` | des tours délimités, le coût et l'usage dans le message `result`, l'acquittement par `--replay-user-messages` | lié à Claude Code ; un protocole à parler, et c'est le plus gros adaptateur de la migration |
| **un processus par action** — `--session-id` / `--resume` | le plus simple à écrire | paie le démarrage et le rechargement de session par action ; « ouverte » devient une fiction |

- Sous tmux, les marqueurs `AGENT_LOOP_OK`/`AGENT_LOOP_STOP` — aujourd'hui un
  « contrat verbal » avec repli structurel — deviendraient le signal de
  complétion **primaire**. C'est tenable, mais ça déplace le poids.
- Sous tmux, `tmux` s'ajoute aux binaires du préflight, comme `claude` l'est.
- Il n'existe pas de SDK Rust officiel. C'est le vrai coût de quitter Python :
  `adapters/agent/` passe d'« emballer une bibliothèque » à « parler à quelque
  chose ».

C'est le seul arbitrage encore ouvert. Tout le reste est tranché.

## Ce qui a été tranché en route

- **`Workspace` descend dans `domain/`** — c'est de l'algèbre de chemins pure,
  qui « déclare et ne monte pas ». **`repo_root()` et `claim()` montent dans
  `adapters/`** : ils font du vrai I/O. L'ex-`filesystem/` est coupé selon la
  ligne de pureté, au lieu de rester un fourre-tout.
- **`traces/` reste une feuille.** Elle ne connaît ni `Verdict` ni `Halt` :
  l'exécuteur lit `halt.prefix()` et `halt.level()`, et passe une chaîne. C'est
  déjà ainsi que le Python fonctionne.
- **`design/` ne survit pas.** `Blueprint` + `build` existaient pour éviter
  quatre classes d'emballage recopiées *et* pour garder le chemin rapide. Le
  chemin rapide n'existe plus ; le blanket impl écrit l'emballage une fois ; et
  en variante B un workflow écrit son propre `Round`. Il ne reste rien à
  déclarer.
- **`Context<S>.results` disparaît.** Dans une stage, la session porte ce qu'une
  action a rendu à la suivante. Ce qui doit survivre à la stage, une action le
  promeut **explicitement** dans l'état. Plus de dict indexé par nom de skill.
- **L'idempotence est au core**, via une borne `Resumable` sur l'état
  (`done()`, `mark()`). Ça supprime le `done=st.stages_done` passé à chaque
  appel et le cas particulier `mark=False`.
- **Deux choses sont du framework et ne se déclarent plus** : le filtre
  `--stages` (`Settings.stages` contre `stage.name`) et la résolution
  modèle/effort (la table déclare le défaut, `MODEL`/`EFFORT` l'écrasent).

## Étapes

1. ~~Décisions structurantes~~ — faite.
2. ~~La forme du core~~ — faite : les deux traits, les gates autour d'un
   exécutable, la stage et sa session, la hiérarchie stricte.
3. ~~Le core par l'usage~~ — faite : `docs/ROUND-DRAFT.md`, variante B retenue.
4. **Le squelette du workspace** : trois crates, `domain/`, `traces/` et les
   traits d'exécution. `Stage`/`Round` attendent l'étape 5 : ils tiennent une
   `Session`, dont le porteur n'est pas tranché.
5. Le port `Session` et son porteur, puis les adaptateurs dont la boucle a
   besoin.
6. La boucle, puis le launcher qui la trigge.
