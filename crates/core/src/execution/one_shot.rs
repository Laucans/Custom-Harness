//! Une séquence d'étapes, une fois : la forme d'un workflow qui a une cible
//! et s'arrête — une revue de PR, un round de raffinage.
//!
//! **Deux exemples réels avant d'écrire ceci**, suivant la règle que
//! `docs/ROUND-DRAFT.md` s'est donnée pour le round de la boucle : la revue
//! de PR et le raffinage d'une issue suivent exactement la même forme —
//! vérifier qu'il y a quelque chose à faire, tenir un verrou pour qu'au plus
//! un porteur de la même cible tourne à la fois, enchaîner une séquence
//! jusqu'au premier échec non toléré, laisser une ligne de résumé. La boucle
//! de dev n'en a pas besoin : elle répète un round reprenable sur plusieurs
//! tours, ce qui est une forme différente — `workflow::DevLoop` l'écrit à la
//! main plutôt que de forcer les deux dans un seul moule.
//!
//! Un trait, pas une liste de fermetures : chaque méthode a un sens propre
//! (« y a-t-il quelque chose à faire », « ce verrou est-il déjà tenu »), et
//! Rust type chacune séparément là où Python les passait comme un sac de
//! callables au même constructeur.

use std::path::Path;

use async_trait::async_trait;

use crate::adapters::store::lock::Locks;
use crate::domain::{Halt, Outcome, Verdict};
use crate::execution::context::Context;
use crate::execution::gate::Gate;
use crate::execution::stage::Stage;
use crate::execution::traits::Guarded;

/// La politique d'un workflow qui tourne une fois.
#[async_trait(?Send)]
pub trait OneShot<S> {
    /// Ce qui doit tenir avant que le premier stage soit payé — l'outillage,
    /// et ce que ce workflow-ci exige en plus. `None` : rien à vérifier,
    /// laissé au lanceur.
    fn pre(&self) -> Option<&Gate<S>> {
        None
    }

    /// Y a-t-il quelque chose à faire ? `Ok(Some(message))` arrête le run
    /// proprement, avant même d'essayer le verrou — une PR en brouillon ou
    /// déjà revue, une issue fermée, n'ont rien à obtenir, et ce n'est pas un
    /// échec.
    ///
    /// # Errors
    /// Une lecture qui n'a pas abouti.
    async fn precheck(&self, ctx: &mut Context<S>) -> Outcome<Option<String>>;

    /// Le dossier et le nom du verrou : au plus un porteur de cette cible à
    /// la fois.
    fn lock(&self) -> (&Path, &str);

    /// Ce qui tient les verrous.
    fn locks(&self) -> &dyn Locks;

    /// Le message quand le verrou est déjà tenu par quelqu'un d'autre.
    fn held(&self, ctx: &Context<S>) -> String;

    /// La séquence à faire tourner une fois le verrou obtenu, dans l'ordre.
    fn stages(&self) -> &[Stage<S>];

    /// Un stage qui échoue peut-il être toléré ? `None` — le défaut — arrête
    /// la séquence, comme n'importe quel [`Round`](crate::execution::Round).
    /// La revue de PR tolère un échec de sa passe ligne-à-ligne (sauf quota) :
    /// la passe de synthèse continue sans ses trouvailles plutôt que de
    /// perdre les deux passes payées.
    fn tolerate(&self, _stage: &str, _failed: &Halt) -> Option<String> {
        None
    }

    /// La ligne qu'un run réussi laisse derrière lui.
    fn summary(&self, ctx: &Context<S>) -> String;

