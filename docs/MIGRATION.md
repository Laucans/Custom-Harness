# Pipeline to Rust Harness Migration

Working document, written as decisions are made. It records decisions taken
and their implications — not a narrative of the migration.

Source: `event_assistant/pipeline/` (Python, 121 modules, 667 test cases).
Its map: `pipeline/ARCHITECTURE.md` and `pipeline/src/pipeline/core/ARCHITECTURE.md`.

## Purpose

Rewrite the runner in Rust, named **harness** — not "pipeline": it does not merely
move data through stages, it drives agent sessions through verification gates, which
is precisely what the term means in the code agent ecosystem (loop, observation,
actions, verification, state persistence). And take the opportunity to fix three
things Python let accumulate:

- execution vocabulary is implicit — there is no `Round` type, nor an executable
  `Stage` type; they are closures passed to `shape`s;
- execution state lives in three overlapping objects (`Ctx`, `RoundCtx(Ctx)`,
  `RoundState`);
- "verify" and "do" are mixed: two guards write to GitHub.

## The three crates

The directory structure is preserved. It becomes a crate graph in a Cargo
workspace:

```
harness-launcher (bin)  ->  harness-workflows (lib)  ->  harness-core (lib)
     what triggers              the definition of             the framework
                                    workflows
```

**What it replaces**: `tests/test_layering.py` and its `ALLOWED` table.
"The framework ignores its users" stops being a test that traverses ASTs — a
`use harness_workflows::` in `harness-core` is a cyclic dependency, and Cargo
refuses to compile. Invariant #1 of the project becomes a compilation error.
Verified in practice at step 4: the crate name doesn't even resolve (`E0432`),
before any layering rule even applies.

### Inside `harness-core`

| Module | Exports | Note |
| --- | --- | --- |
| `domain/` | `Halt`, `Verdict`, `ActionResult`, `Workspace` | pure, no I/O |
| `traces/` | `Logbook`, `Tally`, metrics | formerly `runtime/monitoring/` |
| `adapters/` | agent, git, gh, notify, store | the outside, wrapped |
| `execution/` | `Context<S>`, both traits, `Gate`, sequencing | |

- `adapters/` remains a wrapper between code and external calls. A gate
  decides; it never calls `gh`/`git`/`which` itself.
- `runtime/` disappears as a name. Its `monitoring/` becomes `traces/`.

## Locked-in decisions

| # | Decision | Why |
| --- | --- | --- |
| 1 | **Two traits**: `Executable` (`&mut Context`), `Verification` (`&Context`) | the borrow checker makes true the rule "Action does, Verification judges" |
| 2 | **`Context<S>` generic** over a `State` declared by the workflow | no downcast; `core` remains ignorant of workflows, verified at compile time |
| 3 | **Async, tokio `current_thread`**, `#[async_trait(?Send)]` | the agent is async; the harness drives one session at a time, so no `Send` bound to pay |
| 4 | **Three crates in a workspace** | the layering rule becomes a compilation error |
| 5 | **`.execute()` returns control, not data** | follows from "Context transports state" + "executor carries action results" |
| 6 | **`Settings` immutable** | constructed once, never rewritten in place |
| 7 | **A gate surrounds an executable**, at all levels | validate in isolation (around an action) or integrated (around a stage, round) |
| 8 | **Paid session remains a `Stage`**, carrying a `Session` object | multiple actions interoperate with the same open session |
| 9 | **`Stage` is the variation point**: session or local | Round strictly holds Stages; diagram hierarchy is held by types |
| 10 | **First workflow: the dev loop** | it exercises Round + budget + resumption, most of core |
| 11 | **Round is code, sequence is a table** (variant B) | repo already removed a graph engine because the declarative layer lied about execution |
| 12 | **Core composes prompt** from a `Scoped` bound on state | "a session never starts without scope" becomes a type guarantee |
| 13 | **`Workflow` counts turns**, `Round` receives its number | budget is a `Settings`; a round doesn't need to know it's the 3rd of 5 |
| 14 | **`Context` carries a resumption port**; `ctx.checkpoint()` delegates | core never writes to disk itself |

### On async

