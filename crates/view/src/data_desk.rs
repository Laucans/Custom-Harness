//! The data desk: answers the page's questions about the product's data
//! model — which versions the archive keeps, and one version merged — from
//! one task that owns the `Product` port.
//!
//! The port is `?Send` (it wraps core's `GitHub`), the HTTP handlers are not
//! allowed to hold it: they send a question down a channel and wait for the
//! answer. An archived version never changes, so it is read once and kept;
//! the archive's index and the live files are read again after a while.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use harness_workflows::data_snapshot::action::archive::{INDEX, MODEL_FILE, SCHEMA_FILE};
use harness_workflows::data_snapshot::config::ARCHIVE_BRANCH;
use harness_workflows::data_snapshot::data::index::Index;
use jiff::tz::TimeZone;
use tokio::sync::{mpsc, oneshot};

use crate::domain::data_versions::{self, Versions};
use crate::ports::Product;
use harness_workflows::data_snapshot::data::model::{
    self as data_model, DataModel, MODEL_PATH, SCHEMA_PATH,
};

/// How long the archive's index is trusted before it is read again.
const INDEX_FRESH: Duration = Duration::from_secs(20);

/// One version the page asked for: a branch, a folder, or neither.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    /// The branch, newest version.
    pub branch: Option<String>,
    /// A version, by its folder on the archive branch.
    pub dir: Option<String>,
}

/// A version merged, and which one it is.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The model.
    pub model: Arc<DataModel>,
    /// The branch shown; empty for the live files.
    pub branch: String,
    /// The version's folder; empty for the live files.
    pub dir: String,
}

enum Ask {
    Versions(oneshot::Sender<Arc<Versions>>),
    Model(Wanted, oneshot::Sender<Result<Shown, String>>),
    Issue(Issue, oneshot::Sender<Result<String, String>>),
}

/// An issue to open on the product's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Its title.
    pub title: String,
    /// Its body, markdown.
    pub body: String,
    /// Its labels.
    pub labels: Vec<String>,
}

/// The handle the HTTP side keeps: `Send`, cheap to clone.
#[derive(Clone)]
pub struct DataDesk {
    tx: mpsc::Sender<Ask>,
}

impl DataDesk {
    /// The versions the archive keeps.
    ///
    /// # Errors
    ///
    /// The desk is gone.
    pub async fn versions(&self) -> Result<Arc<Versions>, String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Ask::Versions(reply))
            .await
            .map_err(|_| "the data desk is closed".to_string())?;
        answer
            .await
            .map_err(|_| "the data desk is closed".to_string())
    }

    /// Opens an issue on the product's board, and says where it is.
    ///
    /// # Errors
    ///
    /// GitHub refused, or the desk is gone.
    pub async fn open_issue(&self, issue: Issue) -> Result<String, String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Ask::Issue(issue, reply))
            .await
            .map_err(|_| "the data desk is closed".to_string())?;
        answer
            .await
            .map_err(|_| "the data desk is closed".to_string())?
    }

    /// One version, merged.
    ///
    /// # Errors
    ///
    /// The version is not in the archive, or the desk is gone.
    pub async fn model(&self, wanted: Wanted) -> Result<Shown, String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Ask::Model(wanted, reply))
            .await
            .map_err(|_| "the data desk is closed".to_string())?;
        answer
            .await
            .map_err(|_| "the data desk is closed".to_string())?
    }
}

/// The desk and the task that serves it; the task must run on a `LocalSet`.
///
/// `zone` is the one a version's clock is shown in: the harness machine's.
pub fn open(
    product: Rc<dyn Product>,
    live_every: Duration,
    zone: TimeZone,
) -> (DataDesk, impl Future<Output = ()>) {
    let (tx, rx) = mpsc::channel(16);
    (DataDesk { tx }, serve(product, rx, live_every, zone))
}

struct Kept {
    index: Option<(Instant, Option<Index>)>,
    live: Option<(Instant, Arc<DataModel>)>,
    versions: HashMap<String, Arc<DataModel>>,
}

async fn serve(
    product: Rc<dyn Product>,
    mut rx: mpsc::Receiver<Ask>,
    live_every: Duration,
    zone: TimeZone,
) {
    let mut kept = Kept {
        index: None,
        live: None,
        versions: HashMap::new(),
    };
    while let Some(ask) = rx.recv().await {
        match ask {
            Ask::Versions(reply) => {
                let index = index(&*product, &mut kept).await;
                let _ = reply.send(Arc::new(data_versions::versions(
                    index.as_ref(),
                    &product.live_ref(),
                )));
            }
            Ask::Model(wanted, reply) => {
                let shown = model(&*product, &mut kept, &wanted, live_every, &zone).await;
                let _ = reply.send(shown);
            }
            Ask::Issue(issue, reply) => {
                let labels: Vec<&str> = issue.labels.iter().map(String::as_str).collect();
                let opened = product
                    .open_issue(&issue.title, &issue.body, &labels)
                    .await
                    .map_err(|h| h.to_string());
                let _ = reply.send(opened);
            }
        }
    }
}

