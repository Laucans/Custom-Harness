# Le round de la boucle, écrit

Étape 3 : le round de la boucle de dev tel qu'on le lirait, en deux variantes.
Rien n'est implémenté — c'est la déclaration qui est en jeu, parce que c'est
elle qui fige les signatures du core.

Les types sont ceux de `MIGRATION.md`, et ils compilent : une sonde a validé
`Open<'a, S>` passé à un `dyn SessionAction`, le `Deref` vers `Context<S>`, le
blanket impl des gates, et deux actions de session dans une stage.
`async_trait` n'a demandé aucune borne sur `S`.

## Ce que le round doit faire

Repris de `internals/round.py` et `stages/__init__.py`, sans rien perdre :

1. **choisir la task** — lire le tableau (ou réutiliser celui que la boucle a
   lu), poser milestone et task dans l'état, dire au registre sous quelle task
   facturer. Trois sorties : une task jouable, aucune mais des `pipeline:agent`
   ouvertes (arrêt, en nommant le geste qui débloque), aucune du tout
   (rollover) ;
2. **poser un point de reprise** après le choix ;
3. **brancher** : rollover → `/planner` ; sinon la séquence des trois stages ;
4. **la séquence** — `/business-analyst`, puis `/tech-analyst` → `/code`, puis
   `/create-test`, chacune avec ses gates ;
5. **constater la livraison** — une PR mergée porte `Closes #N`, sinon arrêt.

Les deux gates hybrides sont scindées, comme la décision n°1 l'impose :

| Aujourd'hui | Verification | Action |
| --- | --- | --- |
| `spec_is_in_the_issue` | le corps de l'issue n'est pas vide | relire le corps dans l'état, poser `pipeline:spec-written` |
| `task_is_delivered` | une PR mergée porte `Closes #N` | poser `pipeline:waiting-merge` |

## Ce que les deux variantes partagent

```rust
// L'état du round. C'est le `S` de Context<S>.
pub struct Loop {
    pub milestone: Milestone,
    pub task: Option<Task>,
    pub rollover: bool,
    pub spec_written: bool,
    pub done: Vec<String>,     // les stages déjà faites, tous runs confondus
}

// Ce que le core exige d'un état pour savoir sauter et marquer.
pub trait Resumable {
    fn done(&self) -> &[String];
    fn mark(&mut self, stage: &str);
}
```

`Resumable` remplace le `done=st.stages_done` passé à chaque appel, et le
`mark=False` du stage d'archivage. **Le core porte l'idempotence** — « au plus
une fois par task, tous runs confondus » — dès qu'il sait lire cette liste.

Deux autres choses sont du framework et n'apparaissent dans aucune des deux
déclarations :

- **le filtre `--stages`** : `Settings.stages` contre `stage.name`. Tous les
  workflows le veulent, aucun n'a à le déclarer ;
- **la résolution modèle/effort** : la table déclare le défaut, `MODEL` et
  `EFFORT` l'écrasent. C'est `cfg.resolve(spec)` d'aujourd'hui.

---

## Variante A — le round est une table

Tout est donnée. Une seule sorte d'action de session (`Ask`), qui porte sa
commande et ses consignes. Le branchement rollover est un pré-gate sur chaque
stage.

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

## Variante B — le round est du code

Le `Round` générique du core sert les cas simples. La boucle écrit le sien :
un type, et un `perform` qui branche. La table des stages reste une table.

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

## Ce que chacune coûte, côté à côté

| | A — table | B — code |
| --- | --- | --- |
| Le branchement rollover | trois `SkipIfRollover` + un `OnlyIfRollover` | un `if` |
| Un round en rollover | 5 stages, 3 qui ne font rien | 2 stages |
| Où lire la séquence | un endroit | un endroit |
| Où lire ce qui l'entoure | nulle part : c'est la table | `perform` |
| Le point de reprise | implicite, posé par le core après chaque stage | explicite, `ctx.checkpoint()` |
| L'action après une post-gate | un champ `then` sur `Round` | une ligne de `perform` |
| Types nouveaux par workflow | aucun | un (`TaskRound`) |
| Le `Round` du core | sert la boucle | ne sert que les workflows sans branche |

## Recommandation : B

Pas par goût du code sur la donnée, mais parce que **le dépôt a déjà tranché
cette question une fois.** `internals/round.py` porte le compte rendu de
l'expérience A :

> Un `if/elif/else`. Le routeur du graphe avait trois sorties dont une muette,
> parce que c'était la façon la plus courte de dire « ce round ne va nulle
> part » à un moteur qui, sinon, enchaînait.

Le moteur de graphe a été retiré précisément parce que la couche déclarative
mentait sur l'exécution : la séquence vivait dans les décorateurs, la table
n'alimentait qu'un affichage, « et le test qui vérifiait l'ordre d'exécution
passait quand même ». La variante A réintroduit la même chose en plus petit —
un round dont le vrai parcours est la résultante de quatre prédicats répartis
sur cinq lignes, au lieu d'un `if` qu'on lit.

Ce que A a de juste, B le garde : la table des stages, où l'ordre des entrées
est l'ordre d'exécution, et où une gate se lit sur la ligne de ce qu'elle garde.

## Ce que les deux laissent ouvert

- **La composition du prompt.** `_extra(stage, ctx, st)` monte aujourd'hui
  préambule + portée (milestone, issue, injecteur) + consignes. Est-ce que le
  core la compose depuis une borne `Scoped` sur l'état, ou est-ce que chaque
  `Ask` la monte ? La première garantit qu'aucune session ne part sans portée —
  ce que le Python appelle une règle et ne peut pas tenir.
- **Qui compte les tours.** Le budget `max_rounds` et l'écart « budget épuisé »
  ≠ « plus rien à faire » : dans le `Workflow`, ou dans le `Round` ?
- **`ctx.checkpoint()`** : le core écrit-il l'état de reprise, ou est-ce un
  adaptateur que le round appelle ? Il touche le disque, donc il vient de
  `adapters/store/`.
- **Ce que rend une action de session.** `Ask` rend `Verdict`, mais la réponse
  de l'agent va où ? Dans la session (relisible par l'action suivante), dans
  l'état, ou les deux ?