    /// Précontrôle, verrou, séquence, résumé — dans cet ordre, et c'est tout
    /// ce que la revue de PR et le raffinage ont en commun.
    ///
    /// Une méthode du trait, pas un `impl Executable` générique : `Round<S>`
    /// et `Stage<S>` implémentent déjà `Executable<S>` directement, et un
    /// `impl<S, O: OneShot<S>> Executable<S> for O` entrerait en conflit avec
    /// eux aux yeux du compilateur — qui ne peut pas prouver qu'un type ne
    /// portera jamais les deux. Chaque workflow écrit donc `Executable` à la
    /// main, en deux lignes qui délèguent ici — le même geste que
    /// `TaskRound`.
    async fn execute(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        if let Some(message) = self.precheck(ctx).await? {
            ctx.traces.say(&message);
            return Ok(Verdict::Continue);
        }

        let (dir, name) = self.lock();
        if !self.locks().acquire(dir, name)? {
            ctx.traces.say(&self.held(ctx));
            return Ok(Verdict::Continue);
        }

        let ran = self.run_stages(ctx).await;
        self.locks().release(dir, name);
        ran?;

        ctx.traces.say(&self.summary(ctx));
        Ok(Verdict::Continue)
    }

    /// La séquence, stage par stage, un échec non toléré arrête tout.
    async fn run_stages(&self, ctx: &mut Context<S>) -> Outcome<()> {
        for stage in self.stages() {
            if let Err(halt) = stage.execute(ctx).await {
                match self.tolerate(&stage.name, &halt) {
                    Some(message) => ctx.traces.say(&message),
                    None => return Err(halt),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::store::lock::DirLocks;
    use crate::domain::Verdict as DomainVerdict;
    use crate::execution::action::Action;
    use crate::execution::context::Settings;
    use crate::execution::stage::StageBody;
    use crate::execution::traits::Verification;
    use crate::traces::Logbook;
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("harness-one-shot-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Push(&'static str);

    #[async_trait(?Send)]
    impl Action<RefCell<Vec<String>>> for Push {
        async fn run(&self, ctx: &mut Context<RefCell<Vec<String>>>) -> Outcome<DomainVerdict> {
            ctx.state.borrow_mut().push(self.0.to_string());
            Ok(DomainVerdict::Continue)
        }
    }

    fn local_stage(tag: &'static str) -> Stage<RefCell<Vec<String>>> {
        Stage {
            name: tag.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Push(tag))],
            },
        }
    }

    struct AlwaysFails;

    #[async_trait(?Send)]
    impl Verification<RefCell<Vec<String>>> for AlwaysFails {
        async fn verify(&self, _ctx: &Context<RefCell<Vec<String>>>) -> Outcome<DomainVerdict> {
            Err(Halt::Failed("cassé".to_string()))
        }
    }

    /// Un `OneShot` de test : verrou, séquence, pas de précontrôle qui arrête.
    struct Fixture {
        dir: PathBuf,
        name: String,
        stages: Vec<Stage<RefCell<Vec<String>>>>,
        tolerate_all: bool,
    }

    #[async_trait(?Send)]
    impl OneShot<RefCell<Vec<String>>> for Fixture {
        async fn precheck(
            &self,
            _ctx: &mut Context<RefCell<Vec<String>>>,
        ) -> Outcome<Option<String>> {
            Ok(None)
        }

        fn lock(&self) -> (&Path, &str) {
            (&self.dir, &self.name)
        }

        fn locks(&self) -> &dyn Locks {
            &DirLocks
        }

        fn held(&self, _ctx: &Context<RefCell<Vec<String>>>) -> String {
            format!("skip — {} already running", self.name)
        }

        fn stages(&self) -> &[Stage<RefCell<Vec<String>>>] {
            &self.stages
        }

        fn tolerate(&self, _stage: &str, failed: &Halt) -> Option<String> {
            self.tolerate_all
                .then(|| format!("tolerated: {}", failed.reason()))
        }

        fn summary(&self, _ctx: &Context<RefCell<Vec<String>>>) -> String {
            "done".to_string()
        }
    }

    fn ctx() -> Context<RefCell<Vec<String>>> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            RefCell::new(Vec::new()),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_stages_run_in_order_and_the_lock_is_released_after() {
        let dir = Dir::new("ordered");
        let fixture = Fixture {
            dir: dir.0.clone(),
            name: "32".to_string(),
            stages: vec![local_stage("a"), local_stage("b")],
            tolerate_all: false,
        };
        let mut context = ctx();
        fixture.execute(&mut context).await.expect("un succès");
        assert_eq!(
            *context.state.borrow(),
            vec!["a".to_string(), "b".to_string()]
        );
        // Relâché : un second appel doit pouvoir reprendre le verrou.
        assert!(DirLocks.acquire(&dir.0, "32").expect("réacquis"));
    }

    #[tokio::test]
    async fn an_untolerated_failure_stops_the_sequence_and_still_releases_the_lock() {
        let dir = Dir::new("stops");
        let fixture = Fixture {
            dir: dir.0.clone(),
            name: "32".to_string(),
            stages: vec![
                local_stage("a"),
                Stage {
                    name: "fails".to_string(),
                    pre: Some(Gate {
                        name: "toujours",
                        checks: vec![Box::new(AlwaysFails)],
                    }),
                    post: None,
                    body: StageBody::Local { actions: vec![] },
                },
                local_stage("never"),
            ],
            tolerate_all: false,
        };
        let mut context = ctx();
        let err = fixture
            .execute(&mut context)
            .await
            .expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Failed(_)));
        assert_eq!(*context.state.borrow(), vec!["a".to_string()]);
        assert!(
            DirLocks
                .acquire(&dir.0, "32")
                .expect("le verrou reste relâché")
        );
    }