`async fn` in a trait is stable since Rust 1.75 but **is not dyn-compatible**:
the size of the returned future is unknown. A Stage holding `Vec<Box<dyn
Executable<S>>>` needs it, hence `#[async_trait]`, which desugars to
`Pin<Box<dyn Future>>`. One allocation per `execute()` — negligible when an
action is a multi-minute session.

`?Send` + `current_thread` because nothing is parallel: it removes `Arc<Mutex<_>>`
from Context and leaves heartbeat as `spawn_local`. When real parallelization
appears, that's where `Send` gets paid.

## Core shape

### `Halt` — stops are values

```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum Halt {
    #[error("{0}")] Halted(String),      // intentional stop
    #[error("{0}")] Unreadable(String),  // a store did not respond
    #[error("{0}")] Failed(String),      // nothing usable returned
    #[error("{0}")] Quota(String),       // subscription window exhausted
}

pub type Outcome<T> = Result<T, Halt>;
```

- The four variants, their exit codes, their log prefixes, and their levels are
  **unchanged**: an external scheduler reads them.
- `Halted` and `Unreadable` remain separate. `[]` reads as "nothing left to do",
  and confusing it with "unreadable" would charge a `/planner` for an expired token.
- **Trap**: `Err` in Rust reads "error", and a `Halted` is a *correct* result
  (exit code 1, info level). The report must not call it a failure.

### `Verdict`

```rust
pub enum Verdict {
    Continue,            // I did my job, continue
    Skip(String),        // I had nothing to do here, continue
    NothingLeft(String), // there's nothing left to do, stop cleanly
}
```

- `Err(halt)` — stop. **`Stop` is not a variant**: it's `Err`, and `?` propagates it.
- `Continue` and `Skip` absorb Python's three-valued `skip` and two-valued `before`/`after`.
- **`NothingLeft` was born from decision #13.** Asking who counts turns revealed the
  gap: `Repeat` today carries the difference between "budget exhausted" and "nothing
  left to do" in a `Result[bool]`, and that second case is a **success** (exit code 0),
  not a stop. Without this variant it had nowhere to go — neither `Continue`, nor
  `Skip`, nor `Err`. Only a `Round` emits it; repetition stops on it, logs the reason,
  and returns success.

### `Context<S>`

```rust
pub struct Context<S> {
    settings: Settings,   // immutable after construction
    state: S,             // what the workflow declares
    traces: Logbook,
    tally: Tally,
}
```

- `Settings` and `state` each carry their data. Context transports them;
  a `Verification` reaches them via `&Context`.
- **`Settings` becomes immutable.** Today `provisioning.mount()` rewrites
  `cfg.workspace` in place. Here the mounting makes `Settings` final *before*
  Context exists: `mount(settings) -> Outcome<(Settings, Mount)>`.
- Replaces `Ctx` + `RoundCtx(Ctx)` + `RoundState`. Python's inheritance has no
  equivalent; the `S` parameter replaces it without casting.

### The two traits

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

- A `Verification` receives `&Context`: **writing won't compile.** This is the
  responsibility table, held by the compiler.
- Type parameters don't break dyn-compatibility (only bare `async fn` and
  generic methods do), so `Box<dyn Executable<S>>` remains possible.

### `Gate` is a `Verification`

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

- A Gate implements what it orchestrates: it composes itself.
- `?` gives "first one to fail wins". It's `verify_all()` in six lines, and as
  an object rather than a free function — so loggable and testable as one piece.

### Gates surround an executable

A gate doesn't live *in* the sequence: it surrounds an executable, at any level.
Around an action, it validates in isolation; around a stage or round, it validates
the integrated result.

```rust
pub trait Executable<S> {
    fn gates(&self) -> Gates<'_, S> { Gates::none() }
    async fn perform(&self, ctx: &mut Context<S>) -> Outcome<Verdict>;
}

// Blanket impl: not implementable by hand, so not bypassable.
#[async_trait(?Send)]
impl<S, E: Executable<S> + ?Sized> Guarded<S> for E {
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        // pre → perform → post. Written here, and nowhere else.
    }
}
```

- The sequencer only calls `execute()`. A level can't deliver without its gates:
  a specific impl would conflict with the blanket impl.
- This is what `contract/workflow.py` describes as "fifteen lines, never
  rewritten" — held by the compiler instead of discipline.
