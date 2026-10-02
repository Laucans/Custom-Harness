# The dev loop round, written

Step 3: the dev loop round as one would read it, in two variants. Nothing is
implemented — it's the declaration at stake, because it's what locks in the core
signatures.

Les types sont ceux de `MIGRATION.md`, et ils compilent : une sonde a validé
`Open<'a, S>` passé à un `dyn SessionAction`, le `Deref` vers `Context<S>`, le
blanket impl des gates, et deux actions de session dans une stage.
`async_trait` n'a demandé aucune borne sur `S`.

## What the round must do

Taken from `internals/round.py` and `stages/__init__.py`, nothing lost:

1. **pick the task** — read the board (or reuse what the loop read), place
   milestone and task in state, tell the ledger what task to charge. Three exits:
   one playable task, none but open `pipeline:agent`s (stop, naming the unblock
   gesture), none at all (rollover);
2. **place a resumption point** after the pick;
3. **branch**: rollover → `/planner`; else the three-stage sequence;
4. **the sequence** — `/business-analyst`, then `/tech-analyst` → `/code`, then
   `/create-test`, each with its gates;
5. **verify delivery** — a merged PR carries `Closes #N`, else stop.

The two hybrid gates are split, as decision #1 demands:

| Today | Verification | Action |
| --- | --- | --- |
| `spec_is_in_the_issue` | issue body is not empty | re-read body into state, set `pipeline:spec-written` |
| `task_is_delivered` | merged PR carries `Closes #N` | set `pipeline:waiting-merge` |

## What both variants share

```rust
// The round state. This is the `S` of Context<S>.
pub struct Loop {
    pub milestone: Milestone,
    pub task: Option<Task>,
    pub rollover: bool,
    pub spec_written: bool,
    pub done: Vec<String>,     // stages already done, all runs combined
}

// What core demands of state to know skip and mark.
pub trait Resumable {
    fn done(&self) -> &[String];
    fn mark(&mut self, stage: &str);
}
```

`Resumable` replaces the `done=st.stages_done` passed per call, and the `mark=False`
of the archive stage. **Core carries idempotence** — "at most once per task, all
runs combined" — as soon as it can read this list.

Deux autres choses sont du framework et n'apparaissent dans aucune des deux
déclarations :

- **le filtre `--stages`** : `Settings.stages` contre `stage.name`. Tous les
  workflows le veulent, aucun n'a à le déclarer ;
- **la résolution modèle/effort** : la table déclare le défaut, `MODEL` et
  `EFFORT` l'écrasent. C'est `cfg.resolve(spec)` d'aujourd'hui.

---

## Variant A — the round is a table

Everything is data. One kind of session action (`Ask`), carrying its command and
instructions. The rollover branch is a pre-gate on each stage.

```rust
pub fn round() -> Round<Loop> {
    Round {
        stages: vec![
            // 1. Choisir la task. Locale : rien n'est payé.
            Stage {
                name: "pick-task",
                pre: None,
                post: None,
                body: Local(vec![Box::new(PickTask)]),
            },

            // 2. Le rollover : /planner, seulement s'il n'y a aucune task.
            Stage {
                name: "planner",
                pre: Some(gate("rollover", vec![Box::new(OnlyIfRollover)])),
                post: Some(gate("a-ouvert-une-task",
                                vec![Box::new(PlannerOpenedATask)])),
                body: Session {
                    spec: spec("opus", "high"),
                    actions: vec![Box::new(Ask {
                        command: "/planner",
                        instructions: PLANNER,
                    })],
                },
            },

            // 3. La séquence. Chacune saute si on est en rollover.
            Stage {
                name: "business-analyst",
                pre: Some(gate("a-faire", vec![
                    Box::new(SkipIfRollover),
                    Box::new(SpecAlreadyWritten),
                ])),
                post: Some(gate("le-spec-est-dans-l-issue",
                                vec![Box::new(IssueBodyIsNotEmpty)])),
                body: Session {
                    spec: spec("opus", "high"),
                    actions: vec![
                        Box::new(Ask {
                            command: "/business-analyst",
                            instructions: BUSINESS_ANALYST,
                        }),
                        // L'autre moitié de l'ex-`spec_is_in_the_issue`.
                        Box::new(RecordSpecWritten),
                    ],
                },
            },

            Stage {
                name: "code",
                pre: Some(gate("a-faire", vec![
                    Box::new(SkipIfRollover),
                    Box::new(CodeAlreadyDelivered),
                    Box::new(CodeHasASpec),
                ])),
                post: None,
                body: Session {
                    spec: spec("opus", "high"),
                    // Ce que `lead` imitait : une session, deux commandes.
                    actions: vec![
                        Box::new(Ask { command: "/tech-analyst",
                                       instructions: TECH_ANALYST }),
                        Box::new(Ask { command: "/code",
                                       instructions: CODE }),
                    ],
                },
            },

            Stage {
                name: "create-test",
                pre: Some(gate("a-faire", vec![Box::new(SkipIfRollover)])),
                post: None,
                body: Session {
                    spec: spec("sonnet", "high"),
                    actions: vec![Box::new(Ask { command: "/create-test",
                                                 instructions: "" })],
                },
            },
        ],

        // La post-condition du round : scindée en deux.
        post: Some(gate("livree", vec![Box::new(AMergedPrClosesTheTask)])),
        then: Some(Box::new(MarkWaitingMerge)),
    }
}
```

**Ce que ça donne.** L'ordre de la table *est* l'ordre d'exécution, et la table
entière tient dans un écran. Aucun type nouveau par stage : ajouter une stage,
c'est ajouter une entrée. Les gates se lisent sur la même ligne que ce qu'elles
gardent — ce que `stages/__init__.py` prise explicitement.