/// The archive's index, read again once stale; `None` when there is no archive.
async fn index(product: &dyn Product, kept: &mut Kept) -> Option<Index> {
    if let Some((at, index)) = &kept.index
        && at.elapsed() < INDEX_FRESH
    {
        return index.clone();
    }
    let read = product.file(INDEX, ARCHIVE_BRANCH).await;
    let index = match read {
        Ok(Some(text)) => Some(Index::parse(Some(&text))),
        Ok(None) => None,
        // A failed read keeps what was known, and is tried at the next ask.
        Err(_) => return kept.index.as_ref().and_then(|(_, i)| i.clone()),
    };
    kept.index = Some((Instant::now(), index.clone()));
    index
}

async fn model(
    product: &dyn Product,
    kept: &mut Kept,
    wanted: &Wanted,
    live_every: Duration,
    zone: &TimeZone,
) -> Result<Shown, String> {
    let live_ref = product.live_ref();
    let Some(index) = index(product, kept)
        .await
        .filter(|i| !i.snapshots.is_empty())
    else {
        // No archive yet: the live files of the integration branch.
        return Ok(live(product, kept, &live_ref, live_every).await);
    };
    let snapshot = data_versions::pick(
        &index,
        wanted.branch.as_deref(),
        wanted.dir.as_deref(),
        &live_ref,
    )
    .cloned()
    .ok_or_else(|| "no such version in the archive".to_string())?;
    if let Some(model) = kept.versions.get(&snapshot.dir) {
        return Ok(Shown {
            model: Arc::clone(model),
            branch: snapshot.branch,
            dir: snapshot.dir,
        });
    }
    let read = |file: &str| {
        let path = format!("{}/{file}", snapshot.dir);
        async move {
            product
                .file(&path, ARCHIVE_BRANCH)
                .await
                .map_err(|h| h.to_string())
        }
    };
    let schema = read(SCHEMA_FILE).await;
    let declared = read(MODEL_FILE).await;
    let short: String = snapshot.sha.chars().take(7).collect();
    let origin = format!(
        "{} · {} @ {short} · {}",
        product.origin(),
        snapshot.branch,
        local_clock(&snapshot.at, zone)
    );
    let model = Arc::new(data_model::build(&origin, &schema, &declared));
    // Kept only when both reads answered: a failed read is tried again.
    if schema.is_ok() && declared.is_ok() {
        kept.versions
            .insert(snapshot.dir.clone(), Arc::clone(&model));
    }
    Ok(Shown {
        model,
        branch: snapshot.branch,
        dir: snapshot.dir,
    })
}

/// A trace clock (`2026-10-02T14:00:00Z`) as the wall clock of `zone`
/// (`2026-10-02 10:00:00`); one that does not parse is shown as written.
fn local_clock(at: &str, zone: &TimeZone) -> String {
    at.parse::<jiff::Timestamp>().map_or_else(
        |_| at.to_string(),
        |at| {
            at.to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M:%S")
                .to_string()
        },
    )
}