- A pre-gate returning `Skip(why)` logs and **absorbs**: the body doesn't run,
  the parent receives `Continue` and continues. This is today's `skip`.
- **What it removes**: the callables `skip`/`before`/`after`/`excluded`/`tolerate`
  from `run_sequence`, and the `Step` protocol that carried them. The sequencer
  becomes a loop over executables again.

### The stage and its session

A `Stage` has two forms, and it's the only variation point in the hierarchy.
`Round` strictly holds `Stage`s.

```rust
pub struct Stage<S> {
    pub name: String,
    pub pre: Option<Gate<S>>,     // gates at the stage level,
    pub post: Option<Gate<S>>,    // regardless of its form
    pub body: StageBody<S>,
}

pub enum StageBody<S> {
    Session {
        spec: SessionSpec,
        sessions: Rc<dyn SessionFactory>,   // the coupling; see below
        actions: Vec<Box<dyn SessionAction<S>>>,
    },
    Local { actions: Vec<Box<dyn Action<S>>> },
}
```

- Gates are on `Stage`, not on the body: a gate surrounds an executable, and
  the executable is the Stage. **Tested**: a pre-gate that fails stops before
  even calling `sessions.open()` — no session opens for a skipped stage.
- A closed set to two variants by design: the match is exhaustive, and a
  workflow can't invent a third way to run a stage.
- `pick_task` becomes a `StageBody::Local` with one action.
- **`sessions` lives on `StageBody::Session`, not on `Context`.** A local stage
  never has to carry a factory it doesn't use; the reach of "who can access a
  session" stays as narrow as the next rule wants — even the factory stays scoped
  to the stage that needs it.

**One session open, multiple actions.** This is the fundamental difference from Python:
a paid stage is no longer a prompt and a result, it's a session held open that
multiple actions dialogue with.

```rust
// What a session action receives. Derefs to Context<S>.
pub struct Open<'a, S> {
    pub ctx: &'a mut Context<S>,
    pub session: &'a mut dyn Session,
}

#[async_trait(?Send)]
pub trait SessionAction<S> {
    async fn run(&self, open: &mut Open<'_, S>) -> Outcome<Verdict>;
}

// The port, in adapters/agent/ — mirror of AgentRunner (ABC) on Python side:
// an interface decided now, a concrete carrier decided later.
#[async_trait(?Send)]
pub trait Session {
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply>;
}

pub struct Reply {
    pub text: String,
    pub stop_line: Option<String>,
    // Optional by design: a terminal pane doesn't report usage, unlike a
    // structured JSON stream. The trait doesn't presuppose the most generous
    // carrier's capability — else the interface would already lean toward
    // stream-json before the choice is made.
    pub cost: Option<f64>,
}

#[async_trait(?Send)]
pub trait SessionFactory {
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>>;
}
```

- **The `Session` never leaves its Stage.** It's not in Context, so nothing
  outside a stage can reach it — guaranteed by types, not convention. **Tested**:
  two `SessionAction`s in the same `Stage` share a single call to
  `sessions.open()`, so one session for both turns.
- **What it removes**: `StageSpec.lead`. The field existed to cram `/tech-analyst`
  and `/code` into a single prompt "because the tech-analyst plan isn't written
  in any file — a new process would discard it". Two session actions in a stage
  are what `lead` imitated.
- **What it preserves**: the session staying at stage level, accounting stays too.
  `costs.tsv` keeps its line per stage, frozen format included; turns add up.
- **Generic `Round<S>` exists in `harness-core`**: `Vec<Stage<S>>` + post-gate,
  for workflows without branching. The dev loop won't use it — its round branches
  (rollover), and variant B (`docs/ROUND-DRAFT.md`) makes it write its own type.

## What Rust removes — don't migrate

The part of `core/` that exists only to compensate for Python.

| What goes away | Lines | Replaced by |
| --- | --- | --- |
| `Result[T]`, `Status`, `recast()`, `map()`, `but()` | 165 | `Result<T, Halt>` and `?`; `map` is in stdlib |
| `steps.run_sequence` and its 7 optional callables | 103 | a `for` loop with `?` |
| The `Step` protocol and its three guards | — | gates surround the executable |
| `shapes.perform` — "the only thing distinguishing `StageSpec` from `Action`" | — | a Stage is a Stage, an Action an Action: types distinguish them |
| `StageSpec.lead` | — | two session actions in a stage |
| `tests/test_layering.py` + `ALLOWED` table | — | the crate graph |
| The entire fast path | — | a compiled binary starts |

