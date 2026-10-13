//! A factory taken down, in two degrees.
//!
//! **Delink**: the harness lets go of the factory — its watch is asked to
//! stop (soft: the running tasks finish, none starts), what the harness keeps
//! in its folder is removed, it leaves the register — and its GitHub
//! repository stays as it is. The repository can be connected again, later,
//! as any other.
//!
//! **Destroy**: the same, and the repository is deleted from GitHub, for
//! good. The watch is killed (a draining watch would push to a repository
//! that is no more). The token's right to delete is asked *first*: without
//! it nothing is touched and the refusal carries the commands to run by hand.
//!
//! Both need the human to have typed the name asked: the check is made here,
//! not only by the page.
//!
//! The folder goes whole when the view made it — a direct child of the
//! factories' folder — and only its `.llocal/` and `.env.local` otherwise: a
//! checkout someone else keeps stays. The factory this view is on is taken
//! down too, but its folder is the view's working directory: the view moves
//! to another factory, and *that* process removes the folder it left
//! (`Removal::finish`). The register is forgotten last, so a step that failed
//! leaves the factory listed and the gesture can be repeated.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use harness_core::ports::store::factories::{Factories, Factory};
use serde::{Serialize, Serializer};

use crate::domain::plant::{self, Action, Gesture};
use crate::ports::{Plant, Rights, Teardown};

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
    /// It is the factory this view is on, and no other one can take its place.
    Alone,
    /// The name typed is not the one asked for.
    Confirmation(String),
    /// The token may not delete the repository: the commands that would.
    Rights(String),
    /// A step failed; the factory is still listed.
    Failed(String),
}

impl Refusal {
    /// The sentence the page shows.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Unknown(id) => format!("no factory {id}"),
            Self::Alone => {
                "this view is on the only factory there is: create or connect another one first, then take this one down"
                    .to_string()
            }
            Self::Confirmation(name) => format!("type `{name}` to confirm"),
            Self::Rights(slug) => format!(
                "the token may not delete {slug}, and nothing was touched. Grant the right once, \
                 then delete it: `{GRANT}`, then `gh repo delete {slug} --yes`"
            ),
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
    /// Left in place, emptied of what the harness keeps there: the view did
    /// not make it.
    Wiped,
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
    /// The factory the view moves to — when this was the one it was on.
    #[serde(rename = "reopening", serialize_with = "id_of")]
    pub next: Option<Factory>,
}

/// A factory by its id, the way the page reads it.
// serde hands `serialize_with` a reference to the field, whatever its type.
#[allow(clippy::ref_option)]
fn id_of<S: Serializer>(next: &Option<Factory>, out: S) -> Result<S::Ok, S::Error> {
    next.as_ref().map(|factory| &factory.id).serialize(out)
}

