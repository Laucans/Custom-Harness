# La reprise en main — ce qu'un humain doit faire, et quand

> **Où en est le harness.** Il tourne. `harness --dry-run --no-workspace` passe
> la porte de version, interroge `gh`, vérifie la branche et la CI, puis
> **s'arrête sur les étiquettes** — parce que les sept `harness:*` n'existent
> pas encore. Les deux gestes ci-dessous sont tout ce qui manque.

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

### Attention : `claude update` peut être masqué par un shim

Sur cette machine, `claude update` a bien installé 2.1.285 dans
`~/.local/share/claude/versions/2.1.285`, et ce binaire rapporte bien sa
version. Mais `~/.local/bin/claude` est un shim écrit à la main qui résout
**le binaire embarqué dans l'extension VS Code** :

```sh
bin=$(ls -d "$HOME"/.vscode/extensions/anthropic.claude-code-*/resources/native-binary/claude \
      | sort -V | tail -1)
exec "$bin" "$@"
```

L'extension étant en 2.1.257, `claude --version` rapporte 2.1.257 et la porte
refusera de tourner — alors que la mise à jour a réussi. Deux sorties, au
choix :

1. **mettre à jour l'extension VS Code** : le shim la suit déjà, donc rien
   d'autre à changer. C'est l'intention d'origine du shim ;
2. **faire pointer le shim sur l'installation native**
   (`~/.local/share/claude/versions/`, plus récente), ou sur le plus récent
   des deux.

Vérifier avec le binaire que le `PATH` résout vraiment, pas avec celui qu'on
croit :

```bash
command -v claude && claude --version
```

**La porte le dit elle-même**, et c'est voulu : son message nomme la commande
*et* le piège du shim, parce que « `claude update` a marché mais la porte
refuse toujours » est précisément la situation où il faut le lire.

```
STOP: claude 2.1.257 is older than 2.1.277 — … Run: claude update (and check
that `command -v claude` resolves to what you just updated)
```

En attendant, un run se vérifie en mettant l'installation native en tête du
`PATH` pour cette commande-là seulement :

```bash
d=$(mktemp -d) && ln -s "$HOME/.local/share/claude/versions/2.1.285" "$d/claude"
PATH="$d:$PATH" harness --dry-run --no-workspace
```

## Ce qui reste partagé, et ce qui ne l'est pas

| Chemin | Qui l'écrit après la bascule |
| --- | --- |
| `.llocal/agent-loop/costs.tsv` | le Rust, en ajoutant à l'historique — l'en-tête est gelé, les anciennes lignes restent lisibles |
| `.llocal/agent-loop/state` | le Rust, même format deux lignes (`task=`, `flow_id=`) |
| `.llocal/agent-loop/flow_states.db` | plus personne. Le Rust écrit `flow-<id>.jsonl` à côté |

Le sqlite n'est pas supprimé par la bascule : il reste lisible au `sqlite3` si
une autopsie en a besoin un jour.

## Le premier vrai run, quand les deux gestes sont faits

Dans cet ordre, et chacun répond à une question que le suivant suppose.

```bash
# 1. Qu'est-ce qu'il choisirait ? Ne dépense rien, n'ouvre aucune session.
harness --dry-run --no-workspace --allow-dirty

# 2. Un seul tour, dans un clone, un seul stage : la plus petite dépense
#    réelle qui prouve que la chaîne entière tient.
harness --rounds 1 --stages business-analyst

# 3. Un round entier.
harness --rounds 1
```

Deux choses à savoir avant le premier run qui monte un workspace :

- **le clone n'a ni `node_modules` ni venv** — rien de ce que git ne suit pas.
  Une porte de préflight le dit et refuse de tourner, plutôt que de brûler un
  `/code` dont chaque commande de vérification répond `command not found`. Il
  faut les installer **dans le workspace**, une fois ; `PERMANENT` les garde
  ensuite d'un run à l'autre ;
- **ce qui n'est pas poussé n'existe pas pour le run.** Le montage clone
  `origin`. Un avertissement le dit si le checkout porte des commits non
  poussés ou un arbre sale, mais c'est un avertissement : il ne refuse pas.

`--rollover` n'est **pas** le défaut : un milestone fini arrête la boucle au
lieu de payer un `/planner` qui engage le projet sur un item de roadmap que
personne n'a lu. Le brancher est une décision, pas un réglage.