**The fast path was a Python constraint.** The ~130 ms of `--status`, late imports
(`_round`, `provisioning`), `if TYPE_CHECKING:` in `shapes/` and `design/`,
`test_declaring_a_workflow_does_not_load_the_engine`, and the hand-written `Route`
to avoid 7 ms of `inspect`: **none of that migrates.** These constraints shaped
`design/`; they no longer matter.

## What "two traits" demands: inventory of guards

Round's seven guards, sorted. Five pass through as-is.

| Guard | What it does | Becomes |
| --- | --- | --- |
| `spec_already_written` | reads `st.spec_written` | `Verification` |
| `code_already_delivered` | reads issue + merged PR | `Verification` |
| `already_delivered` | reads issue + merged PR | `Verification` |
| `code_has_a_spec` | reads `st.task_body` | `Verification` |
| `planner_opened_a_task` | re-reads the board | `Verification` |
| `spec_is_in_the_issue` | reads, **sets `spec-written`**, **mutates state** | to split |
| `task_is_delivered` | reads, **sets `waiting-merge`** | to split |

Plus `Once.precheck`, which fills state by explicit contract — so an `Action`,
not a verification.

Three points to split. The cost of decision #1 is bounded and known.

## What remains to decide

### What carries a `Session` (step 5)

`Session` is an interface in `adapters/agent/`, built at step 4 together with
`Stage`/`Round` — `Stage` only holds an `Rc<dyn SessionFactory>`, never a concrete
carrier. This choice doesn't disrupt core or workflows: `Stage`/`Round` are tested
(fakes in hand), written, and have nothing left to change when what follows is
decided. Three candidates, and the decision is between fidelity and agnosticism.

**The full report is in `docs/SESSION-CARRIER.md`** — three costed proposals, to arbitrate.
Summary, and **a correction**:

> Ce document affirmait que sous tmux « le coût et l'usage ne sont pas
> récupérables ». **C'est faux**, et l'erreur était de supposer que le pane est
> le seul canal. Le coût en USD exact, les jetons, les ratios de cache **et les
> quotas** sortent par quatre canaux structurés qui vivent à côté du terminal :
> `statusLine` (stdin JSON, `cost.total_cost_usd`, `rate_limits`), les métriques
> OTel (`claude_code.cost.usage`, en USD, exportable en local), le transcript
> JSONL (usage par message, mais **aucun** champ dollar), et le hook `Stop`
> (frontière de tour, en événement).

| Porteur | Gagne | Coûte |
| --- | --- | --- |
| **A — tmux + instrumentation** | attachable en vol ; coût USD, ratios de cache, **quotas prédictifs** ; conteneur générique | trois surfaces spécifiques à Claude Code à maintenir ; l'entrée reste des frappes dans une TUI |
| **B — stream-json, processus long** | un seul canal typé, en bande ; la vraie session ouverte ; marche sans TTY | non attachable ; un protocole bidirectionnel à écrire |
| **C — un processus par action (`--resume`)** | le moins de code ; coût USD et fin de tour gratuits par appel | « ouverte » est une fiction ; rechargement par action ; non attachable |

- **tmux n'achète pas l'agnosticisme d'agent**, contrairement à ce qu'on
  espérait : il achète un conteneur générique **plus** un port
  d'instrumentation à écrire par agent. `statusLine`, les noms de métriques,
  les hooks et le schéma du transcript sont tous propres à Claude Code.
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

## Steps

1. ~~Structural decisions~~ — done.
2. ~~Core shape~~ — done: both traits, gates around an executable, stage and its
   session, strict hierarchy.
3. ~~Core by usage~~ — done: `docs/ROUND-DRAFT.md`, variant B retained.
4. ~~Workspace skeleton~~ — done: three crates, `domain/`, `traces/`, `execution/`
   (`Context<S>`, both traits, `Gate`, `Stage`, generic `Round`), and the
   `adapters::agent::Session`/`SessionFactory` port. 23 tests. `Stage` holds only
   an `Rc<dyn SessionFactory>` — no concrete carrier wired.
