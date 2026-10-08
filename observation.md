# Observation — tour du harness (2026-10-08)

> **Suivi (même jour)** — branche `chore/observation-actions`, PR #1 ouverte, CI verte (4 jobs), non mergée.
> Découverte en route : la CI lint sur `stable` = clippy 1.99, la machine est en 1.98 ; 78 `assert!(x.is_empty())` préexistants rendaient `main` rouge → corrigés dans la PR (4e commit).
> ✅ fait · ⏸ reporté (chantier à part) · 👤 geste humain.
>
> - ✅ Port `Disk` dans `explore.rs` / `publish.rs`, grep de revue étendu (§1, §6.1)
> - ✅ Règle « un worktree par session » dans `CLAUDE.md` (§1, §6.2) — l'isolation reste une discipline, pas un verrou
> - ✅ `.env.example` recommande `AGENTIC_WORKSPACES_DIR=../harness-workspaces` ; le doctor suit ce réglage (§1, §6.3) — 👤 reporter la ligne dans `.env.local` (reclone des lanes au prochain démarrage)
> - ✅ Règle `tracing` réécrite, français résiduel traduit, `README`, `.env.example` (§2, §6.6)
> - ✅ CI : jobs `msrv` et `wasm` ; hook `pre-push` clippy (§3, §6.5)
> - ✅ `harness doctor` nomme les dossiers étrangers de `.llocal` avec leur poids (§3, §6.8)
> - ✅ Carte `explore` sur la branche du jalon (§5)
> - ✅ Sections inconnues conservées au round suivant (§5)
> - ⏸ Test d'intégration « un tick de `watch` » (§4, §6.4) : demande d'exposer les fakes de `workflows` hors `cfg(test)` (feature `fakes`) — à décider
> - ⏸ Découpe de `scene.rs` / `github.rs` (§4, §6.7), docs bilingues (§2), `bypassPermissions` par défaut (§1), commit déterministe, serveur à hooks, réarmement du breaker (§5)


Tour en lecture seule : overview, les 5 crates, CI, hooks, `.llocal`, `todo.md`,
`observations.md`. Commandes passées : `cargo fmt --check` ✅, `cargo clippy -D warnings` ✅,
`cargo test --workspace` : 1019 tests, 2 échecs **transitoires** (voir §1).

Chiffres : 54 commits, ~53 k lignes Rust, 1019 tests, 1 fichier d'intégration.

---

## 1. Risques immédiats

- **Plusieurs sessions sur le même checkout.** Pendant le tour, une autre session
  modifiait `init_repo/action/install.rs` (ajout de `ci.yml` aux assets). Mon
  `cargo test` a vu un état à moitié édité → 2 rouges (`16` vs `17 fichiers`),
  verts au rerun. Conséquence : un `cargo test` vert n'est fiable que si une seule
  session écrit. Pistes : worktree par session (`git worktree`), ou verrou simple.
- **Violations hexagonales en code livré** (règle 3 de `CLAUDE.md`) :
  - `crates/workflows/src/common/explore.rs:332-392, 458` — `std::fs` dans une
    `Verification` (`Keep`) et dans `mapped_at` / `repo_context`.
  - `crates/workflows/src/refinement/action/publish.rs:132-137` — `std::fs::write`
    du corps de l'issue dans une action.
  - Le grep de revue (`adapters::`) ne les attrape pas : il ne cherche pas `std::fs`.
    Ajouter `grep -rn 'std::fs\|std::process\|tokio::fs' crates/workflows/src crates/core/src/{domain,execution,ports}`
    au contrôle. Port manquant : un `Artifacts` (lire/écrire un fichier de trace par cible).
- **`PERMISSION_MODE=bypassPermissions` par défaut** dans `.env.example`. Les
  sessions cibles tournent sans garde. Acceptable en labo, à documenter comme
  choix explicite (et à proposer `acceptEdits` en défaut « prudent »).
- **Workspaces montés sous le dépôt du harness** (`.llocal/agentic_workspaces`) :
  chaque session cible hérite du `CLAUDE.md` Rust. Déjà noté dans `observations.md`
  (l. 69), toujours vrai. `AGENTIC_WORKSPACES_DIR` existe : faire du hors-dépôt le
  défaut, ou poser un `CLAUDE.md` vide dans `.llocal/`.

## 2. Règles `CLAUDE.md` ↔ code : écarts

- **`tracing` jamais utilisé** dans `core` ni `workflows` (0 occurrence, pas même
  déclaré dans leurs `Cargo.toml`). Le journal passe par `Logbook`/`Sink`. Soit la
  règle « use tracing in library code » est fausse, soit le code l'est. Proposition :
  réécrire la règle (le `Sink` est le port de journal, décision assumée).
- **Français résiduel en code** : ~70 lignes accentuées dans `workflows` et `core`,
  surtout des fixtures de test, mais aussi :
  - `pr_review/data/state.rs:23` — `expect("precheck doit poser la PR…")` en livré.
  - `refinement/action/publish.rs:133` — message d'erreur « impossible de créer ».
  - `Cargo.toml` racine : 3 commentaires en français.
- **Docs bilingues** : `docs/MIGRATION.md` (78 lignes accentuées), `ROUND-DRAFT.md`
  en français ; `ARCHITECTURE*.md` en anglais. Choisir une langue par dossier.