**Ce que ça coûte.** Le branchement n'existe pas : il est simulé par un
`SkipIfRollover` répété sur trois stages, et un `OnlyIfRollover` sur la
quatrième. Un round en rollover fait donc tourner cinq stages dont trois
ne font rien, et « ce round ne va nulle part » se lit en trois lignes de
journal « sauté ». Le `post` du round a besoin d'un champ `then` pour porter
l'Action que la Verification ne peut plus faire.

---

## Variant B — the round is code

Core's generic `Round` serves simple cases. The loop writes its own: a type and a
`perform` that branches. The stage table remains a table.

```rust
// Les stages, en table — inchangé dans l'esprit de `stages/__init__.py`.
fn pipeline() -> Vec<Stage<Loop>> {
    vec![
        Stage {
            name: "business-analyst",
            pre: Some(gate("deja-ecrit", vec![Box::new(SpecAlreadyWritten)])),
            post: Some(gate("spec-dans-l-issue",
                            vec![Box::new(IssueBodyIsNotEmpty)])),
            body: Session {
                spec: spec("opus", "high"),
                actions: vec![
                    Box::new(Ask { command: "/business-analyst",
                                   instructions: BUSINESS_ANALYST }),
                    Box::new(RecordSpecWritten),
                ],
            },
        },
        Stage {
            name: "code",
            pre: Some(gate("a-faire", vec![
                Box::new(CodeAlreadyDelivered),
                Box::new(CodeHasASpec),
            ])),
            post: None,
            body: Session {
                spec: spec("opus", "high"),
                actions: vec![
                    Box::new(Ask { command: "/tech-analyst",
                                   instructions: TECH_ANALYST }),
                    Box::new(Ask { command: "/code", instructions: CODE }),
                ],
            },
        },
        Stage {
            name: "create-test",
            pre: None,
            post: None,
            body: Session {
                spec: spec("sonnet", "high"),
                actions: vec![Box::new(Ask { command: "/create-test",
                                             instructions: "" })],
            },
        },
    ]
}

/// Un round de la boucle : une task, ou le rollover.
pub struct TaskRound {
    pick: Stage<Loop>,
    pipeline: Vec<Stage<Loop>>,
    planner: Stage<Loop>,
    delivered: Gate<Loop>,
    mark: MarkWaitingMerge,
}

#[async_trait(?Send)]
impl Executable<Loop> for TaskRound {
    async fn perform(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        self.pick.execute(ctx).await?;
        ctx.checkpoint();

        // Le branchement, écrit. Pas simulé par des sauts.
        if ctx.state.rollover {
            return self.planner.execute(ctx).await;
        }

        for stage in &self.pipeline {
            stage.execute(ctx).await?;
            ctx.checkpoint();
        }

        // La post-condition, puis l'action qu'elle ne peut pas faire.
        self.delivered.verify(ctx).await?;
        self.mark.run(ctx).await
    }
}
```

**Ce que ça donne.** Le branchement est un `if`, lisible comme tel. Le point de
reprise est posé là où il a un sens, visiblement. La séquence des trois stages
reste une table qu'on réordonne en réordonnant ses lignes. Et l'ordre
pré-gate → corps → post-gate → action de suite se lit dans `perform`.

**Ce que ça coûte.** Le round de la boucle est un type à part, donc « qu'est-ce
que fait un round » demande d'ouvrir deux endroits : la table pour la séquence,
`perform` pour ce qui l'entoure. Et le `Round` générique du core ne sert plus la
boucle — il ne sert que les workflows sans branche.

---

## What each costs, side by side

| | A — table | B — code |
| --- | --- | --- |
| Rollover branch | three `SkipIfRollover` + one `OnlyIfRollover` | one `if` |
| A rollover round | 5 stages, 3 do nothing | 2 stages |
| Where to read sequence | one place | one place |
| Where to read its surround | nowhere: it's the table | `perform` |
| Resumption point | implicit, placed by core after each stage | explicit, `ctx.checkpoint()` |
| Action after post-gate | a `then` field on `Round` | one line of `perform` |
| New types per workflow | none | one (`TaskRound`) |
| Core's `Round` | serves the loop | only serves branchless workflows |

## Recommendation: B

Not for love of code over data, but because **the repo already decided this once.**
`internals/round.py` carries the account of experience A:

> Un `if/elif/else`. Le routeur du graphe avait trois sorties dont une muette,
> parce que c'était la façon la plus courte de dire « ce round ne va nulle
> part » à un moteur qui, sinon, enchaînait.

The graph engine was removed precisely because the declarative layer lied about
execution: the sequence lived in decorators, the table fed only a display, "and
the test verifying execution order passed anyway". Variant A reintroduces the same
thing smaller — a round whose real path is the resultant of four predicates spread
over five lines, instead of an `if` you read.

What A gets right, B keeps: the stage table, where entry order is execution order,
and a gate reads on the line of what it guards.

## What both leave open

- **Prompt composition.** `_extra(stage, ctx, st)` today mounts preamble + scope
  (milestone, issue, injector) + instructions. Does core compose it from a `Scoped`
  bound on state, or does each `Ask` mount it? The first guarantees no session
  starts without scope — what Python calls a rule and can't keep.
- **Who counts turns.** The `max_rounds` budget and the gap "budget exhausted" ≠
  "nothing left": in `Workflow`, or in `Round`?
- **`ctx.checkpoint()`**: does core write resumption state, or is it an adapter the
  round calls? It touches disk, so it comes from `adapters/store/`.
- **What a session action returns.** `Ask` returns `Verdict`, but the agent's reply
  goes where? In the session (re-readable by the next action), in state, or both?