5. ~~`Session` carrier and adapters~~ — done. Proposal **C** (`adapters::agent::claude_cli`,
   one `claude -p --output-format json` per action, stitched by `--resume`), with
   **A (tmux) as destination**. See `docs/SESSION-CARRIER.md`, including version trap
   on `total_cost_usd`. **Also done**: `domain::Spend`/`Tokens` (what a turn costs,
   all fields optional — `None` reads "not observed", never "zero"), `domain::Issue`,
   `adapters::shell::process` (the only place in the package that spawns a subprocess;
   a non-zero exit is a **datum**, not an error, because `git rev-parse --verify`
   answers that way), and `adapters::shell::git` (surface reduced to what the loop
   and its preflight ask for).

   **Also written**: `shell::github` (a failed read yields `Halt::Unreadable`,
   never `[]`), `store::ledger` (header frozen, empty distinct from zero),
   `store::checkpoint` (two-line pointer + JSONL, no sqlite). Step 5 is done.

   **What these three learned along the way:**

   - `clippy::future_not_send` (a `nursery` lint) is categorically incompatible
     with decision #3: it presupposes `Send` futures. An `allow` at `harness-core`
     root, justified in place, rather than scattered — see `CLAUDE.md`, which
     carries the exception.
   - `OnceLock` over `RefCell` for memoizing `owner/name`: same semantics (write
     once), but `Sync`, and crucially no borrow to keep open across `await`.

   **L'ancienne liste, pour mémoire :**

   | Quoi | Surface | Invariant à ne pas perdre |
   | --- | --- | --- |
   | `adapters::shell::github` | `authenticated`, `repo` (mémoïsé), `labels`, `issue`, `issues_labelled`, `sub_issues`, `blocked_by`, `merged_prs`, `issue_comments` (paginé), et les écritures `add_label`, `remove_label`, `set_body`, `post_issue_comment`, `close_issue` | **une lecture ratée n'est jamais `[]`** : `Halt::Unreadable`, jamais « plus rien à faire » — sinon un jeton expiré fait payer un `/planner`. Les issues passent par `gh api`, pas `gh issue` (les sous-issues et dépendances n'ont pas de flag natif). Filtrer les PR que `/issues` renvoie (clé `pull_request`). Étiquettes acceptées en objets **ou** en chaînes |
   | `adapters::store::ledger` | `append`, `rows` | en-tête **gelé caractère pour caractère**, colonnes ajoutées **en fin** de ligne, une ligne par stage. `outcome` vide = « non enregistré » |
   | `adapters::store::checkpoint` | pointeur 2 lignes (`task=`, `flow_id=`) + états | **ne pas porter le sqlite** : son schéma n'existait que par héritage du `@persist` d'un moteur de graphe, qui est mort. Du JSONL suffit et garde « une ligne par étape ». Un magasin illisible est `Unreadable`, jamais « rien n'a tourné » — l'écart vaut une session de `/code` |

   Volontairement hors étape 5 : le montage de workspace (`clone`, `fetch`,
   `reset --hard`, `clean`, `branches_at_risk`) et les lectures de PR
   (`pr`, `comment_bodies`, `post_comment`) qui servent la revue, pas la
   boucle.
