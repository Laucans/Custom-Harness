//! A factory taken down, in two degrees.
//!
//! **Delink**: the harness lets go of the factory — its watch is asked to
//! stop (soft: the running tasks finish, none starts), its folder is removed,
//! it leaves the register — and its GitHub repository stays as it is. The
//! repository can be connected again, later, as any other.
//!
//! **Destroy**: the same, and the repository is deleted from GitHub, for
//! good. The watch is killed (a draining watch would push to a repository
//! that is no more), and the human must have typed the repository's name:
//! the check is made here, not only by the page.
//!
//! Neither touches the factory this view is on — its folder is the view's own
//! working directory — nor a folder the view did not make: only a direct
//! child of the factories' folder goes, a checkout someone else keeps stays.
//! The register is forgotten last, so a step that failed leaves the factory
//! listed and the gesture can be repeated.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use harness_core::ports::store::factories::{Factories, Factory};
use serde::Serialize;

use crate::domain::plant::{self, Action, Gesture};
use crate::ports::{Plant, Teardown};

/// The watch of the factory whose folder is given.
pub type PlantOf = Arc<dyn Fn(&Path) -> Arc<dyn Plant> + Send + Sync>;

/// What the human asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degree {
    /// Let go; the repository stays.
    Delink,
    /// Let go, and delete the repository.
    Destroy,
}

/// Why a factory was not taken down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The register does not know it.
    Unknown(String),
    /// It is the factory this view is on.
    Current,
    /// The name typed is not the one asked for.
    Confirmation(String),
    /// A step failed; the factory is still listed.
    Failed(String),
}

impl Refusal {
    /// The sentence the page shows.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Unknown(id) => format!("no factory {id}"),
            Self::Current => {
                "this view is on that factory: open another one first, then take this one down"
                    .to_string()
            }
            Self::Confirmation(name) => format!("type `{name}` to confirm"),
            Self::Failed(why) => why.clone(),
        }
    }
}

/// What became of the factory's watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Watch {
    /// None ran.
    None,
    /// Asked to stop; it finishes the tasks it runs.
    Draining,
    /// Killed.
    Killed,
}

/// What became of the factory's repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Repository {
    /// Still on GitHub.
    Kept,
    /// Deleted from GitHub.
    Deleted,
    /// There was none to delete: the factory was not linked to one, or it
    /// was gone already.
    Gone,
}

/// What became of the factory's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Folder {
    /// Removed from the disk.
    Removed,
    /// Left in place: the view did not make it.
    Kept,
}

/// What a factory taken down leaves to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The factory's id, no longer in the register.
    pub factory: String,
    /// Its name.
    pub name: String,
    /// Its watch.
    pub watch: Watch,
    /// Its repository.
    pub repository: Repository,
    /// Its folder.
    pub folder: Folder,
}

/// The name a human types to destroy `factory`: its repository's, as GitHub
/// asks it; its own when it has none.
#[must_use]
pub fn confirmation_of(factory: &Factory) -> &str {
    if factory.slug.is_empty() {
        &factory.name
    } else {
        &factory.slug
    }
}

/// Whether GitHub's refusal says the repository is not there any more.
fn is_gone(why: &str) -> bool {
    why.contains("Could not resolve to a Repository") || why.contains("HTTP 404")
}

/// Takes factories down.
pub struct Removal {
    register: Arc<dyn Factories + Send + Sync>,
    teardown: Arc<dyn Teardown>,
    plant_of: PlantOf,
    factories_dir: PathBuf,
    here: String,
}

impl Removal {
    /// A removal working through `register`, from the view of the factory
    /// `here`; the folders it may remove are the children of `factories_dir`.
    #[must_use]
    pub fn new(
        register: Arc<dyn Factories + Send + Sync>,
        teardown: Arc<dyn Teardown>,
        plant_of: PlantOf,
        factories_dir: PathBuf,
        here: &str,
    ) -> Self {
        Self {
            register,
            teardown,
            plant_of,
            factories_dir,
            here: here.to_string(),
        }
    }

    /// Whether `root` is a folder the view made — a direct child of the
    /// factories' folder — and so the only kind a take-down removes.
    #[must_use]
    pub fn is_ours(&self, root: &Path) -> bool {
        root.strip_prefix(&self.factories_dir).is_ok_and(|rest| {
            rest.components().count() == 1
                && rest
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
        })
    }

