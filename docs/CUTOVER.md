# La reprise en main — ce qu'un humain doit faire, et quand

Le harness Rust prend la main sur le suivi plutôt que de cohabiter : les
étiquettes passent de `pipeline:*` à `harness:*`, et l'état reprend
`.llocal/agent-loop/`. **Rien ici n'est fait automatiquement**, et c'est
délibéré : chacun de ces gestes arrête le pipeline Python.

## Avant de basculer

**Attendre qu'aucun round ne soit en vol.** Le Python garde l'état de reprise
d'un round dans `.llocal/agent-loop/flow_states.db`, en sqlite. Le harness Rust
**ne lit pas le sqlite** — il écrit du JSONL (`docs/MIGRATION.md`, décision de
l'étape 5). Un round interrompu au milieu, basculé, devient donc un round dont
plus personne ne sait quelles stages ont déjà tourné.

```bash
# Doit ne rien afficher : pas de pointeur = aucun round en cours.
cat .llocal/agent-loop/state 2>/dev/null
```

Si le fichier existe, finir ou abandonner le round avec le Python d'abord.

## Renommer les sept étiquettes

Un renommage GitHub **garde les issues déjà étiquetées** : rien à re-étiqueter
à la main. Mais le Python cesse de trouver son tableau à la première commande.

```bash
gh label edit "pipeline:roadmap"       --name "harness:roadmap"
gh label edit "pipeline:milestone"     --name "harness:milestone"
gh label edit "pipeline:agent"         --name "harness:agent"
gh label edit "pipeline:human"         --name "harness:human"
gh label edit "pipeline:ready"         --name "harness:ready"
gh label edit "pipeline:spec-written"  --name "harness:spec-written"
gh label edit "pipeline:waiting-merge" --name "harness:waiting-merge"
```

`pipeline:refinement` n'est pas dans la liste : la boucle ne la lit pas, et son
préflight refuserait de tourner s'il l'exigeait. À renommer avec le raffinage,
quand il arrivera.

Pour vérifier après coup :

```bash
gh label list --limit 100 | grep -E 'harness:|pipeline:'
```

## Mettre `claude` à jour

Le préflight exige **`claude` >= 2.1.277**, et refusera de tourner en dessous.
La raison est dans `docs/SESSION-CARRIER.md` : `total_cost_usd` sur un appel
`--resume` est cumulatif pour toute la conversation depuis cette version, et ne
couvrait que l'appel avant. Le registre ne peut pas être juste sans savoir de
quel côté on est, et une porte coûte un appel local là où un `costs.tsv` faux
ne se voit pas.

```bash
claude --version   # 2.1.257 au moment où ceci est écrit — en dessous de la porte
claude update
```

## Ce qui reste partagé, et ce qui ne l'est pas

| Chemin | Qui l'écrit après la bascule |
| --- | --- |
| `.llocal/agent-loop/costs.tsv` | le Rust, en ajoutant à l'historique — l'en-tête est gelé, les anciennes lignes restent lisibles |
| `.llocal/agent-loop/state` | le Rust, même format deux lignes (`task=`, `flow_id=`) |
| `.llocal/agent-loop/flow_states.db` | plus personne. Le Rust écrit `flow-<id>.jsonl` à côté |

Le sqlite n'est pas supprimé par la bascule : il reste lisible au `sqlite3` si
une autopsie en a besoin un jour.