/// The command that grants `gh` the right to delete a repository.
const GRANT: &str = "gh auth refresh -h github.com -s delete_repo";

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
    /// The folder of the factory this view was on and has just taken down,
    /// for the process that replaces it to deal with.
    leaving: Mutex<Option<String>>,
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
            leaving: Mutex::default(),
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
    /// The factory is unknown, the name typed is not the one asked for, the
    /// view is on it and no other factory can take its place, the token may
    /// not delete the repository, or a step failed — the factory is then
    /// still in the register.
    pub fn take_down(&self, id: &str, degree: Degree, confirm: &str) -> Result<Report, Refusal> {
        let factory = self
            .register
            .get(id)
            .map_err(|why| Refusal::Failed(why.reason().to_string()))?
            .ok_or_else(|| Refusal::Unknown(id.to_string()))?;
        if confirm != confirmation_of(&factory) {
            return Err(Refusal::Confirmation(confirmation_of(&factory).to_string()));
        }
        let next = if factory.id == self.here {
            Some(self.successor(&factory.id)?)
        } else {
            None
        };
        let deletes = degree == Degree::Destroy && !factory.slug.is_empty();
        if deletes && self.teardown.delete_right() == Rights::Missing {
            return Err(Refusal::Rights(factory.slug));
        }

        let watch = self.stop_watch(&factory, degree)?;

        let repository = match (degree, deletes) {
            (Degree::Delink, _) => Repository::Kept,
            (Degree::Destroy, false) => Repository::Gone,
            (Degree::Destroy, true) => match self.teardown.delete_repository(&factory.slug) {
                Ok(()) => Repository::Deleted,
                Err(why) if is_gone(&why) => Repository::Gone,
                Err(why) => return Err(Refusal::Failed(why)),
            },
        };

        let owned = self.is_ours(&factory.root);
        if next.is_some() {
            // The view works in this folder: it goes once the view has left.
            *self
                .leaving
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(Self::spec(owned, &factory.root));
        } else if owned {
            self.teardown
                .remove_folder(&factory.root)
                .map_err(Refusal::Failed)?;
        } else {
            self.teardown
                .wipe_harness_data(&factory.root)
                .map_err(Refusal::Failed)?;
        }

        self.register
            .forget(&factory.id)
            .map_err(|why| Refusal::Failed(why.reason().to_string()))?;
        Ok(Report {
            factory: factory.id,
            name: factory.name,
            watch,
            repository,
            folder: if owned {
                Folder::Removed
            } else {
                Folder::Wiped
            },
            next,
        })
    }

    /// The factory that takes the view's place when the one it is on goes:
    /// the first other one whose folder is still on the disk.
    fn successor(&self, leaving: &str) -> Result<Factory, Refusal> {
        self.register
            .list()
            .map_err(|why| Refusal::Failed(why.reason().to_string()))?
            .into_iter()
            .find(|factory| factory.id != leaving && factory.root.is_dir())
            .ok_or(Refusal::Alone)
    }

    /// What the next process is told about the folder it must deal with.
    fn spec(owned: bool, root: &Path) -> String {
        format!(
            "{}:{}",
            if owned { "remove" } else { "wipe" },
            root.display()
        )
    }

    /// The folder this view left behind by taking the factory it was on
    /// down, written the way the next process reads it (`finish`).
    #[must_use]
    pub fn leaving(&self) -> Option<String> {
        self.leaving
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Deals with the folder a previous view left (`leaving`): removed whole
    /// if the view made it, emptied of the harness's data otherwise. The
    /// folder this process runs in is never touched.
    ///
    /// # Errors
    ///
    /// The spec is not one `leaving` wrote, the folder is not one the view
    /// may remove, or the disk refused.
    pub fn finish(&self, spec: &str, running_in: &Path) -> Result<(), String> {
        let (how, root) = spec
            .split_once(':')
            .ok_or_else(|| format!("`{spec}` is not a folder to take down"))?;
        let root = Path::new(root);
        if root == running_in {
            return Err(format!("{} is where this view runs", root.display()));
        }
        match how {
            "remove" if self.is_ours(root) => self.teardown.remove_folder(root),
            "remove" => Err(format!("{} is not a folder the view made", root.display())),
            "wipe" => self.teardown.wipe_harness_data(root),
            other => Err(format!("`{other}` is not a way to take a folder down")),
        }
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
        right: Option<Rights>,
    }

    impl Teardown for Wreck {
        fn delete_right(&self) -> Rights {
            self.said.lock().expect("held").push("right".to_string());
            self.right.unwrap_or(Rights::Granted)
        }

        fn wipe_harness_data(&self, root: &Path) -> Result<(), String> {
            self.said
                .lock()
                .expect("held")
                .push(format!("wipe {}", root.display()));
            Ok(())
        }

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
            .take_down("made", Degree::Delink, "me/made")
            .expect("delinked");
        assert_eq!(
            report,
            Report {
                factory: "made".to_string(),
                name: "MADE".to_string(),
                watch: Watch::Draining,
                repository: Repository::Kept,
                folder: Folder::Removed,
                next: None,
            }
        );
        assert_eq!(sent(&rig.plant), ["Term Process(100)"]);
        assert_eq!(said(&rig.wreck), ["folder /f/made"]);
        assert_eq!(forgotten(&rig.book), ["made"]);
    }

    #[test]
    fn a_take_down_needs_the_repository_typed_exactly_and_touches_nothing_before() {
        let rig = rig(Wreck::default(), running_watch());
        for typed in ["", "made", "me/MADE", "me/made ", "ME/made"] {
            for degree in [Degree::Delink, Degree::Destroy] {
                let why = rig
                    .removal
                    .take_down("made", degree, typed)
                    .expect_err("not confirmed");
                assert_eq!(why, Refusal::Confirmation("me/made".to_string()), "{typed}");
            }
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
        assert_eq!(
            said(&rig.wreck),
            ["right", "repo me/made", "folder /f/made"]
        );
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

    /// A register whose factories sit in folders that exist.
    fn lived_in(rig_root: &Path) -> Rig {
        let book = Arc::new(Book {
            known: vec![
                factory(
                    "here",
                    &rig_root.join("here").display().to_string(),
                    "me/here",
                ),
                factory("other", &rig_root.display().to_string(), "me/other"),
            ],
            forgotten: Mutex::default(),
        });
        let wreck = Arc::new(Wreck::default());
        let plant = Arc::new(running_watch());
        let own = Arc::clone(&plant);
        Rig {
            removal: Removal::new(
                book.clone(),
                wreck.clone(),
                Arc::new(move |_| Arc::clone(&own) as Arc<dyn Plant>),
                rig_root.to_path_buf(),
                "here",
            ),
            book,
            wreck,
            plant,
        }
    }

    #[test]
    fn the_factory_the_view_is_on_leaves_its_folder_to_the_view_that_replaces_it() {
        let tmp = std::env::temp_dir();
        let rig = lived_in(&tmp);
        let report = rig
            .removal
            .take_down("here", Degree::Delink, "me/here")
            .expect("delinked");
        assert_eq!(report.next.map(|f| f.id), Some("other".to_string()));
        assert_eq!(report.folder, Folder::Removed);
        // Nothing of its folder is touched now; the next view is told.
        assert!(said(&rig.wreck).is_empty());
        assert_eq!(
            rig.removal.leaving(),
            Some(format!("remove:{}", tmp.join("here").display()))
        );
        assert_eq!(forgotten(&rig.book), ["here"]);
    }

    #[test]
    fn the_only_factory_cannot_be_taken_down_from_inside_and_nothing_is_touched() {
        let rig = rig(Wreck::default(), running_watch());
        // None of the other roots of this register exists on the disk.
        for degree in [Degree::Delink, Degree::Destroy] {
            assert_eq!(
                rig.removal.take_down("here", degree, "me/here"),
                Err(Refusal::Alone)
            );
        }
        assert!(sent(&rig.plant).is_empty());
        assert!(said(&rig.wreck).is_empty());
        assert!(forgotten(&rig.book).is_empty());
        assert_eq!(rig.removal.leaving(), None);
    }

    #[test]
    fn without_the_right_to_delete_nothing_is_touched_and_the_commands_are_given() {
        let rig = rig(
            Wreck {
                right: Some(Rights::Missing),
                ..Wreck::default()
            },
            running_watch(),
        );
        let why = rig
            .removal
            .take_down("made", Degree::Destroy, "me/made")
            .expect_err("no right");
        assert_eq!(why, Refusal::Rights("me/made".to_string()));
        let said_so = why.reason();
        assert!(
            said_so.contains("gh auth refresh -h github.com -s delete_repo"),
            "{said_so}"
        );
        assert!(
            said_so.contains("gh repo delete me/made --yes"),
            "{said_so}"
        );
        assert_eq!(said(&rig.wreck), ["right"]);
        assert!(sent(&rig.plant).is_empty());
        assert!(forgotten(&rig.book).is_empty());
    }

    #[test]
    fn an_unknown_right_is_tried_and_a_delink_never_asks() {
        let unknown = rig(
            Wreck {
                right: Some(Rights::Unknown),
                ..Wreck::default()
            },
            Switch::default(),
        );
        unknown
            .removal
            .take_down("made", Degree::Destroy, "me/made")
            .expect("tried");
        assert_eq!(
            said(&unknown.wreck),
            ["right", "repo me/made", "folder /f/made"]
        );

        let missing = rig(
            Wreck {
                right: Some(Rights::Missing),
                ..Wreck::default()
            },
            Switch::default(),
        );
        missing
            .removal
            .take_down("made", Degree::Delink, "me/made")
            .expect("delinked");
        assert_eq!(said(&missing.wreck), ["folder /f/made"]);
    }

    #[test]
    fn a_view_that_replaces_another_removes_or_wipes_the_folder_left_but_never_its_own() {
        let rig = rig(Wreck::default(), Switch::default());
        assert_eq!(
            rig.removal.finish("remove:/f/old", Path::new("/f/new")),
            Ok(())
        );
        assert_eq!(
            rig.removal.finish("wipe:/code/old", Path::new("/f/new")),
            Ok(())
        );
        assert_eq!(said(&rig.wreck), ["folder /f/old", "wipe /code/old"]);

        // Not a folder the view made, itself, or something unreadable.
        for (spec, here) in [
            ("remove:/code/old", "/f/new"),
            ("remove:/f/new", "/f/new"),
            ("wipe:/f/new", "/f/new"),
            ("remove:/f/a/b", "/f/new"),
            ("burn:/f/old", "/f/new"),
            ("/f/old", "/f/new"),
        ] {
            assert!(rig.removal.finish(spec, Path::new(here)).is_err(), "{spec}");
        }
        assert_eq!(said(&rig.wreck).len(), 2);
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
            (Repository::Deleted, Folder::Wiped)
        );
        assert_eq!(
            said(&rig.wreck),
            ["right", "repo me/mine", "wipe /code/mine"]
        );
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
        assert_eq!(said(&rig.wreck), ["right", "repo me/made"]);
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
            .take_down("made", Degree::Delink, "me/made")
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
            .take_down("made", Degree::Delink, "me/made")
            .expect("delinked");
        assert_eq!(*asked.lock().expect("held"), [PathBuf::from("/f/made")]);
    }

    /// A watch that refuses every signal from the `ok`th on.
    struct Stubborn {
        ok: usize,
        sent: Mutex<usize>,
    }

    impl Plant for Stubborn {
        fn processes(&self) -> Vec<Process> {
            running_watch().processes
        }

        fn works_here(&self, _pid: u32) -> bool {
            true
        }

        fn start(&self, _lanes: Option<usize>) -> Result<u32, String> {
            Err("never started".to_string())
        }

        fn output_tail(&self) -> String {
            String::new()
        }

        fn send(
            &self,
            _signal: crate::ports::Signal,
            _target: crate::ports::Target,
        ) -> Result<(), String> {
            let mut sent = self.sent.lock().expect("held");
            *sent += 1;
            if *sent > self.ok {
                Err("no such process".to_string())
            } else {
                Ok(())
            }
        }
    }

    fn removal_over(plant: Stubborn) -> (Removal, Arc<Book>) {
        let book = Arc::new(Book {
            known: vec![factory("made", "/f/made", "me/made")],
            forgotten: Mutex::default(),
        });
        let plant = Arc::new(plant);
        let removal = Removal::new(
            book.clone(),
            Arc::new(Wreck::default()),
            Arc::new(move |_| Arc::clone(&plant) as Arc<dyn Plant>),
            PathBuf::from("/f"),
            "here",
        );
        (removal, book)
    }

    #[test]
    fn a_first_signal_that_does_not_land_stops_the_take_down_where_it_is() {
        for degree in [Degree::Delink, Degree::Destroy] {
            let (removal, book) = removal_over(Stubborn {
                ok: 0,
                sent: Mutex::default(),
            });
            assert_eq!(
                removal.take_down("made", degree, "me/made"),
                Err(Refusal::Failed("no such process".to_string()))
            );
            assert!(forgotten(&book).is_empty());
        }
    }

    #[test]
    fn the_signals_after_the_first_may_find_the_watch_gone() {
        for degree in [Degree::Delink, Degree::Destroy] {
            let (removal, book) = removal_over(Stubborn {
                ok: 1,
                sent: Mutex::default(),
            });
            assert!(removal.take_down("made", degree, "me/made").is_ok());
            assert_eq!(forgotten(&book), ["made"]);
        }
    }

    #[test]
    fn the_reason_of_each_refusal_reads_as_a_sentence() {
        assert!(
            Refusal::Alone
                .reason()
                .contains("create or connect another one")
        );
        assert!(
            Refusal::Confirmation("me/app".to_string())
                .reason()
                .contains("`me/app`")
        );
        assert_eq!(Refusal::Unknown("x".to_string()).reason(), "no factory x");
        assert_eq!(Refusal::Failed("boom".to_string()).reason(), "boom");
    }
}