- **`.env.example` l. 6** parle encore de `pipelinev2/`.
- **`README.md` vide.** Un nouveau venu n'a que `CLAUDE.md` (destiné aux sessions).
- **proptest** : règle « pour tout parseur/validateur », réalité 2 fichiers
  (`remote.rs` + 1). Candidats : `sections::parse`, `common::routing`, `json_reply`,
  `signatures.rs` (889 l.), `stream_log.rs`.
- **`#[allow(clippy::too_many_arguments)]`** ×8 dans `view-render/scene.rs` avec
  la même justification copiée-collée : un `struct Rect`/`Paint` les ferait tomber.

## 3. CI / outillage

- CI tourne sur `stable` ; `rust-version = "1.98"` n'est jamais vérifié. Ajouter un
  job `dtolnay/rust-toolchain@1.98` + `cargo check` (ou retirer le MSRV).
- `cargo-audit` absent de la machine locale ; seul CI le fait. OK, mais
  `cargo deny` (licences + bans) couvrirait plus pour le même prix.
- Le hook `pre-commit` ne fait que `rustfmt`. `cargo clippy` avant commit n'est
  qu'une consigne de `CLAUDE.md` : la faire tenir par le hook (ou `pre-push`).
- Aucun test ne compile `view-render` en `wasm32` en CI ; le bundle n'est pas
  vérifié par la CI (`scripts/build-render.sh` seulement en local).
- `.llocal/` dérive : `archive/` (191 Mo), `postgres/`, `init/`, `lanes/`,
  `planner-locks/`, `split-locks/`. `observations.md` annonçait « `logs/` et
  `agentic_workspaces/` seulement ». Documenter ou ranger ; `harness doctor`
  pourrait signaler les dossiers inconnus.

## 4. Lisibilité / taille

| fichier | lignes | remarque |
| --- | --- | --- |
| `view-render/src/scene.rs` | 2832 | 3 niveaux dans un fichier, `too_many_lines` ×3 |
| `core/src/execution/provisioning.rs` | 1361 | mount/unmount + stratégies + tests |
| `view-render/src/app.rs` | 1313 | |
| `core/src/adapters/shell/github.rs` | 1251 | un adapter `gh` pour tout GitHub |
| `workflows/src/dev_loop/action/actions.rs` | 1197 | |

- `github.rs` : un `GitHub` port unique → l'adapter porte issues, PR, labels,
  checks, sub-issues. Un split `Issues` / `Pulls` / `Labels` rendrait les fakes
  plus petits et la famille « consulting » n'hériterait que du nécessaire.
- `dev_loop::Ports` n'a que 4 `Rc<dyn>` ; le reste passe par `Config` (9 `RefCell`
  dans `workflows`). À surveiller : décision #6 « `Settings` immuable ».
- Tests : 1019 unitaires, **1 seul fichier d'intégration** (`depends_on_core.rs`).
  Aucun test ne joue un `watch` complet avec fakes (routeur → dispatch → workflow).
  C'est là que les bugs réels sont tombés (`observations.md` : `-R slug`, glob
  `milestone/**`, STOP repayé 5 fois).

## 5. Fonctionnel — points ouverts hérités

Repris de `todo.md` / `observations.md`, toujours non tranchés dans le code :

- **Carte `explore` dessinée sur la branche par défaut**, pas sur la branche de
  milestone : la dernière tâche d'un jalon ne voit pas ses sœurs (`observations.md` l. 68).
- **Commit déterministe des tâches** (`todo.md` §1) : le stage `code` fait encore
  branche→PR→merge lui-même ; `dev_loop::Ports` n'a pas de port `git`.
- **Serveur à hooks** (`todo.md` §4) : décidé, pas construit.
- **`sections::KEYS` = 7 titres** : une section inconnue dans un corps d'issue est
  toujours perdue au round suivant (`observations.md` l. 86). Vérifier que
  `## Assumptions (autonomous run)` et `## Owner Decisions` sont dans les 7, sinon
  conserver les sections inconnues telles quelles plutôt que les jeter.
- **Breaker** (`domain/breaker.rs`) : 3 échecs identiques → refus. Pas de
  commande humaine pour le lire/réarmer autrement qu'en éditant l'issue.
- **Parallélisme** : `--parallel` lance jusqu'à 10 lanes (processus). `CLAUDE.md`
  dit encore « drives one session at a time ». Vrai par processus, faux par
  machine ; préciser, et documenter le partage de quota entre lanes.
- **Steward** dans `view` : un Claude Code interactif, donc un chemin d'écriture
  vers le dépôt depuis la page « read-only ». Assumé dans `view/ARCHITECTURE.md`,
  à répercuter dans `CLAUDE.md` (« `view` never writes anything a run reads »).

## 6. Améliorations proposées, par ordre

1. Port `Artifacts` + purge de `std::fs` dans `explore.rs` et `publish.rs` ;
   étendre le grep de revue.
2. Isolation des sessions parallèles (worktree ou verrou) sur ce dépôt.
3. `AGENTIC_WORKSPACES_DIR` hors dépôt par défaut.
4. Un test d'intégration « un tick de `watch` » avec tous les fakes.
5. Job MSRV en CI, clippy dans le hook, build wasm en CI.
6. Hygiène : `README.md`, `.env.example`, français résiduel, `tracing` dans `CLAUDE.md`.
7. Découper `scene.rs` par niveau et `github.rs` par sous-port.
8. `harness doctor` : lister les dossiers `.llocal` inconnus et la taille de `archive/`.
