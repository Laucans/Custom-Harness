# Migration du pipeline vers Rust

Document de travail, écrit au fur et à mesure. Il porte les décisions prises
et ce qu'elles impliquent — pas un récit de la migration.

La source : `event_assistant/pipeline/` (Python, 121 modules, 667 cas de test).
Sa carte : `pipeline/ARCHITECTURE.md` et `pipeline/src/pipeline/core/ARCHITECTURE.md`.

## Objet

Réécrire le runner en Rust, et profiter du passage pour corriger trois choses
que le Python a laissées s'installer :

- le vocabulaire d'exécution est implicite — il n'y a pas de type `Round`, ni
  de type `Stage` exécutable ; ce sont des fermetures passées à des `shape`s ;
- l'état d'exécution vit dans trois objets qui se chevauchent (`Ctx`,
  `RoundCtx(Ctx)`, `RoundState`) ;
- « vérifier » et « faire » sont mélangés : deux gardes écrivent sur GitHub.

## Les trois crates

L'arborescence est conservée. Elle devient un graphe de crates, dans un
workspace Cargo :

```
pipeline-launcher (bin)  ->  pipeline-workflows (lib)  ->  pipeline-core (lib)
     ce qui trigge              la définition des             le framework
                                    workflows
```

**Ce que ça remplace** : `tests/test_layering.py` et sa table `ALLOWED`.
« Le framework ignore ses utilisateurs » cesse d'être un test qui parcourt des
AST — un `use pipeline_workflows::` dans `pipeline-core` est une dépendance
cyclique, et Cargo refuse de compiler. L'invariant n°1 du projet devient une
erreur de compilation.

### Dans `pipeline-core`

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
| 3 | **Async, tokio `current_thread`**, `#[async_trait(?Send)]` | l'agent est async ; le pipeline est séquentiel, donc pas de borne `Send` à payer |
| 4 | **Trois crates dans un workspace** | la règle de couches devient une erreur de compilation |
| 5 | **`.execute()` rend du contrôle, pas de la donnée** | découle de « le Context transporte l'état » + « l'exécuteur porte les résultats d'action » |

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
pub enum Verdict { Continue, Skip(String) }
```

- `Ok(Continue)` — enchaîner. `Ok(Skip(why))` — déjà fait, passer sans payer.
- `Err(halt)` — s'arrêter. **`Stop` n'est pas une variante** : c'est `Err`, et
  `?` le propage.
- Absorbe le `skip` trivalué et les `before`/`after` bivalués du Python.

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

## Ce que Rust supprime — à ne pas migrer

La part de `core/` qui n'existe que pour compenser Python.

| Ce qui disparaît | Lignes | Remplacé par |
| --- | --- | --- |
| `Result[T]`, `Status`, `recast()`, `map()`, `but()` | 165 | `Result<T, Halt>` et `?` ; `map` est dans la stdlib |
| `steps.run_sequence` et ses 7 callables optionnels | 103 | une boucle `for` avec `?` |
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

- **À quels niveaux une Gate s'attache.** Le Python en compte huit positions
  (préflight du workflow, `announce`, `obtained`, `skip`/`before`/`after` par
  étape, postcondition de round, `precheck`). Champ `pre`/`post` sur chaque
  exécutable, ou nœud dans la séquence ? Un `skip` doit tourner *avant*
  l'action qu'il annule, donc il la connaît.
- **La session payante est-elle une `Action` ?** Si oui, `shapes.perform` — « le
  seul point qui distingue `StageSpec` de `Action` » — disparaît, mais la
  comptabilité se déplace : `costs.tsv` a une ligne par *stage*, et `--stages`,
  `STAGES` et la colonne `stage` parlent l'ancien vocabulaire.
- **Où vont `Workspace`, `repo_root()` et `claim()`.** Proposition :
  `Workspace` est de l'algèbre de chemins pure et descend dans `domain/` ;
  `repo_root()` et `claim()` font du vrai I/O et montent dans `adapters/`.
  Ça coupe l'ex-`filesystem/` selon la ligne de pureté.
- **`traces/` reste-t-il une feuille ?** `runtime/` n'importe rien aujourd'hui.
  Si une Gate journalise son verdict, `traces/` doit-il connaître
  `Verdict`/`Halt`, ou reste-t-il aveugle et c'est l'exécuteur qui formate ?
- **`design/` survit-il ?** `Blueprint` + `build` existaient pour éviter quatre
  classes d'emballage recopiées *et* pour garder le chemin rapide. Le chemin
  rapide n'existe plus, et un `Workflow` devient lui-même un `Executable`.
- **`Once` / `Repeat` fusionnent-ils ?** Si `Round` est un niveau obligatoire,
  pr-review devient « un round, budget 1 ». Il faut garder l'écart que
  `Repeat` porte dans `Result[bool]` : « budget épuisé » ≠ « plus rien à
  faire ».
- **Quel workflow en premier.** Le premier workflow *est* l'usage qui façonne le
  core. La boucle exerce Round + budget + reprise ; pr-review exerce `precheck`
  + garde d'exclusion ; le raffinage exerce les rounds à sections.

## Étapes

1. ~~Décisions structurantes~~ — faite.
2. **Le core par l'usage** — décrire comment on veut l'écrire, côté workflow,
   et en déduire la forme définitive de `execution/`.
3. Le squelette du workspace : trois crates, `domain/` et `traces/` d'abord.
4. Les adaptateurs dont le premier workflow a besoin.
5. Le premier workflow, puis le launcher qui le trigge.
