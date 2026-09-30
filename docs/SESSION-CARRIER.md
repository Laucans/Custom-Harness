# Ce qui porte une `Session` — rapport décisionnel

Étape 5. Trois propositions, à arbitrer. Le port est déjà écrit
(`adapters::agent::Session` / `SessionFactory`), donc ce choix ne touche ni
`Stage`/`Round`, ni les workflows : c'est une décision **réversible**, et ça
compte dans l'arbitrage.

## Correction : ce que j'ai écrit à l'étape 4 était faux

`MIGRATION.md` disait, sous tmux : « **le coût et l'usage ne sont pas
récupérables** ». C'est faux. L'erreur était de supposer que le pane est le
seul canal. Il ne l'est pas — et la question « peut-on demander le prix à la
session ? » a une meilleure réponse : **on ne le demande pas au pane du tout.**

Quatre canaux structurés existent *à côté* du terminal, tous compatibles avec
une session interactive hébergée dans tmux.

### 1. `statusLine` — le plus riche

Claude Code invoque un script de status line en lui passant du JSON sur stdin.
Le schéma porte, entre autres :

```json
{
  "session_id": "...", "transcript_path": "...", "model": { "id": "..." },
  "cost": {
    "total_cost_usd": 0.01234,
    "total_duration_ms": 45000, "total_api_duration_ms": 2300,
    "total_lines_added": 156, "total_lines_removed": 23
  },
  "context_window": {
    "current_usage": { "input_tokens": 8500, "output_tokens": 1200,
      "cache_creation_input_tokens": 5000, "cache_read_input_tokens": 2000 }
  },
  "prompt_cache": { "hit_ratio": 0.91, "cache_write_tokens": 352000, "…": "…" },
  "effort": { "level": "high" },
  "rate_limits": {
    "five_hour":   { "used_percentage": 23.5, "resets_at": 1738425600 },
    "seven_day":   { "used_percentage": 41.2, "resets_at": 1738857600 },
    "spend_limit": { "used_percentage": 62.8, "resets_at": 1740787200 }
  }
}
```

- **Le coût en USD exact**, pas des jetons à convertir.
- **Les ratios de cache**, que `runtime/monitoring/metrics.py` calcule
  aujourd'hui à la main.
- **Les quotas, structurés et *prédictifs*.** Aujourd'hui `Halt::Quota` est
  détecté **après coup**, en cherchant une phrase dans un run déjà échoué.
  Ici le harness peut savoir *avant* d'ouvrir une stage que la fenêtre de 5 h
  est à 95 % — et s'arrêter proprement au lieu de brûler un stage. C'est un
  gain que ni stream-json ni le transcript n'offrent.
- Mises à jour événementielles, débouncées à 300 ms, plus un
  `refreshInterval` optionnel (minimum 1 s).
- **Piège** : un script en vol est **annulé** si une nouvelle mise à jour
  arrive. Donc écriture atomique (fichier temporaire + `rename`), sinon
  enregistrement déchiré.
- C'est un élément de TUI : disponible précisément dans le cas interactif
  (donc tmux), pas en mode `-p`.

### 2. OpenTelemetry — le plus propre à agréger

| Métrique | Unité | Quand |
| --- | --- | --- |
| `claude_code.cost.usage` | **USD** | après chaque requête API |
| `claude_code.token.usage` | tokens | après chaque requête API |

Attributs : `session.id`, `model`, `effort`, `query_source`
(`main` / `subagent` / `auxiliary`), `speed`. Donc attribuable par session, et
distinguant un subagent du thread principal.

- Exportable **localement, sans réseau** : `OTEL_METRICS_EXPORTER=prometheus`
  expose `http://localhost:9464/metrics`, que le harness scrape.
- **À ne pas utiliser** : `OTEL_METRICS_EXPORTER=console` — ça écrit sur
  stdout, donc dans le pane, par-dessus la TUI.