    #[tokio::test]
    async fn a_tolerated_failure_lets_the_sequence_continue() {
        let dir = Dir::new("tolerated");
        let fixture = Fixture {
            dir: dir.0.clone(),
            name: "32".to_string(),
            stages: vec![
                Stage {
                    name: "fails".to_string(),
                    pre: Some(Gate {
                        name: "toujours",
                        checks: vec![Box::new(AlwaysFails)],
                    }),
                    post: None,
                    body: StageBody::Local { actions: vec![] },
                },
                local_stage("after"),
            ],
            tolerate_all: true,
        };
        let mut context = ctx();
        fixture.execute(&mut context).await.expect("toléré");
        assert_eq!(*context.state.borrow(), vec!["after".to_string()]);
    }

    #[tokio::test]
    async fn a_lock_already_held_skips_the_sequence_without_erroring() {
        let dir = Dir::new("held");
        assert!(DirLocks.acquire(&dir.0, "32").expect("tenu par un autre"));
        let fixture = Fixture {
            dir: dir.0.clone(),
            name: "32".to_string(),
            stages: vec![local_stage("never")],
            tolerate_all: false,
        };
        let mut context = ctx();
        fixture
            .execute(&mut context)
            .await
            .expect("un succès, pas une erreur");
        assert!(context.state.borrow().is_empty(), "rien n'a dû tourner");
    }

    #[tokio::test]
    async fn a_precheck_that_has_nothing_to_do_never_touches_the_lock() {
        struct NothingToDo {
            dir: PathBuf,
        }

        #[async_trait(?Send)]
        impl OneShot<RefCell<Vec<String>>> for NothingToDo {
            async fn precheck(
                &self,
                _ctx: &mut Context<RefCell<Vec<String>>>,
            ) -> Outcome<Option<String>> {
                Ok(Some("skip — draft PR".to_string()))
            }

            fn lock(&self) -> (&Path, &str) {
                (&self.dir, "32")
            }

            fn locks(&self) -> &dyn Locks {
                &DirLocks
            }

            fn held(&self, _ctx: &Context<RefCell<Vec<String>>>) -> String {
                unreachable!("ne doit jamais être atteint")
            }

            fn stages(&self) -> &[Stage<RefCell<Vec<String>>>] {
                &[]
            }

            fn summary(&self, _ctx: &Context<RefCell<Vec<String>>>) -> String {
                unreachable!("le précontrôle a déjà tout dit")
            }
        }

        let dir = Dir::new("precheck-skips-lock");
        let fixture = NothingToDo { dir: dir.0.clone() };
        let mut context = ctx();
        fixture.execute(&mut context).await.expect("un succès");
        // Le verrou n'a jamais été touché : il reste libre.
        assert!(DirLocks.acquire(&dir.0, "32").expect("toujours libre"));
    }
}