6. ~~The loop, then the launcher that triggers it~~ — **done**. The first
   workflow in `harness-workflows`: `Loop` as state (`Resumable` + `Scoped`),
   the four task-selection rules, `Board` as pure data, the typed `TaskRound` of
   variant B, the table of three stages, and the split gates from the inventory
   above. Then workspace mounting and the launcher. **270 tests.**

   **Ce que l'étape a ajouté au core, et pourquoi :**

   | Quoi | Pourquoi là et pas ailleurs |
   | --- | --- |
   | `execution::guards` — `InThisRun`, `StageAlreadyDone`, `MarkDone` | les trois règles que **tout** stage subit, quel que soit le workflow. Côté Python c'étaient trois branches dans le corps de `StageRunner.run`, donc impossibles à retirer d'un stage, à tester seules, ou à lire dans la table. Les deux premières **jugent**, la troisième **fait** : la décision n°1 appliquée à une mécanique qui mélangeait les trois |
   | `execution::Unpaid<A>` | une action locale glissée dans une stage payante. Ce qui est dedans ne reçoit pas la session, donc **ne peut pas** dépenser. Un `impl` générique dirait la même chose sans l'enrobage, mais la cohérence le refuse — le compilateur ne sait pas prouver qu'un type n'implémente *pas* `Action` |
   | `adapters::store::spending` — le port, pas le fichier | `Ledger` sait écrire une ligne mais pas quelle heure il est. L'horloge, le nom du run et celui de la machine sont des faits du **lanceur**, et les y mettre garde `harness-core` **sans aucune dépendance au temps** |
   | `adapters::agent::rehearsal` | un dry-run devient un **choix de câblage**, pas une branche du framework. Côté Python chaque fonction qui exécutait portait son `if cfg.dry_run`, donc le chemin à blanc était un second parcours du code — celui qu'aucun test d'intégration ne couvre |
   | `adapters::shell::disk` | un port pour cinq opérations de fichiers, et sa seule raison est de rendre **testable le module qui supprime**. Un faux disque rend « ce test prouve qu'on ne supprime pas » vérifiable au lieu d'espérable |
   | `shell::git::Repos` | le montage parle à trois dépôts — la source, le dossier parent, le clone. Une seule couture de test les couvre |
   | `Logbook::warn`/`debug` | un workspace gardé n'arrête pas le run, et un `--quiet` qui l'avalerait laisserait du travail sur le disque sans que personne l'apprenne |

   **Ce que le portage a corrigé en passant :**

   - les consignes des stages **nommaient les étiquettes en dur** dans la prose
     du prompt (`pipeline:human`, `pipeline:waiting-merge`, …), à côté des
     constantes que le code lisait. Le renommage en `harness:*` aurait laissé
     les prompts réclamer des étiquettes mortes, et une session aurait obéi en
     en créant de nouvelles. Elles sont **épissées depuis `labels`** au montage
     de la table ; un futur renommage ne peut plus désynchroniser les deux ;
   - la garde « ce dossier est-il un workspace ? » passe désormais **avant**
     `--force-reset` : côté Python elle était derrière, c'est-à-dire absente du
     seul chemin destructeur ;
   - `Settings` étant immuable (décision n°6), le montage **rend** un `Mount`
     au lieu de réécrire `cfg.workspace` en place. Le hack que le Python
     assumait n'a plus d'objet ;
   - `clap` avec `env = …` met un argument et sa variable d'environnement au
     même endroit : « toute variable que le code lit apparaît dans `--help` »
     devient vrai **par construction**, là où le Python l'assérait dans un test.

   **Deux dépendances, et elles ne vivent que dans le lanceur** : `clap` pour
   ce qui précède, et `jiff` pour l'horloge — `harness-core` reste sans
   dépendance au temps parce qu'une ligne de registre **reçoit** son instant.

   **Pas de forme `Repeat` dans le core.** Le Python avait
   `core/execution/shapes/repeat.py`, une couche déclarative pour quinze lignes
   de `for`. Le compte de tours vit dans `dev_loop::workflow::DevLoop`, qui
   l'écrit. Le jour où un second workflow veut la même forme, elle se
   factorisera — avec deux exemples sous les yeux plutôt qu'un.

   **Vérifié en vrai** (`--dry-run --no-workspace --allow-dirty` depuis
   `event_assistant`, avec `claude` 2.1.285 sur le `PATH`) : la porte de
   version passe, `gh` répond, les portes de branche et de CI passent, la porte
   « arbre propre » arrête sur l'arbre réellement sale, et la porte des
   étiquettes arrête en nommant les sept `gh label create`. Le run s'arrête
   donc exactement là où `docs/CUTOVER.md` dit qu'un humain doit agir.

## What remains, after step 6

Nothing of the loop. What isn't ported is what didn't serve it:

- the two other workflows (PR review, issue refinement) and the hooks that
  trigger them;
- the GitHub adapter's PR reads (`pr`, `comment_bodies`, `post_comment`), which
  serve review;
- `--status` and `--costs`, the two read sub-commands;
- heartbeat and session progress traces. Under proposal C, a `claude -p` returns
  nothing until done: there's no stream to beat. It's the price of C, and proposal
  A (tmux) is what pays it;
- `PIPELINE_HOME`, which Python set so the review hook launched from the clone
  could find the package. No hooks in Rust yet, so no object for it.