- Compteur cumulatif : le coût d'un tour est un delta entre deux scrapes. Si
  une session = une stage, une seule lecture à la fin suffit.
- Un port par processus : sans objet ici, le harness est séquentiel
  (décision n°3).

### 3. Le transcript JSONL — vérifié sur votre machine

`~/.claude/projects/<cwd-slug>/<session-id>.jsonl`, et `--session-id <uuid>`
permet au harness de **choisir** l'uuid, donc de connaître le chemin d'avance.

Mesuré sur le transcript de la session en cours (332 messages `assistant`) :

- `message.usage` présent sur **100 %** d'entre eux : `input_tokens`,
  `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`,
  `service_tier`, `speed`, `iterations`.
- `stopReason` au niveau racine — une frontière de tour exploitable.
- **Aucun champ coût** : `grep` sur tout nom contenant `cost`/`usd` → rien.
  Les jetons oui, les dollars non. Il faudrait une table de prix à tenir à
  jour, ce qui est exactement le genre de duplication qui dérive en silence.

### 4. Le hook `Stop` — la frontière de tour

Payload : `session_id`, `transcript_path`, `cwd`, `permission_mode`,
`hook_event_name`. Déclenché quand l'agent principal a fini de répondre —
donc **la fin d'un tour, en événement**, sans lire un seul octet de terminal.

- Le projet sait déjà faire : `event_assistant` fait déjà tourner un
  `PostToolUse` sur `gh pr create` et un `PreToolUse` de garde de branche.