    /// The watch of `factory`, stopped the way `degree` says.
    fn stop_watch(&self, factory: &Factory, degree: Degree) -> Result<Watch, Refusal> {
        let plant = (self.plant_of)(&factory.root);
        let Some(running) = plant::running(plant.as_ref()) else {
            return Ok(Watch::None);
        };
        let (gesture, stopped) = match degree {
            Degree::Delink => (Gesture::Soft, Watch::Draining),
            Degree::Destroy => (Gesture::Hard, Watch::Killed),
        };
        let actions =
            plant::decide(gesture, Some(running), &plant.processes()).map_err(Refusal::Failed)?;
        for (at, action) in actions.into_iter().enumerate() {
            if let Action::Send(signal, target) = action {
                // The first signal must land; the rest reach processes that
                // may be gone with the watch already.
                if let Err(why) = plant.send(signal, target)
                    && at == 0
                {
                    return Err(Refusal::Failed(why));
                }
            }
        }
        Ok(stopped)
    }

    /// Takes the factory `id` down to `degree`. Blocking: `ps`, `kill`, `gh`
    /// and `git` run here.
    ///
    /// # Errors
    ///
    /// The factory is unknown or is the one this view is on, a destruction
    /// was not confirmed by typing the right name, or a step failed — the
    /// factory is then still in the register.
    pub fn take_down(&self, id: &str, degree: Degree, confirm: &str) -> Result<Report, Refusal> {
        let factory = self
            .register
            .get(id)
            .map_err(|why| Refusal::Failed(why.reason().to_string()))?
            .ok_or_else(|| Refusal::Unknown(id.to_string()))?;
        if factory.id == self.here {
            return Err(Refusal::Current);
        }
        if degree == Degree::Destroy && confirm != confirmation_of(&factory) {
            return Err(Refusal::Confirmation(confirmation_of(&factory).to_string()));
        }

        let watch = self.stop_watch(&factory, degree)?;

        let repository = if degree == Degree::Destroy && !factory.slug.is_empty() {
            match self.teardown.delete_repository(&factory.slug) {
                Ok(()) => Repository::Deleted,
                Err(why) if is_gone(&why) => Repository::Gone,
                Err(why) => return Err(Refusal::Failed(why)),
            }
        } else if degree == Degree::Destroy {
            Repository::Gone
        } else {
            Repository::Kept
        };

        let folder = if self.is_ours(&factory.root) {
            self.teardown
                .remove_folder(&factory.root)
                .map_err(Refusal::Failed)?;
            Folder::Removed
        } else {
            Folder::Kept
        };

        self.register
            .forget(&factory.id)
            .map_err(|why| Refusal::Failed(why.reason().to_string()))?;
        Ok(Report {
            factory: factory.id,
            name: factory.name,
            watch,
            repository,
            folder,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use harness_core::domain::Outcome;

    use super::*;
    use crate::domain::plant::fake::Switch;
    use crate::ports::Process;

    /// A register of fixed factories that remembers who was forgotten.
    struct Book {
        known: Vec<Factory>,
        forgotten: Mutex<Vec<String>>,
    }

    impl Factories for Book {
        fn list(&self) -> Outcome<Vec<Factory>> {
            Ok(self.known.clone())
        }

        fn get(&self, id: &str) -> Outcome<Option<Factory>> {
            Ok(self.known.iter().find(|f| f.id == id).cloned())
        }

        fn enroll(&self, _: &Path, _: &str, _: &str, _: &str) -> Outcome<Factory> {
            unreachable!("a removal never enrolls")
        }

        fn creation(&self, _: &str) -> Outcome<Option<String>> {
            Ok(None)
        }

        fn keep_creation(&self, _: &str, _: &str) -> Outcome<()> {
            Ok(())
        }

        fn end_creation(&self, _: &str) -> Outcome<()> {
            Ok(())
        }

        fn forget(&self, id: &str) -> Outcome<()> {
            self.forgotten.lock().expect("held").push(id.to_string());
            Ok(())
        }
    }

    /// A teardown that writes what it was asked, and fails where told.
    #[derive(Default)]
    struct Wreck {
        said: Mutex<Vec<String>>,
        repository_fails: Option<&'static str>,
        folder_fails: bool,
    }

    impl Teardown for Wreck {
        fn delete_repository(&self, slug: &str) -> Result<(), String> {
            self.said.lock().expect("held").push(format!("repo {slug}"));
            self.repository_fails
                .map_or(Ok(()), |why| Err(why.to_string()))
        }

        fn remove_folder(&self, root: &Path) -> Result<(), String> {
            self.said
                .lock()
                .expect("held")
                .push(format!("folder {}", root.display()));
            if self.folder_fails {
                Err("busy".to_string())
            } else {
                Ok(())
            }
        }
    }

    fn factory(id: &str, root: &str, slug: &str) -> Factory {
        Factory {
            id: id.to_string(),
            name: id.to_uppercase(),
            slug: slug.to_string(),
            root: PathBuf::from(root),
            integration_branch: "main_agent".to_string(),
        }
    }

    fn running_watch() -> Switch {
        Switch {
            processes: vec![Process {
                pid: 100,
                parent: 1,
                group: 100,
                command: "target/debug/harness watch".to_string(),
            }],
            ..Switch::default()
        }
    }

    struct Rig {
        removal: Removal,
        book: Arc<Book>,
        wreck: Arc<Wreck>,
        plant: Arc<Switch>,
    }

    fn rig(wreck: Wreck, plant: Switch) -> Rig {
        let book = Arc::new(Book {
            known: vec![
                factory("here", "/f/here", "me/here"),
                factory("made", "/f/made", "me/made"),
                factory("mine", "/code/mine", "me/mine"),
                factory("loose", "/f/loose", ""),
            ],
            forgotten: Mutex::default(),
        });
        let wreck = Arc::new(wreck);
        let plant = Arc::new(plant);
        let own = Arc::clone(&plant);
        let removal = Removal::new(
            book.clone(),
            wreck.clone(),
            Arc::new(move |_| Arc::clone(&own) as Arc<dyn Plant>),
            PathBuf::from("/f"),
            "here",
        );
        Rig {
            removal,
            book,
            wreck,
            plant,
        }
    }

    fn said(wreck: &Wreck) -> Vec<String> {
        wreck.said.lock().expect("held").clone()
    }

    fn forgotten(book: &Book) -> Vec<String> {
        book.forgotten.lock().expect("held").clone()
    }

    fn sent(plant: &Switch) -> Vec<String> {
        plant.sent.lock().expect("held").clone()
    }

    #[test]
    fn a_delink_stops_the_watch_softly_removes_the_folder_and_keeps_the_repository() {
        let rig = rig(Wreck::default(), running_watch());
        let report = rig
            .removal
            .take_down("made", Degree::Delink, "")
            .expect("delinked");
        assert_eq!(
            report,
            Report {
                factory: "made".to_string(),
                name: "MADE".to_string(),
                watch: Watch::Draining,
                repository: Repository::Kept,
                folder: Folder::Removed,
            }
        );
        assert_eq!(sent(&rig.plant), ["Term Process(100)"]);
        assert_eq!(said(&rig.wreck), ["folder /f/made"]);
        assert_eq!(forgotten(&rig.book), ["made"]);
    }

    #[test]
    fn a_destruction_needs_the_repository_typed_exactly_and_touches_nothing_before() {
        let rig = rig(Wreck::default(), running_watch());
        for typed in ["", "made", "me/MADE", "me/made ", "ME/made"] {
            let why = rig
                .removal
                .take_down("made", Degree::Destroy, typed)
                .expect_err("not confirmed");
            assert_eq!(why, Refusal::Confirmation("me/made".to_string()), "{typed}");
        }
        assert!(sent(&rig.plant).is_empty());
        assert!(said(&rig.wreck).is_empty());
        assert!(forgotten(&rig.book).is_empty());
    }

    #[test]
    fn a_destruction_kills_the_watch_deletes_the_repository_then_the_folder() {
        let rig = rig(Wreck::default(), running_watch());
        let report = rig
            .removal
            .take_down("made", Degree::Destroy, "me/made")
            .expect("destroyed");
        assert_eq!(
            (report.watch, report.repository, report.folder),
            (Watch::Killed, Repository::Deleted, Folder::Removed)
        );
        assert!(
            sent(&rig.plant).iter().all(|s| s.starts_with("Kill")),
            "{:?}",
            sent(&rig.plant)
        );
        assert_eq!(said(&rig.wreck), ["repo me/made", "folder /f/made"]);
        assert_eq!(forgotten(&rig.book), ["made"]);
    }

    #[test]
    fn a_factory_without_a_repository_is_confirmed_by_its_name() {
        let rig = rig(Wreck::default(), Switch::default());
        assert_eq!(
            rig.removal.take_down("loose", Degree::Destroy, "me/loose"),
            Err(Refusal::Confirmation("LOOSE".to_string()))
        );
        let report = rig
            .removal
            .take_down("loose", Degree::Destroy, "LOOSE")
            .expect("destroyed");
        assert_eq!(
            (report.watch, report.repository),
            (Watch::None, Repository::Gone)
        );
        assert_eq!(said(&rig.wreck), ["folder /f/loose"]);
    }

    #[test]
    fn the_factory_the_view_is_on_is_never_taken_down() {
        let rig = rig(Wreck::default(), running_watch());
        for degree in [Degree::Delink, Degree::Destroy] {
            assert_eq!(
                rig.removal.take_down("here", degree, "me/here"),
                Err(Refusal::Current)
            );
        }
        assert!(sent(&rig.plant).is_empty());
        assert!(said(&rig.wreck).is_empty());
        assert!(forgotten(&rig.book).is_empty());
    }

    #[test]
    fn an_unknown_factory_is_refused() {
        let rig = rig(Wreck::default(), Switch::default());
        assert_eq!(
            rig.removal.take_down("ghost", Degree::Delink, ""),
            Err(Refusal::Unknown("ghost".to_string()))
        );
    }

    #[test]
    fn a_folder_the_view_did_not_make_stays_where_it_is() {
        let rig = rig(Wreck::default(), Switch::default());
        let report = rig
            .removal
            .take_down("mine", Degree::Destroy, "me/mine")
            .expect("destroyed");
        assert_eq!(
            (report.repository, report.folder),
            (Repository::Deleted, Folder::Kept)
        );
        assert_eq!(said(&rig.wreck), ["repo me/mine"]);
        assert_eq!(forgotten(&rig.book), ["mine"]);
    }

    #[test]
    fn only_a_direct_child_of_the_factories_folder_is_ours() {
        let rig = rig(Wreck::default(), Switch::default());
        for ours in ["/f/a", "/f/some-app"] {
            assert!(rig.removal.is_ours(Path::new(ours)), "{ours}");
        }
        for not_ours in [
            "/f",
            "/f/",
            "/f/a/b",
            "/f/../etc",
            "/f/a/..",
            "/other/a",
            "/",
            "",
        ] {
            assert!(!rig.removal.is_ours(Path::new(not_ours)), "{not_ours}");
        }
    }

    #[test]
    fn a_repository_github_refuses_to_delete_leaves_the_factory_listed() {
        let rig = rig(
            Wreck {
                repository_fails: Some("gh repo delete failed: HTTP 403: admin rights"),
                ..Wreck::default()
            },
            running_watch(),
        );
        let why = rig
            .removal
            .take_down("made", Degree::Destroy, "me/made")
            .expect_err("refused");
        assert_eq!(
            why,
            Refusal::Failed("gh repo delete failed: HTTP 403: admin rights".to_string())
        );
        assert_eq!(said(&rig.wreck), ["repo me/made"]);
        assert!(forgotten(&rig.book).is_empty());
    }

    #[test]
    fn a_repository_already_gone_does_not_block_the_rest() {
        for gone in [
            "gh repo delete failed: GraphQL: Could not resolve to a Repository with the name 'me/made'.",
            "gh repo delete failed: HTTP 404: Not Found",
        ] {
            let rig = rig(
                Wreck {
                    repository_fails: Some(gone),
                    ..Wreck::default()
                },
                Switch::default(),
            );
            let report = rig
                .removal
                .take_down("made", Degree::Destroy, "me/made")
                .expect("destroyed");
            assert_eq!(report.repository, Repository::Gone, "{gone}");
            assert_eq!(forgotten(&rig.book), ["made"]);
        }
    }

    #[test]
    fn a_folder_that_will_not_go_leaves_the_factory_listed_to_try_again() {
        let rig = rig(
            Wreck {
                folder_fails: true,
                ..Wreck::default()
            },
            Switch::default(),
        );
        let why = rig
            .removal
            .take_down("made", Degree::Delink, "")
            .expect_err("busy");
        assert_eq!(why, Refusal::Failed("busy".to_string()));
        assert!(forgotten(&rig.book).is_empty());
    }

    #[test]
    fn the_watch_stopped_is_the_one_of_that_factory_folder() {
        let asked = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let record = Arc::clone(&asked);
        let book = Arc::new(Book {
            known: vec![factory("made", "/f/made", "me/made")],
            forgotten: Mutex::default(),
        });
        let removal = Removal::new(
            book,
            Arc::new(Wreck::default()),
            Arc::new(move |root| {
                record.lock().expect("held").push(root.to_path_buf());
                Arc::new(Switch::default()) as Arc<dyn Plant>
            }),
            PathBuf::from("/f"),
            "here",
        );
        removal
            .take_down("made", Degree::Delink, "")
            .expect("delinked");
        assert_eq!(*asked.lock().expect("held"), [PathBuf::from("/f/made")]);
    }

    #[test]
    fn the_reason_of_each_refusal_reads_as_a_sentence() {
        assert!(Refusal::Current.reason().contains("open another one first"));
        assert!(
            Refusal::Confirmation("me/app".to_string())
                .reason()
                .contains("`me/app`")
        );
        assert_eq!(Refusal::Unknown("x".to_string()).reason(), "no factory x");
        assert_eq!(Refusal::Failed("boom".to_string()).reason(), "boom");
    }
}