async fn live(product: &dyn Product, kept: &mut Kept, live_ref: &str, every: Duration) -> Shown {
    let fresh = kept.live.as_ref().filter(|(at, _)| at.elapsed() < every);
    let model = if let Some((_, model)) = fresh {
        Arc::clone(model)
    } else {
        let schema = product
            .file(SCHEMA_PATH, live_ref)
            .await
            .map_err(|h| h.to_string());
        let declared = product
            .file(MODEL_PATH, live_ref)
            .await
            .map_err(|h| h.to_string());
        let origin = format!(
            "{} @ {live_ref} (live files, no archive yet)",
            product.origin()
        );
        let model = Arc::new(data_model::build(&origin, &schema, &declared));
        kept.live = Some((Instant::now(), Arc::clone(&model)));
        model
    };
    Shown {
        model,
        branch: String::new(),
        dir: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use async_trait::async_trait;
    use harness_core::domain::Outcome;

    use super::*;

    /// A product in memory that counts its reads.
    #[derive(Default)]
    struct Shelf {
        files: HashMap<(String, String), String>,
        reads: RefCell<usize>,
    }

    #[async_trait(?Send)]
    impl Product for Shelf {
        fn origin(&self) -> String {
            "o/r".to_string()
        }
        fn live_ref(&self) -> String {
            "main_agent".to_string()
        }
        async fn file(&self, path: &str, git_ref: &str) -> Outcome<Option<String>> {
            *self.reads.borrow_mut() += 1;
            Ok(self
                .files
                .get(&(path.to_string(), git_ref.to_string()))
                .cloned())
        }
        async fn open_issue(&self, title: &str, _body: &str, labels: &[&str]) -> Outcome<String> {
            Ok(format!("{title} [{}]", labels.join(",")))
        }
    }

    fn put(shelf: &mut Shelf, path: &str, git_ref: &str, text: &str) {
        shelf
            .files
            .insert((path.to_string(), git_ref.to_string()), text.to_string());
    }

    #[tokio::test]
    async fn without_an_archive_the_live_files_are_shown() {
        let mut shelf = Shelf::default();
        put(
            &mut shelf,
            SCHEMA_PATH,
            "main_agent",
            "CREATE TABLE live (a int);",
        );
        let shelf = Rc::new(shelf);
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let (desk, task) = open(
                    Rc::clone(&shelf) as Rc<dyn Product>,
                    Duration::from_secs(60),
                    TimeZone::fixed(jiff::tz::offset(-4)),
                );
                let task = tokio::task::spawn_local(task);
                let versions = desk.versions().await.expect("answered");
                assert!(!versions.archive);
                let opened = desk
                    .open_issue(Issue {
                        title: "t".into(),
                        body: "b".into(),
                        labels: vec!["harness:roadmap".into()],
                    })
                    .await
                    .expect("opened");
                assert_eq!(opened, "t [harness:roadmap]");
                let shown = desk.model(Wanted::default()).await.expect("answered");
                assert!(shown.dir.is_empty());
                assert_eq!(shown.model.tables[0].name, "live");
                task.abort();
            })
            .await;
    }

    #[tokio::test]
    async fn an_archived_version_is_read_once_and_kept() {
        let index = r#"{"version":1,"heads":{"main_agent":"b"},"snapshots":[
            {"branch":"main_agent","sha":"aaaaaaa1","at":"2026-10-01T10:00:00Z","dir":"main_agent/v1","schema":true,"model":false},
            {"branch":"main_agent","sha":"bbbbbbb2","at":"2026-10-02T10:00:00Z","dir":"main_agent/v2","schema":true,"model":false}]}"#;
        let mut shelf = Shelf::default();
        put(&mut shelf, INDEX, ARCHIVE_BRANCH, index);
        put(
            &mut shelf,
            "main_agent/v1/schema.sql",
            ARCHIVE_BRANCH,
            "CREATE TABLE one (a int);",
        );
        put(
            &mut shelf,
            "main_agent/v2/schema.sql",
            ARCHIVE_BRANCH,
            "CREATE TABLE one (a int); CREATE TABLE two (b int);",
        );
        let shelf = Rc::new(shelf);
        let local = tokio::task::LocalSet::new();
        let counted = Rc::clone(&shelf);
        local
            .run_until(async move {
                let (desk, task) = open(
                    Rc::clone(&shelf) as Rc<dyn Product>,
                    Duration::from_secs(60),
                    TimeZone::fixed(jiff::tz::offset(-4)),
                );
                let task = tokio::task::spawn_local(task);
                let versions = desk.versions().await.expect("answered");
                assert_eq!(versions.branches[0].versions.len(), 2);
                let newest = desk.model(Wanted::default()).await.expect("answered");
                assert_eq!(newest.dir, "main_agent/v2");
                assert_eq!(newest.model.tables.len(), 2);
                assert!(
                    newest
                        .model
                        .origin
                        .contains("main_agent @ bbbbbbb · 2026-10-02 06:00:00")
                );
                let old = desk
                    .model(Wanted {
                        branch: None,
                        dir: Some("main_agent/v1".into()),
                    })
                    .await
                    .expect("answered");
                assert_eq!(old.model.tables.len(), 1);
                let before = *counted.reads.borrow();
                let again = desk
                    .model(Wanted {
                        branch: None,
                        dir: Some("main_agent/v1".into()),
                    })
                    .await
                    .expect("answered");
                assert_eq!(again.model.tables.len(), 1);
                assert_eq!(*counted.reads.borrow(), before, "kept, not read again");
                assert!(
                    desk.model(Wanted {
                        branch: Some("nowhere".into()),
                        dir: None,
                    })
                    .await
                    .is_err()
                );
                task.abort();
            })
            .await;
    }
}