- **Réserves connues** : des bugs rapportés de `transcript_path` /
  `session_id` **périmés** après `/exit` et `--continue`
  ([#8564](https://github.com/anthropics/claude-code/issues/8564),
  [#9188](https://github.com/anthropics/claude-code/issues/9188)). À tester
  avant d'en dépendre.
- Le payload ne porte pas l'usage ; c'est une demande ouverte
  ([#91767](https://github.com/anthropics/claude-code/issues/91767)).

## Le vrai coût de tmux, qui n'est pas celui que je croyais

Le coût n'est pas « pas de données de coût ». C'est celui-ci :

> **tmux n'achète pas l'agnosticisme d'agent. Il achète un conteneur
> générique plus un port d'instrumentation à écrire par agent.**

`statusLine`, les noms de métriques OTel, les hooks, le schéma du transcript :
**tout est spécifique à Claude Code.** Un pane tmux qui héberge `opencode`
n'aura aucun de ces quatre canaux — il en aura d'autres, ou aucun. Donc la
motivation d'origine (« un Tmux portera un claude ou un opencode ») tient pour
le *conteneur*, pas pour l'instrumentation. C'est défendable — c'est même la
bonne architecture — mais c'est deux ports, pas un.

Ce qui reste difficile sous tmux, et qui ne se résout par aucun canal
out-of-band :

- **Piloter l'entrée reste des frappes clavier.** `send-keys` dans une TUI est
  plus fragile qu'écrire une ligne JSON : bracketed paste, prompts multilignes,
  un `/` en tête interprété, un pane dans un état modal. `send-keys -l` et
  `load-buffer` + `paste-buffer` atténuent, mais on simule un humain.
- **Les demandes de permission bloquent.** En non surveillé, un dialogue de
  permission est un blocage indéfini. `--permission-mode bypassPermissions`
  règle la question — et le projet l'a déjà choisi (`PERMISSION_MODE`).

## Les trois propositions

### A — tmux + instrumentation Claude Code

Un pane par stage, `claude` interactif, `--session-id <uuid>` imposé par le
harness, `--permission-mode bypassPermissions`. Entrée par `send-keys`. Fin de
tour par le hook `Stop`. Coût et quotas par `statusLine`, écrit atomiquement
dans un fichier par session.

| | |
| --- | --- |
| **Gagne** | attachable en vol (`tmux attach`) : un humain reprend une stage coincée à 3 h du matin. Coût USD exact, ratios de cache, **quotas prédictifs**. Conteneur générique pour un futur agent. |
| **Coûte** | trois surfaces spécifiques à Claude Code à écrire et à maintenir (statusline, hook, frappes). Fragilité TUI en entrée. Réserves de péremption sur le hook. |
| **Effort** | le plus élevé |

### B — stream-json, processus long

`claude -p --input-format stream-json --output-format stream-json
--replay-user-messages`. Un seul processus enfant, JSON par lignes dans les
deux sens. Fin de tour et `total_cost_usd` dans le message `result`.

| | |
| --- | --- |
| **Gagne** | un seul canal, typé, en bande. Aucune instrumentation annexe. Acquittement natif (`--replay-user-messages`). La vraie « session ouverte ». Marche en CI, sans TTY. |
| **Coûte** | non attachable : personne ne peut regarder ni reprendre. Un protocole bidirectionnel à écrire — le plus gros adaptateur de la migration. Pas de quotas prédictifs. |
| **Effort** | élevé, concentré en un endroit |

### C — un processus par action, recousu par `--resume`

Chaque `SessionAction` = un `claude -p --resume <uuid> --output-format json`.
Un objet JSON à parser, `total_cost_usd` dedans, frontière de tour = sortie du
processus.

| | |
| --- | --- |
| **Gagne** | de loin le moins de code : ni protocole, ni hook, ni statusline, ni frappes. Coût USD et fin de tour **gratuits** à chaque appel. La continuité de conversation — ce pour quoi `lead` existait — est bien préservée par `--resume`. |
| **Coûte** | « session ouverte » est une fiction : rechargement du contexte par action (le cache de prompt en amortit le coût, pas la latence). Non attachable. Pas de quotas prédictifs. |
| **Effort** | le plus faible |

## Recommandation

**C d'abord, A comme destination si l'attachabilité s'avère compter.**

Trois raisons :

1. **Le projet optimise pour la simplicité et l'itération rapide**, pas pour
   l'exhaustivité (`CLAUDE.md` du dépôt parent, en toutes lettres). C est
   petit, et il donne *déjà* coût exact et frontières de tour.
2. **La décision est réversible par construction.** Le trait `Session` existe,
   `Stage`/`Round` sont testés contre des fakes. Passer de C à A ne touche
   aucun workflow. Prendre le chemin cher d'abord, c'est payer une option dont
   on ne sait pas encore si on a besoin.
3. **B est le mauvais achat.** Il coûte le plus (un protocole bidirectionnel)
   pour un bénéfice — un processus réellement vivant — que C approxime à une
   fraction de l'effort, et sans rien gagner sur l'observabilité.

Ce qui ferait basculer vers **A** : si en usage réel les stages se coincent et
qu'on veut pouvoir reprendre la main sans tuer le run, ou si les quotas
prédictifs deviennent nécessaires pour ne pas gâcher des rounds. Les deux sont
plausibles — d'où « destination », pas « jamais ».

## Sources

- [Monitoring — Claude Code Docs](https://code.claude.com/docs/en/monitoring-usage) — `claude_code.cost.usage` (USD), `claude_code.token.usage`, exporteurs locaux
- [Customize your status line — Claude Code Docs](https://code.claude.com/docs/en/statusline) — schéma JSON stdin, cadence, annulation en vol
- [Hooks reference — Claude Code Docs](https://code.claude.com/docs/en/hooks) — événements et champs communs
- [Manage costs effectively — Claude Code Docs](https://code.claude.com/docs/en/costs)
- [Track cost and usage — Agent SDK](https://code.claude.com/docs/en/agent-sdk/cost-tracking)
- Issues : [#8564](https://github.com/anthropics/claude-code/issues/8564), [#9188](https://github.com/anthropics/claude-code/issues/9188) (péremption hook), [#91767](https://github.com/anthropics/claude-code/issues/91767) (usage dans les hooks)
- Mesure locale : `claude` 2.1.257, `tmux` 3.6b, transcript de session (332 messages `assistant`)
