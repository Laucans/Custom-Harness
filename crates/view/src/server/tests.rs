//! The HTTP side, driven through the router with fakes behind the state.

use super::desks::Control;
use super::*;
use crate::desk::fake::Echoing;
use crate::domain::annotations;
use crate::domain::assemble::{self, Inputs};
use crate::domain::blueprint;
use crate::domain::drafts::Draft;
use crate::domain::observe::Observed;
use crate::domain::observe::fake::Shelf;
use crate::domain::plant::fake::Switch;
use crate::domain::snapshot::Project;
use crate::ports::Process;
use crate::ports::TrialAsk;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use tower::ServiceExt as _;

fn state(shelf: Shelf, desk: Option<Desk>) -> AppState {
    let lines = blueprint::lines();
    let project = Project {
        name: "p".to_string(),
        slug: String::new(),
        url: String::new(),
        integration_branch: "main_agent".to_string(),
    };
    let observed = Observed::default();
    let told = crate::domain::gates::Told::default();
    let snap = assemble::snapshot(&Inputs {
        observed: &observed,
        board: None,
        project: &project,
        lines: &lines,
        now: 0,
        demo: false,
        told: &told,
        zone: &jiff::tz::TimeZone::UTC,
    });
    let (_, snapshot) = watch::channel(Arc::new(snap));
    let (_, board) = watch::channel(None);
    AppState {
        snapshot,
        pulse: crate::pulse::Pulse::default(),
        board,
        traces: Arc::new(shelf),
        static_dir: None,
        render_dir: PathBuf::from("/nonexistent/render"),
        desk,
        doctor: None,
        diagnoses: Arc::default(),
        asking: Arc::default(),
        plant: None,
        start_grace: Duration::ZERO,
        limits: None,
        claude_read: Arc::default(),
        events: None,
        janitor: None,
        data: None,
        pins: Arc::new(Pins::default()),
        drafts: Arc::new(Versions::default()),
        steps: Arc::new(Steps::default()),
        history: Arc::new(Lines::default()),
        journal: Arc::new(Diary::default()),
        reworks: Arc::default(),
        note_desks: None,
        mockups: None,
        kit: None,
        mf: None,
        builds: None,
        integration_branch: "main_agent".to_string(),
        prompts: Arc::new(Book::default()),
        prompts_lock: Arc::default(),
        trials: Some(Arc::new(Bench::default())),
        restart: Arc::default(),
        restart_poll: Duration::from_millis(5),
        units: None,
        zone: jiff::tz::TimeZone::UTC,
        factories: None,
        creation: None,
        usage: None,
        factory: String::new(),
        switching: Arc::new(watch::channel(None).0),
        removal: None,
    }
}

/// A prompt book and its drafts, in memory.
#[derive(Default)]
struct Book(
    std::sync::Mutex<(
        harness_core::domain::prompt_book::PromptBook,
        harness_core::domain::prompt_book::Drafts,
        harness_core::domain::prompt_book::Shelf,
    )>,
);

impl Book {
    /// The factory starts: the drafts become what runs.
    fn start(&self) {
        let mut both = self.0.lock().expect("book");
        let drafts = std::mem::take(&mut both.1);
        both.0
            .apply(&drafts, harness_workflows::prompts::default_of, 9);
    }
}

impl Prompts for Book {
    fn book(&self) -> Result<harness_core::domain::prompt_book::PromptBook, String> {
        Ok(self.0.lock().expect("book").0.clone())
    }

    fn drafts(&self) -> Result<harness_core::domain::prompt_book::Drafts, String> {
        Ok(self.0.lock().expect("book").1.clone())
    }

    fn keep_drafts(
        &self,
        drafts: &harness_core::domain::prompt_book::Drafts,
    ) -> Result<(), String> {
        self.0.lock().expect("book").1 = drafts.clone();
        Ok(())
    }

    fn shelf(&self) -> Result<harness_core::domain::prompt_book::Shelf, String> {
        Ok(self.0.lock().expect("book").2.clone())
    }

    fn keep_shelf(
        &self,
        deactivated: &harness_core::domain::prompt_book::Shelf,
    ) -> Result<(), String> {
        self.0.lock().expect("book").2 = deactivated.clone();
        Ok(())
    }
}

/// Trials that never run: what was asked is kept, and reported running.
#[derive(Default)]
struct Bench(std::sync::Mutex<Vec<TrialAsk>>);

impl Trials for Bench {
    fn start(&self, ask: &TrialAsk) -> Result<(), String> {
        self.0.lock().expect("bench").push(ask.clone());
        Ok(())
    }

    fn reports(&self) -> Vec<(String, harness_core::domain::prompt_book::TrialReport)> {
        self.0
            .lock()
            .expect("bench")
            .iter()
            .map(|ask| {
                (
                    ask.id.clone(),
                    harness_core::domain::prompt_book::TrialReport {
                        key: ask.key.clone(),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }
}

#[tokio::test]
async fn an_edit_waits_for_the_start_and_a_revert_is_kept_once_it_ran() {
    let book = Arc::new(Book::default());
    let mut state = state(Shelf::default(), None);
    state.prompts = Arc::clone(&book) as Arc<dyn Prompts>;
    let (status, shown) = send(state.clone(), "GET", "/api/prompts/split/slice", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        shown.contains("\"edited\":false") && shown.contains("\"draft\":null"),
        "{shown}"
    );

    let edit = r#"{"key":"split/slice","text":"Slice #{num}"}"#;
    let (status, saved) = send(state.clone(), "POST", "/api/prompts/edit", edit).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(
        saved.contains("\"edited\":false"),
        "nothing runs it before a start: {saved}"
    );
    let (_, waiting) = send(state.clone(), "GET", "/api/pending", "").await;
    assert!(
        waiting.contains("\"key\":\"split/slice\"") && waiting.contains("\"kind\":\"edit\""),
        "{waiting}"
    );

    book.start();
    let (_, shown) = send(state.clone(), "GET", "/api/prompts/split/slice", "").await;
    assert!(
        shown.contains("\"running\":\"Slice #{num}\"") && shown.contains("\"edited\":true"),
        "{shown}"
    );
    let (_, waiting) = send(state.clone(), "GET", "/api/pending", "").await;
    assert!(waiting.contains("\"changes\":[]"), "{waiting}");

    let back = r#"{"key":"split/slice","from":"default"}"#;
    let (status, _) = send(state.clone(), "POST", "/api/prompts/restore", back).await;
    assert_eq!(status, StatusCode::OK);
    book.start();
    let (_, shown) = send(state.clone(), "GET", "/api/prompts/split/slice", "").await;
    assert!(
        shown.contains("\"reverted\":[{\"text\":\"Slice #{num}\""),
        "{shown}"
    );

    let far = r#"{"key":"split/slice","from":"history","index":4}"#;
    let (status, _) = send(state.clone(), "POST", "/api/prompts/restore", far).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        state.clone(),
        "POST",
        "/api/prompts/edit",
        r#"{"key":"nope","text":"x"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, none) = send(state, "GET", "/api/prompts/split/publish", "").await;
    assert_eq!(none, "[]");
}

#[tokio::test]
async fn a_dry_run_starts_once_on_its_own_line_with_the_station_s_model() {
    let state = state(Shelf::default(), None);
    let ask = |run: &str, line: &str| {
        format!(
            r#"{{"key":"split/slice","version":"draft","text":"Slice #{{num}}","workflow":"{line}","run":"{run}"}}"#
        )
    };
    let (status, why) = send(
        state.clone(),
        "POST",
        "/api/prompts/trial",
        &ask("20261009-120000-12", "planner"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
    let (status, started) = send(
        state.clone(),
        "POST",
        "/api/prompts/trial",
        &ask("20261009-120000-12", "split"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let (status, _) = send(
        state.clone(),
        "POST",
        "/api/prompts/trial",
        &ask("20261009-120000-12", "split"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "one at a time");
    let (_, listed) = send(state, "GET", "/api/prompts/trials?key=split/slice", "").await;
    assert!(
        listed.contains("\"key\":\"split/slice\"") && listed.contains("\"state\":\"running\""),
        "{listed}"
    );
}

#[tokio::test]
async fn a_restart_drains_then_starts_and_says_so() {
    let mut state = state(Shelf::default(), None);
    let switch = Arc::new(Switch::default());
    switch.start(None).expect("running");
    state.plant = Some(Arc::clone(&switch) as Arc<dyn Plant>);
    let (status, said) = send(state.clone(), "POST", "/api/restart", "").await;
    assert_eq!(status, StatusCode::OK, "{said}");
    assert!(said.contains("soft stop"), "{said}");
    let (status, _) = send(state.clone(), "POST", "/api/restart", "").await;
    assert_eq!(status, StatusCode::CONFLICT);
    *switch.started.lock().expect("table") = None;
    for _ in 0..100 {
        let (_, waiting) = send(state.clone(), "GET", "/api/pending", "").await;
        if waiting.contains("\"state\":\"started\"") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the factory never started again");
}

/// A pins' store in memory.
/// A product with one mocked page on every branch.
struct FakeMockups;

impl crate::ports::Mockups for FakeMockups {
    fn state(&self, branch: &str) -> Result<crate::ports::MockState, String> {
        Ok(crate::ports::MockState {
            branch: branch.to_string(),
            present: true,
            pages: vec![crate::ports::MockPage {
                path: "pages/home.html".into(),
                title: "Home".into(),
                ..crate::ports::MockPage::default()
            }],
            components: Vec::new(),
            edited: false,
            saved: String::new(),
            discarded: false,
            stamp: "s1".into(),
        })
    }
    fn file(&self, _branch: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        Ok((path == "pages/home.html").then(|| b"<h1>hi</h1>".to_vec()))
    }
    fn product_files(
        &self,
        _branch: &str,
        pattern: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        Ok([
            ("Cargo.toml", "secret = 1"),
            ("plan/graph.json", r#"{"ids":{}}"#),
            (
                "crates/table/data-capabilities/move/data-capability.json",
                r#"{"dataCapability":"table.moveToken","payload":{"token":"uuid"}}"#,
            ),
        ]
        .into_iter()
        .filter(|(path, _)| *path == pattern || crate::domain::mockup::matches(pattern, path))
        .map(|(path, text)| (path.to_string(), text.as_bytes().to_vec()))
        .collect())
    }
    fn workspace(&self, _branch: &str) -> Result<PathBuf, String> {
        Err("no workspace in a test".into())
    }
    fn save(&self, _branch: &str, name: &str) -> Result<String, String> {
        if name.is_empty() {
            Err("nothing to save".into())
        } else {
            Ok(format!("mockup/{name}"))
        }
    }
    fn discard(&self, _branch: &str) -> Result<(), String> {
        Ok(())
    }

    fn restore(&self, _branch: &str) -> Result<(), String> {
        Ok(())
    }
    fn files(&self, _branch: &str) -> Result<Vec<crate::ports::MockFile>, String> {
        Ok(vec![
            crate::ports::MockFile {
                path: "fixtures/a.json".into(),
                at: 5,
            },
            crate::ports::MockFile {
                path: "pages/home.html".into(),
                at: 9,
            },
        ])
    }
    fn write(&self, _branch: &str, path: &str, _bytes: &[u8]) -> Result<(), String> {
        if path.contains("..") {
            Err("refused".into())
        } else {
            Ok(())
        }
    }
    fn remove(&self, _branch: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }
    fn branches(&self) -> Result<Vec<String>, String> {
        Ok(vec!["main".into()])
    }
    fn ensure_branch(&self, _branch: &str, _from: &str) -> Result<(), String> {
        Ok(())
    }
    fn publish(&self, _branch: &str, _message: &str) -> Result<String, String> {
        Err("nothing to publish".into())
    }
    fn carry(
        &self,
        _from: &str,
        _to: &str,
        _folders: &[&str],
        _message: &str,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
}

/// Draft versions in memory.
/// Each pin's journal, in memory.
#[derive(Default)]
struct Diary(std::sync::Mutex<Vec<(String, crate::domain::pin_journal::Event)>>);

// A test fake: each method holds its lock for the whole of its few lines.
#[allow(clippy::significant_drop_tightening)]
impl crate::ports::PinJournal for Diary {
    fn events(&self, pin: &str) -> Result<Vec<crate::domain::pin_journal::Event>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        let mut events: Vec<_> = kept
            .iter()
            .filter(|(p, _)| p == pin)
            .map(|(_, e)| e.clone())
            .collect();
        events.sort_by_key(|e| e.seq);
        Ok(events)
    }
    fn put(&self, pin: &str, event: &crate::domain::pin_journal::Event) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(p, e)| !(p == pin && e.seq == event.seq));
        kept.push((pin.to_string(), event.clone()));
        Ok(())
    }
}

/// The mock-up's history, in memory.
#[derive(Default)]
struct Lines(std::sync::Mutex<Vec<(String, crate::domain::mock_history::Entry)>>);

// A test fake: each method holds its lock for the whole of its few lines.
#[allow(clippy::significant_drop_tightening)]
impl crate::ports::HistoryStore for Lines {
    fn entries(&self, branch: &str) -> Result<Vec<crate::domain::mock_history::Entry>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        let mut entries: Vec<_> = kept
            .iter()
            .filter(|(b, _)| b == branch)
            .map(|(_, e)| e.clone())
            .collect();
        entries.sort_by_key(|e| e.id);
        Ok(entries)
    }
    fn put(&self, branch: &str, entry: &crate::domain::mock_history::Entry) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, e)| !(b == branch && e.id == entry.id));
        kept.push((branch.to_string(), entry.clone()));
        Ok(())
    }
    fn remove(&self, branch: &str, id: u64) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, e)| !(b == branch && e.id == id));
        Ok(())
    }
}

/// The request pins' steps, in memory.
#[derive(Default)]
struct Steps(std::sync::Mutex<Vec<(String, crate::domain::steps::Step)>>);

// A test fake: each method holds its lock for the whole of its few lines.
#[allow(clippy::significant_drop_tightening)]
impl crate::ports::StepStore for Steps {
    fn steps(&self, branch: &str) -> Result<Vec<crate::domain::steps::Step>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        let mut steps: Vec<_> = kept
            .iter()
            .filter(|(b, _)| b == branch)
            .map(|(_, s)| s.clone())
            .collect();
        steps.sort_by_key(crate::domain::steps::Step::last);
        Ok(steps)
    }
    fn put(&self, branch: &str, step: &crate::domain::steps::Step) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, s)| !(b == branch && s.pin == step.pin));
        kept.push((branch.to_string(), step.clone()));
        Ok(())
    }
    fn remove(&self, branch: &str, pin: &str) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, s)| !(b == branch && s.pin == pin));
        Ok(())
    }
}

#[derive(Default)]
struct Versions(std::sync::Mutex<Vec<(String, Draft)>>);

// A test fake: each method holds its lock for the whole of its few lines.
#[allow(clippy::significant_drop_tightening)]
impl DraftStore for Versions {
    fn boards(&self) -> Result<Vec<String>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        let mut boards: Vec<String> = kept.iter().map(|(b, _)| b.clone()).collect();
        boards.sort();
        boards.dedup();
        Ok(boards)
    }
    fn versions(&self, board: &str) -> Result<Vec<Draft>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        Ok(kept
            .iter()
            .filter(|(b, _)| b == board)
            .map(|(_, d)| d.clone())
            .collect())
    }
    fn put(&self, board: &str, draft: &Draft) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, d)| !(b == board && d.version == draft.version));
        kept.push((board.to_string(), draft.clone()));
        Ok(())
    }
    fn destroy(&self, board: &str, version: u32) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, d)| !(b == board && d.version == version));
        Ok(())
    }
}

#[tokio::test]
async fn a_draft_opens_writes_keeps_restores_and_validates() {
    let versions = Arc::new(Versions::default());
    let with = || {
        let mut st = state(Shelf::default(), None);
        st.drafts = Arc::clone(&versions) as Arc<dyn DraftStore>;
        st
    };
    let board = "mockup--main--home";
    let (status, body) = post(
        with(),
        &format!("/api/drafts/{board}/open"),
        r#"{"base":"abc"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"version\":1") && body.contains("\"state\":\"open\""),
        "{body}"
    );
    let (status, _) = post(
        with(),
        &format!("/api/drafts/{board}/open"),
        r#"{"base":"abc"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "opening again continues");
    assert_eq!(versions.versions(board).expect("v").len(), 1);
    let ops = r#"{"ops":[{"kind":"remove","by":"human","target":"id:x"}],"viewport":1280,"name":"first"}"#;
    let response = router(with())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/drafts/{board}"))
                .header("content-type", "application/json")
                .body(Body::from(ops))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let (status, body) = post(with(), &format!("/api/drafts/{board}/keep"), "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"current\":null") && body.contains("\"kept\""),
        "{body}"
    );
    let (status, body) = post(
        with(),
        &format!("/api/drafts/{board}/open"),
        r#"{"base":"abc"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"version\":2") && body.contains("\"kind\":\"remove\""),
        "carried over: {body}"
    );
    let (status, _) = post(
        with(),
        &format!("/api/drafts/{board}/validate?version=1"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(with(), &format!("/api/drafts/{board}/restore/1"), "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"version\":3"), "{body}");
    let (status, body) = get(with(), "/api/drafts").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"validated\":[1]"), "{body}");
    let (status, _) = post(
        with(),
        "/api/mockup/write",
        r#"{"branch":"main","path":"../x","text":""}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no product here");
}

async fn post(state: AppState, uri: &str, body: &str) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_easel_is_empty_without_a_product() {
    let (status, body) = get(state(Shelf::default(), None), "/api/mockup").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"enabled\":false"), "{body}");
    let (status, _) = get(state(Shelf::default(), None), "/mock/main/pages/home.html").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_page_reads_the_product_s_graph_and_manifests_and_nothing_else() {
    let (status, _) = get(
        state(Shelf::default(), None),
        "/mock/main/__product/plan/graph.json",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no product, nothing read");
    let with = || {
        let mut st = state(Shelf::default(), None);
        st.mockups = Some(Arc::new(FakeMockups));
        st
    };
    let (status, body) = get(with(), "/mock/main/__product/plan/graph.json").await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, r#"{"ids":{}}"#));
    let manifest =
        "/mock/milestone__1/__product/crates/table/data-capabilities/move/data-capability.json";
    let (status, body) = get(with(), manifest).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("table.moveToken"), "{body}");
    let (status, body) = get(with(), "/mock/main/__product/data-capabilities.json").await;
    assert_eq!(status, StatusCode::OK);
    let index: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        index["manifests"]["table.moveToken"]["path"],
        "crates/table/data-capabilities/move/data-capability.json",
        "{body}"
    );
    assert_eq!(
        index["manifests"]["table.moveToken"]["manifest"]["payload"]["token"],
        "uuid"
    );
    for refused in [
        "/mock/main/__product/Cargo.toml",
        "/mock/main/__product/crates/table/data-capabilities/none/data-capability.json",
        "/mock/main/__product/crates/table/data-capabilities/move/../../../../Cargo.toml",
        "/mock/a..b/__product/plan/graph.json",
    ] {
        let (status, body) = get(with(), refused).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{refused}: {body}");
    }
}

#[tokio::test]
async fn the_easel_serves_the_branch_s_pages_previews_and_saves() {
    let with = || {
        let mut st = state(Shelf::default(), None);
        st.mockups = Some(Arc::new(FakeMockups));
        st
    };
    let (status, body) = get(with(), "/api/mockup?branch=main").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"present\":true") && body.contains("pages/home.html"),
        "{body}"
    );
    assert!(body.contains("\"units\""), "{body}");
    let (status, body) = get(with(), "/mock/main/pages/home.html").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<h1>hi</h1>");
    let (status, _) = get(with(), "/mock/main/pages/none.html").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(with(), "/mock/a..b/pages/home.html").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no branch is named with ..");
    let (status, body) = get(with(), "/mock/milestone__3-x/__preview/RiskBadge").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"data-component="RiskBadge""#) && body.contains("../mock.js"));
    let (status, body) = get(with(), "/mock/design-system/__showcase/contact").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"data-kit="Table""#) && body.contains("../mock.js"));
    let (status, _) = get(with(), "/mock/a..b/__showcase/contact").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(with(), "/mock/main/__preview/risk-badge").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a micro-frontend is named like a Micro-UI"
    );
    let (status, body) = post(
        with(),
        "/api/mockup/save",
        r#"{"branch":"main","name":"home"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("mockup/home"), "{body}");
    let (status, _) = post(with(), "/api/mockup/discard", r#"{"branch":"main"}"#).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(with(), "/api/mockup/restore", r#"{"branch":"main"}"#).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, r#"{"restored":true}"#)
    );
    let (status, body) = get(with(), "/api/mockup/files?branch=main").await;
    assert_eq!(status, StatusCode::OK);
    let files: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        files["files"][0],
        serde_json::json!({"path": "fixtures/a.json", "kind": "fixture", "at": 5})
    );
    assert_eq!(files["files"][1]["kind"], "page");
}

#[derive(Default)]
struct Pins(std::sync::Mutex<Vec<(String, crate::ports::KeptNote)>>);

// A test fake: each method holds its lock for the whole of its few lines.
#[allow(clippy::significant_drop_tightening)]
impl NoteStore for Pins {
    fn boards(&self) -> Result<Vec<String>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        let mut boards: Vec<String> = kept
            .iter()
            .filter(|(_, k)| k.deleted_at.is_none())
            .map(|(b, _)| b.clone())
            .collect();
        boards.sort();
        boards.dedup();
        Ok(boards)
    }
    fn notes(&self, board: &str) -> Result<Vec<crate::ports::KeptNote>, String> {
        let kept = self.0.lock().map_err(|e| e.to_string())?;
        Ok(kept
            .iter()
            .filter(|(b, _)| b == board)
            .map(|(_, k)| k.clone())
            .collect())
    }
    fn put(&self, board: &str, note: &annotations::Note) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        match kept
            .iter_mut()
            .find(|(b, k)| b == board && k.note.id == note.id)
        {
            Some((_, k)) => k.note = note.clone(),
            None => kept.push((
                board.to_string(),
                crate::ports::KeptNote {
                    note: note.clone(),
                    deleted_at: None,
                },
            )),
        }
        Ok(())
    }
    fn mark_deleted(&self, board: &str, id: &str, at: Option<&str>) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        if let Some((_, k)) = kept.iter_mut().find(|(b, k)| b == board && k.note.id == id) {
            k.deleted_at = at.map(str::to_string);
        }
        Ok(())
    }
    fn destroy(&self, board: &str, id: &str) -> Result<(), String> {
        let mut kept = self.0.lock().map_err(|e| e.to_string())?;
        kept.retain(|(b, k)| !(b == board && k.note.id == id));
        Ok(())
    }
    fn conversation(&self, _board: &str, _id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn save_conversation(
        &self,
        _board: &str,
        _id: &str,
        _json: &str,
        _turns: usize,
    ) -> Result<(), String> {
        Ok(())
    }
}

async fn send(state: AppState, method: &str, uri: &str, body: &str) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_pin_is_kept_deleted_brought_back_and_destroyed_past_ten() {
    let state = state(Shelf::default(), None);
    let pin = |id: &str, kind: &str| {
        format!(
            r#"{{"id":"{id}","target":"table:campaign","dx":3,"dy":4,"x":0,"y":0,"text":"why uuid?","at":"t","kind":"{kind}"}}"#
        )
    };
    let (status, _) = send(
        state.clone(),
        "PUT",
        "/api/notes/data-model/p1",
        &pin("p1", "brainstorm"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        state.clone(),
        "PUT",
        "/api/notes/data-model/p2",
        &pin("p1", "note"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the path names the pin");
    let (_, back) = send(state.clone(), "GET", "/api/notes/data-model", "").await;
    assert!(back.contains(r#""kind":"brainstorm""#), "{back}");

    let (status, _) = send(state.clone(), "DELETE", "/api/notes/data-model/p1", "").await;
    assert_eq!(status, StatusCode::OK);
    let (_, back) = send(state.clone(), "GET", "/api/notes/data-model", "").await;
    assert_eq!(back, "[]");
    let (status, restored) = send(state.clone(), "POST", "/api/notes/data-model/undo", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(restored.contains("why uuid?"));
    let (status, _) = send(state.clone(), "POST", "/api/notes/data-model/undo", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nothing left to bring back");

    // A pin destroyed at once (a placement taken back) is gone for good: no
    // undo brings it back.
    let (status, body) = send(
        state.clone(),
        "DELETE",
        "/api/notes/data-model/p1?forever=1",
        "",
    )
    .await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, r#"{"destroyed":"p1"}"#)
    );
    let (_, back) = send(state.clone(), "GET", "/api/notes/data-model", "").await;
    assert!(!back.contains("why uuid?"), "{back}");
    let (status, _) = send(state.clone(), "POST", "/api/notes/data-model/undo", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "destroyed, not deleted");

    for k in 0..12 {
        let id = format!("q{k:02}");
        send(
            state.clone(),
            "PUT",
            &format!("/api/notes/data-model/{id}"),
            &pin(&id, "note"),
        )
        .await;
        send(
            state.clone(),
            "DELETE",
            &format!("/api/notes/data-model/{id}"),
            "",
        )
        .await;
    }
    let kept = state.pins.notes("data-model").expect("read");
    assert_eq!(
        kept.iter().filter(|k| k.deleted_at.is_some()).count(),
        annotations::UNDO_DEPTH,
        "only the last ten deleted pins can come back"
    );
    let (status, _) = send(
        state.clone(),
        "POST",
        "/api/notes/data-model/p1/session",
        "{}",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no sessions without terminals"
    );
    let (status, _) = send(state, "GET", "/api/notes/..%2Fetc", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_data_model_says_when_nothing_reads_it() {
    let (_, body) = send(state(Shelf::default(), None), "GET", "/api/data-model", "").await;
    assert_eq!(
        body,
        r#"{"enabled":false,"model":null,"branch":"","dir":"","error":null}"#
    );
    let (_, body) = send(
        state(Shelf::default(), None),
        "GET",
        "/api/data-model/versions",
        "",
    )
    .await;
    assert_eq!(body, r#"{"enabled":false,"archive":false,"branches":[]}"#);
}

async fn post_to(state: AppState, uri: &str) -> (StatusCode, String) {
    post_body(state, uri, "").await
}

async fn post_body(state: AppState, uri: &str, body: &str) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn watch(pid: u32) -> Vec<Process> {
    vec![Process {
        pid,
        parent: 1,
        group: pid,
        command: "./target/debug/harness watch".to_string(),
    }]
}

fn with_plant(processes: Vec<Process>) -> (AppState, Arc<Switch>) {
    let switch = Arc::new(Switch {
        processes,
        ..Switch::default()
    });
    let mut state = state(Shelf::default(), None);
    state.plant = Some(Arc::clone(&switch) as Arc<dyn Plant>);
    (state, switch)
}

fn sent(switch: &Switch) -> Vec<String> {
    switch.sent.lock().expect("sent").clone()
}

#[tokio::test]
async fn the_switch_starts_an_empty_plant_and_stops_a_running_one() {
    let (state, switch) = with_plant(Vec::new());
    let (status, _) = post_to(state.clone(), "/api/plant/soft").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, body) = post_to(state.clone(), "/api/plant/start").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = post_to(state, "/api/plant/soft").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sent(&switch), ["start", "Term Process(4242)"]);

    let (state, switch) = with_plant(watch(7));
    let (status, _) = post_to(state.clone(), "/api/plant/start").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        post_to(state.clone(), "/api/plant/soft").await.0,
        StatusCode::OK
    );
    assert_eq!(
        post_to(state.clone(), "/api/plant/hard").await.0,
        StatusCode::OK
    );
    assert_eq!(
        post_to(state, "/api/plant/explode").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sent(&switch),
        ["Term Process(7)", "Kill Process(7)", "Kill Group(7)"]
    );
}

#[tokio::test]
async fn a_start_carries_the_agents_asked_within_bounds() {
    let (state, switch) = with_plant(Vec::new());
    let (status, _) = post_to(state.clone(), "/api/plant/start?lanes=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = post_to(state, "/api/plant/start?lanes=4").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("at most 4 agents"), "{body}");
    assert_eq!(sent(&switch), ["start 4"]);
}

#[tokio::test]
async fn a_watch_that_dies_at_start_is_a_failed_start_with_its_words() {
    let switch = Arc::new(Switch {
        dies_at_start: true,
        ..Switch::default()
    });
    let mut state = state(Shelf::default(), None);
    state.plant = Some(Arc::clone(&switch) as Arc<dyn Plant>);
    let (status, body) = post_to(state, "/api/plant/start").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("exited as soon as it started"), "{body}");
    assert!(body.contains("no git repository"), "{body}");
}

/// Counts what it was asked, and answers the same every time.
#[derive(Default)]
struct Gauges {
    probes: std::sync::atomic::AtomicU32,
}

impl Limits for Gauges {
    fn claude(&self) -> Result<Reading, String> {
        self.probes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Reading {
            windows: Vec::new(),
            at: 1,
        })
    }

    fn github(&self) -> Result<Vec<crate::ports::GithubWindow>, String> {
        Err("gh: not logged in".to_string())
    }
}

#[tokio::test]
async fn an_opened_panel_reads_both_limits_and_pays_claude_once() {
    let gauges = Arc::new(Gauges::default());
    let mut state = state(Shelf::default(), None);
    state.limits = Some(Arc::clone(&gauges) as Arc<dyn Limits>);
    let (status, body) = get(state.clone(), "/api/limits").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"claude\":{\"value\":{"), "{body}");
    assert!(body.contains("gh: not logged in"), "{body}");
    let _ = get(state.clone(), "/api/limits").await;
    let _ = get(state, "/api/limits?force=1").await;
    assert_eq!(
        gauges.probes.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a fresh reading is reused, even when asked again at once"
    );
}

#[tokio::test]
async fn a_period_is_read_from_the_traces_and_a_bad_one_refused() {
    let state = state(Shelf::default(), None);
    let (status, body) = get(
        state.clone(),
        "/api/history?from=2026-10-08T20:00:00Z&to=2026-10-08T21:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"from\":\"2026-10-08T20:00:00Z\""), "{body}");
    let (status, _) = get(state.clone(), "/api/history?from=yesterday").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = get(
        state,
        "/api/history?from=2026-10-08T21:00:00Z&to=2026-10-08T20:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_page_reads_whether_a_watch_runs() {
    let (state, _) = with_plant(watch(7));
    let (status, body) = get(state, "/api/plant").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"available\":true"), "{body}");
    assert!(body.contains("\"pid\":7"), "{body}");
    assert!(body.contains("\"max_lanes\":16"), "{body}");
    let (_, none) = get(state_without_plant(), "/api/plant").await;
    assert!(none.contains("\"available\":false"), "{none}");
}

fn state_without_plant() -> AppState {
    state(Shelf::default(), None)
}

async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The event stream's text until every one of `until` shows in it, in any
/// order: the stream merges two sources.
async fn heard_until(body: &mut axum::body::BodyDataStream, until: &[&str]) -> String {
    let mut heard = String::new();
    while !until.iter().all(|u| heard.contains(u)) {
        let chunk = tokio::time::timeout(Duration::from_secs(2), body.next())
            .await
            .unwrap_or_else(|_| panic!("not all of {until:?} in time; heard {heard:?}"))
            .expect("the stream goes on")
            .expect("bytes");
        heard.push_str(&String::from_utf8_lossy(&chunk));
    }
    heard
}

#[tokio::test]
async fn the_event_stream_carries_the_pulse_its_count_first_then_each_beat() {
    let st = state(Shelf::default(), None);
    let pulse = st.pulse.clone();
    pulse.beat();
    pulse.beat();
    let response = router(st)
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    heard_until(&mut body, &["event: snapshot\n", "event: pulse\ndata: 2\n"]).await;
    pulse.beat();
    heard_until(&mut body, &["event: pulse\ndata: 3\n"]).await;
}

#[tokio::test]
async fn the_page_and_the_picture_are_served() {
    let (status, body) = get(state(Shelf::default(), None), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<canvas"), "{body}");
    assert!(body.contains("/vendor/xterm.js"), "{body}");
    assert!(!body.contains("iso.js"), "the canvas engine is gone");
    let (status, body) = get(state(Shelf::default(), None), "/render/render.js").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no bundle in the test checkout"
    );
    assert!(body.contains("build-render"), "{body}");
    let (status, _) = get(state(Shelf::default(), None), "/render/..%2Fapp.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "only the two bundle files");
    let (status, body) = get(state(Shelf::default(), None), "/api/snapshot").await;
    assert_eq!(status, StatusCode::OK);
    let snap: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(snap["rooms"].as_array().map(Vec::len), Some(6));
    assert_eq!(snap["lines"][0]["id"], "agent-loop");
    let (status, body) = get(state(Shelf::default(), None), "/vendor/xterm.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.len() > 100_000, "xterm.js is embedded whole");
}

#[tokio::test]
async fn a_run_s_sessions_come_back_one_per_stage() {
    let mut shelf = Shelf::default();
    shelf.put(
        "agent-loop",
        "20261008-152126-64",
        "run.log",
        "[2026-10-08T15:21:28Z] run 1\n[2026-10-08T15:21:29Z] [code] session opens\n",
    );
    shelf.put(
        "agent-loop",
        "20261008-152126-64",
        "session.log",
        "── turn 1 /code ──\nbuilt it\n",
    );
    let (status, body) = get(
        state(shelf, None),
        "/api/runs/agent-loop/20261008-152126-64/stages",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"stage\":\"code\""), "{body}");
    assert!(body.contains("built it"), "{body}");
    assert!(
        body.contains("\"started_at\":\"2026-10-08T15:21:28Z\""),
        "{body}"
    );
    assert!(
        body.contains("\"opened_at\":\"2026-10-08T15:21:29Z\""),
        "{body}"
    );
    assert!(body.contains("\"closed_at\":null"), "{body}");
    let (status, _) = get(
        state(Shelf::default(), None),
        "/api/runs/agent-loop/..%2Fx/stages",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_tail_is_bounded_to_the_run_files_and_nothing_above_them() {
    let mut shelf = Shelf::default();
    shelf.put(
        "agent-loop",
        "20261006-202608",
        "session.log",
        "line one\nline two\n",
    );
    let (status, body) = get(
        state(shelf, None),
        "/api/runs/agent-loop/20261006-202608/session.log?bytes=100",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "line one\nline two\n");
    for bad in [
        "/api/runs/agent-loop/20261006-202608/costs.tsv",
        "/api/runs/agent-loop/..%2F..%2Fcosts.tsv/run.log",
        "/api/runs/Agent/20261006-202608/run.log",
        "/api/runs/agent-loop/20261006-999999/run.log",
    ] {
        let (status, _) = get(state(Shelf::default(), None), bad).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
    }
}

#[tokio::test]
async fn an_issue_before_the_board_was_read_is_not_found_rather_than_empty() {
    let (status, body) = get(state(Shelf::default(), None), "/api/issues/62").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("board"));
}

#[tokio::test]
async fn the_steward_status_says_whether_there_is_one_and_whether_they_sit() {
    let (status, body) = get(state(Shelf::default(), None), "/api/steward").await;
    assert_eq!(status, StatusCode::OK);
    let none: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(none["available"], false);

    let desk = Desk::new(Arc::new(Echoing::default()));
    let (status, body) = get(state(Shelf::default(), Some(desk.clone())), "/api/steward").await;
    assert_eq!(status, StatusCode::OK);
    let some: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(some["available"], true);
    assert_eq!(some["live"], false, "nobody sits before the first visit");
    assert_eq!(some["command"], "echo");

    // A plain GET is not a WebSocket handshake: the route exists and
    // refuses it before any desk is consulted.
    let (status, _) = get(state(Shelf::default(), None), "/api/steward/term").await;
    assert!(status.is_client_error(), "{status}");
}

#[tokio::test]
async fn the_janitor_weighs_sweeps_and_takes_a_new_limit_from_the_page() {
    use crate::domain::cleanup::fake::Lot;
    use crate::ports::{Heap, Survey, Yard};
    let (status, body) = get(state_without_plant(), "/api/janitor").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"available\":false"), "{body}");
    let (status, _) = post_to(state_without_plant(), "/api/janitor/sweep").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let lot = Arc::new(Lot {
        survey: Survey {
            total: 300,
            folders: vec![Heap {
                rel: "init".to_string(),
                bytes: 300,
                idle_secs: 3 * 86_400,
                dir: true,
            }],
            ..Survey::default()
        },
        ..Lot::default()
    });
    let mut state = state_without_plant();
    state.janitor = Some(Janitor::new(Arc::clone(&lot) as Arc<dyn Yard>));
    let (status, body) = post_to(state.clone(), "/api/janitor/diagnose").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"freeable\":300"), "{body}");
    assert!(
        lot.removed.lock().expect("lock").is_empty(),
        "a diagnosis removes nothing"
    );
    let (status, body) = post_to(state.clone(), "/api/janitor/sweep").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"freed\":300"), "{body}");
    assert_eq!(*lot.removed.lock().expect("lock"), ["init"]);

    let (status, why) = post_body(
        state.clone(),
        "/api/janitor/settings",
        r#"{"limit_bytes": 1000}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
    let twenty = 20_u64 * 1024 * 1024 * 1024;
    let (status, body) = post_body(
        state,
        "/api/janitor/settings",
        &format!(r#"{{"limit_bytes": {twenty}, "auto_sweep": true, "threshold_percent": 90}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains(&format!("\"limit_bytes\":{twenty}")),
        "{body}"
    );
    assert!(
        lot.read_settings()
            .is_some_and(|s| s.contains("\"auto_sweep\": true"))
    );
}

#[tokio::test]
async fn the_doctor_has_a_desk_of_their_own() {
    let (status, body) = get(state(Shelf::default(), None), "/api/doctor").await;
    assert_eq!(status, StatusCode::OK);
    let none: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(none["available"], false);

    let mut with = state(Shelf::default(), None);
    with.doctor = Some(Desk::new(Arc::new(Echoing::default())));
    let (_, body) = get(with.clone(), "/api/doctor").await;
    let some: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(some["available"], true, "the doctor is in");
    let (_, body) = get(with, "/api/steward").await;
    let steward: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        steward["available"], false,
        "but the steward's desk is another"
    );

    let (status, _) = get(state(Shelf::default(), None), "/api/doctor/term").await;
    assert!(status.is_client_error(), "{status}");
}

#[tokio::test]
async fn a_diagnosis_is_typed_to_the_doctor_and_heard_when_the_mark_comes_back() {
    let mut shelf = Shelf::default();
    shelf.put(
        "agent-loop",
        "20261007-142103",
        "run.log",
        "[12:00:01] FAILED: cargo test broke\n",
    );
    let mut with = state(shelf, None);
    let desk = Desk::new(Arc::new(Echoing::default()));
    with.doctor = Some(desk.clone());
    // No doctor, no diagnosis.
    let (status, _) = post_to(
        state(Shelf::default(), None),
        "/api/doctor/diagnose/agent-loop/20261007-142103",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An unknown run is refused before anything is typed.
    let (status, _) = post_to(
        with.clone(),
        "/api/doctor/diagnose/agent-loop/20261007-000000",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!desk.is_live(), "nothing summoned the doctor");

    // The desk is sat at first, so the handler does not wait for a warm-up.
    desk.summon(80, 24).expect("summoned");
    let (status, body) = post_to(
        with.clone(),
        "/api/doctor/diagnose/agent-loop/20261007-142103",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let asked: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(asked["state"], "running");
    let (_, body) = get(with.clone(), "/api/doctor/diagnoses").await;
    assert!(
        body.contains(r#""agent-loop/20261007-142103":{"state":"running""#),
        "{body}"
    );

    // The echoing terminal repeats the question: that is not the answer.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (_, body) = get(with.clone(), "/api/doctor/diagnoses").await;
    assert!(body.contains(r#""state":"running""#), "{body}");

    // A reply that ends with the mark is.
    desk.write(b"...\r\nDIAGNOSIS: 20261007-142103 \xc2\xb7 amber\r\n")
        .expect("written");
    let mut done = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let (_, body) = get(with.clone(), "/api/doctor/diagnoses").await;
        if body.contains(r#""state":"done""#) {
            done = true;
            break;
        }
    }
    assert!(done, "the mark was heard");
    desk.shutdown();
}

#[test]
fn a_workflow_segment_is_a_log_folder_name() {
    assert!(is_workflow("agent-loop"));
    assert!(is_workflow("pr-fix"));
    assert!(!is_workflow(""));
    assert!(!is_workflow("../logs"));
    assert!(!is_workflow("Agent"));
}

#[test]
fn the_page_controls_are_size_and_restart() {
    let size: Control = serde_json::from_str(r#"{"t":"size","cols":100,"rows":40}"#).expect("size");
    assert!(matches!(
        size,
        Control::Size {
            cols: 100,
            rows: 40
        }
    ));
    let restart: Control = serde_json::from_str(r#"{"t":"restart"}"#).expect("restart");
    assert!(matches!(restart, Control::Restart));
    assert!(serde_json::from_str::<Control>(r#"{"t":"nuke"}"#).is_err());
}

/// A register in memory: the factories it was given, no more, and the
/// creations kept beside them.
#[derive(Default)]
struct Roll {
    known: Vec<Factory>,
    enrolled: std::sync::Mutex<Vec<Factory>>,
    creations: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    forgotten: std::sync::Mutex<Vec<String>>,
}

impl Roll {
    fn of(known: Vec<Factory>) -> Self {
        Self {
            known,
            ..Self::default()
        }
    }
}

impl Factories for Roll {
    fn list(&self) -> harness_core::domain::Outcome<Vec<Factory>> {
        let enrolled = self.enrolled.lock().expect("held").clone();
        let gone = self.forgotten.lock().expect("held").clone();
        Ok(self
            .known
            .iter()
            .cloned()
            .chain(enrolled)
            .filter(|f| !gone.contains(&f.id))
            .collect())
    }

    fn get(&self, id: &str) -> harness_core::domain::Outcome<Option<Factory>> {
        let gone = self
            .forgotten
            .lock()
            .expect("held")
            .contains(&id.to_string());
        Ok(self.known.iter().find(|f| f.id == id && !gone).cloned())
    }

    fn enroll(
        &self,
        root: &std::path::Path,
        name: &str,
        slug: &str,
        integration_branch: &str,
    ) -> harness_core::domain::Outcome<Factory> {
        let factory = Factory {
            id: harness_core::ports::store::factories::id_from(name),
            name: name.to_string(),
            slug: slug.to_string(),
            root: root.to_path_buf(),
            integration_branch: integration_branch.to_string(),
        };
        self.enrolled.lock().expect("held").push(factory.clone());
        Ok(factory)
    }

    fn creation(&self, id: &str) -> harness_core::domain::Outcome<Option<String>> {
        Ok(self.creations.lock().expect("held").get(id).cloned())
    }

    fn keep_creation(&self, id: &str, record: &str) -> harness_core::domain::Outcome<()> {
        self.creations
            .lock()
            .expect("held")
            .insert(id.to_string(), record.to_string());
        Ok(())
    }

    fn end_creation(&self, id: &str) -> harness_core::domain::Outcome<()> {
        self.creations.lock().expect("held").remove(id);
        Ok(())
    }

    fn forget(&self, id: &str) -> harness_core::domain::Outcome<()> {
        self.creations.lock().expect("held").remove(id);
        self.forgotten.lock().expect("held").push(id.to_string());
        Ok(())
    }
}

fn factory_in(id: &str, root: &std::path::Path) -> Factory {
    Factory {
        id: id.to_string(),
        name: id.to_string(),
        slug: format!("me/{id}"),
        root: root.to_path_buf(),
        integration_branch: "main_agent".to_string(),
    }
}

fn state_with_register(here: &str, known: Vec<Factory>) -> AppState {
    AppState {
        factories: Some(Arc::new(Roll::of(known))),
        factory: here.to_string(),
        ..state_without_plant()
    }
}

#[tokio::test]
async fn the_factories_are_listed_with_the_one_the_view_is_on() {
    let dir = std::env::temp_dir();
    let known = vec![factory_in("alpha", &dir), factory_in("beta", &dir)];
    let (status, body) = get(state_with_register("beta", known), "/api/factories").await;
    assert_eq!(status, StatusCode::OK);
    let listed: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(listed["current"], "beta");
    assert_eq!(listed["factories"][0]["id"], "alpha");
    assert_eq!(listed["factories"][1]["slug"], "me/beta");
}

#[tokio::test]
async fn without_a_register_there_is_nothing_to_list_nor_to_open() {
    let (status, body) = get(state_without_plant(), "/api/factories").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"factories\":[]"), "{body}");
    let (status, _) = post(state_without_plant(), "/api/factories/alpha/open", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn opening_another_factory_puts_it_where_main_listens() {
    let known = vec![
        factory_in("alpha", &std::env::temp_dir()),
        factory_in("beta", &std::env::temp_dir()),
    ];
    let state = state_with_register("alpha", known);
    let sender = Arc::clone(&state.switching);
    let mut asked = sender.subscribe();
    let (status, body) = post(state, "/api/factories/beta/open", "").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.contains("\"opening\":true"), "{body}");
    assert!(asked.has_changed().expect("sender alive"));
    assert_eq!(
        asked.borrow_and_update().as_ref().map(|f| f.id.clone()),
        Some("beta".to_string())
    );
}

#[tokio::test]
async fn opening_the_factory_already_open_changes_nothing() {
    let known = vec![factory_in("alpha", &std::env::temp_dir())];
    let state = state_with_register("alpha", known);
    let sender = Arc::clone(&state.switching);
    let asked = sender.subscribe();
    let (status, body) = post(state, "/api/factories/alpha/open", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"opening\":false"), "{body}");
    assert!(!asked.has_changed().expect("sender alive"));
}

#[tokio::test]
async fn an_unknown_factory_or_a_folder_gone_is_refused() {
    let known = vec![
        factory_in("alpha", &std::env::temp_dir()),
        factory_in("gone", std::path::Path::new("/nonexistent/factory")),
    ];
    let state = state_with_register("alpha", known);
    let sender = Arc::clone(&state.switching);
    let asked = sender.subscribe();
    let (status, _) = post(state.clone(), "/api/factories/nobody/open", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = post(state, "/api/factories/gone/open", "").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("/nonexistent/factory"), "{body}");
    assert!(!asked.has_changed().expect("sender alive"));
}

/// A teardown that deletes nothing and says what it was asked.
#[derive(Default)]
struct Salvage {
    said: std::sync::Mutex<Vec<String>>,
    missing_right: bool,
}

impl crate::ports::Teardown for Salvage {
    fn delete_right(&self) -> crate::ports::Rights {
        if self.missing_right {
            crate::ports::Rights::Missing
        } else {
            crate::ports::Rights::Granted
        }
    }

    fn wipe_harness_data(&self, root: &std::path::Path) -> Result<(), String> {
        self.said
            .lock()
            .expect("held")
            .push(format!("wipe {}", root.display()));
        Ok(())
    }

    fn delete_repository(&self, slug: &str) -> Result<(), String> {
        self.said.lock().expect("held").push(format!("repo {slug}"));
        Ok(())
    }

    fn remove_folder(&self, root: &std::path::Path) -> Result<(), String> {
        self.said
            .lock()
            .expect("held")
            .push(format!("folder {}", root.display()));
        Ok(())
    }
}

/// A view on `alpha`, which knows `alpha` and `beta` — both under `/f`, the
/// folder the view makes factories in — and can take one down.
fn state_taking_down() -> (AppState, Arc<Salvage>) {
    taking_down(std::path::Path::new("/f/beta"), Salvage::default())
}

/// As `state_taking_down`, with `beta` in `beta_root` and the given teardown.
fn taking_down(beta_root: &std::path::Path, salvage: Salvage) -> (AppState, Arc<Salvage>) {
    let known = vec![
        factory_in("alpha", std::path::Path::new("/f/alpha")),
        factory_in("beta", beta_root),
    ];
    let register = Arc::new(Roll::of(known));
    let salvage = Arc::new(salvage);
    let removal = crate::removal::Removal::new(
        register.clone(),
        salvage.clone(),
        Arc::new(|_| Arc::new(crate::domain::plant::fake::Switch::default()) as Arc<dyn Plant>),
        std::path::PathBuf::from("/f"),
        "alpha",
    );
    let state = AppState {
        factories: Some(register),
        factory: "alpha".to_string(),
        removal: Some(Arc::new(removal)),
        ..state_without_plant()
    };
    (state, salvage)
}

fn ids_of(listing: &str) -> Vec<String> {
    let listed: serde_json::Value = serde_json::from_str(listing).expect("json");
    listed["factories"]
        .as_array()
        .expect("list")
        .iter()
        .map(|f| f["id"].as_str().expect("id").to_string())
        .collect()
}

#[tokio::test]
async fn a_delinked_factory_leaves_the_list_and_its_repository_is_not_touched() {
    let (state, salvage) = state_taking_down();
    let (status, body) = post(
        state.clone(),
        "/api/factories/beta/delink",
        r#"{"confirm":"me/beta"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(report["factory"], "beta");
    assert_eq!(report["repository"], "kept");
    assert_eq!(report["folder"], "removed");
    assert_eq!(report["watch"], "none");
    assert_eq!(*salvage.said.lock().expect("held"), ["folder /f/beta"]);
    let (_, listing) = get(state, "/api/factories").await;
    assert_eq!(ids_of(&listing), ["alpha"]);
}

#[tokio::test]
async fn a_take_down_is_refused_until_the_repository_name_is_typed() {
    let (state, salvage) = state_taking_down();
    for typed in [
        r"{}",
        r#"{"confirm":""}"#,
        r#"{"confirm":"beta"}"#,
        r#"{"confirm":"me/BETA"}"#,
        "",
    ] {
        for kind in ["delink", "destroy"] {
            let path = format!("/api/factories/beta/{kind}");
            let (status, body) = post(state.clone(), &path, typed).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{kind} {typed}: {body}");
        }
    }
    let (status, body) = post(state.clone(), "/api/factories/beta/destroy", r"{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("me/beta"),
        "the page is told what to type: {body}"
    );
    assert!(salvage.said.lock().expect("held").is_empty());
    let (_, listing) = get(state, "/api/factories").await;
    assert_eq!(ids_of(&listing), ["alpha", "beta"]);
}

#[tokio::test]
async fn a_destroyed_factory_loses_its_repository_its_folder_and_its_place() {
    let (state, salvage) = state_taking_down();
    let (status, body) = post(
        state.clone(),
        "/api/factories/beta/destroy",
        r#"{"confirm":"me/beta"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(report["repository"], "deleted");
    assert_eq!(report["folder"], "removed");
    assert_eq!(
        *salvage.said.lock().expect("held"),
        ["repo me/beta", "folder /f/beta"]
    );
    let (_, listing) = get(state, "/api/factories").await;
    assert_eq!(ids_of(&listing), ["alpha"]);
}

#[tokio::test]
async fn the_only_factory_cannot_be_taken_down_from_inside_and_an_unknown_one_is_not_found() {
    // `beta`'s folder is not on the disk: nothing could take the view's place.
    let (state, salvage) = state_taking_down();
    let (status, body) = post(
        state.clone(),
        "/api/factories/alpha/delink",
        r#"{"confirm":"me/alpha"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("create or connect another one"), "{body}");
    let (status, _) = post(
        state.clone(),
        "/api/factories/alpha/destroy",
        r#"{"confirm":"me/alpha"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = post(state.clone(), "/api/factories/nobody/delink", r"{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(salvage.said.lock().expect("held").is_empty());
    let (_, listing) = get(state, "/api/factories").await;
    assert_eq!(ids_of(&listing), ["alpha", "beta"]);
}

#[tokio::test]
async fn taking_down_the_factory_the_view_is_on_moves_the_view_to_another_one() {
    let (state, salvage) = taking_down(&std::env::temp_dir(), Salvage::default());
    let asked = state.switching.subscribe();
    let (status, body) = post(
        state.clone(),
        "/api/factories/alpha/destroy",
        r#"{"confirm":"me/alpha"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let report: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(report["reopening"], "beta");
    assert_eq!(report["repository"], "deleted");
    // The repository goes now; the folder is the next view's to remove.
    assert_eq!(*salvage.said.lock().expect("held"), ["repo me/alpha"]);
    assert_eq!(
        asked.borrow().as_ref().map(|f| f.id.clone()),
        Some("beta".to_string())
    );
    assert_eq!(
        state.removal.as_ref().and_then(|r| r.leaving()),
        Some("remove:/f/alpha".to_string())
    );
    let (_, listing) = get(state, "/api/factories").await;
    assert_eq!(ids_of(&listing), ["beta"]);
}

#[tokio::test]
async fn a_token_without_the_right_to_delete_changes_nothing_and_is_told_what_to_run() {
    let (state, salvage) = taking_down(
        std::path::Path::new("/f/beta"),
        Salvage {
            missing_right: true,
            ..Salvage::default()
        },
    );
    let (status, body) = post(
        state.clone(),
        "/api/factories/beta/destroy",
        r#"{"confirm":"me/beta"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("gh auth refresh -h github.com -s delete_repo"),
        "{body}"
    );
    assert!(body.contains("gh repo delete me/beta --yes"), "{body}");
    assert!(salvage.said.lock().expect("held").is_empty());
    // A delink asks nothing of GitHub, so the missing right does not matter.
    let (status, _) = post(
        state.clone(),
        "/api/factories/beta/delink",
        r#"{"confirm":"me/beta"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn without_a_register_nothing_is_taken_down() {
    let (status, _) = post(state_without_plant(), "/api/factories/alpha/delink", r"{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post(
        state_without_plant(),
        "/api/factories/alpha/destroy",
        r#"{"confirm":"x"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_page_carries_the_factory_menu_and_its_script() {
    let (_, page) = get(state_without_plant(), "/").await;
    assert!(page.contains("id=\"factory-menu\""), "{page}");
    assert!(page.contains("id=\"factory-page\""), "{page}");
    assert!(page.contains("/factories.js"), "{page}");
    let (status, script) = get(state_without_plant(), "/factories.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(script.contains("HarnessFactories"), "{script}");
}

/// A foundry that makes nothing and says what it was asked.
#[derive(Default)]
struct Forge {
    asked: std::sync::Mutex<Vec<String>>,
}

impl crate::ports::Foundry for Forge {
    fn owners(&self) -> Result<Vec<String>, String> {
        Ok(vec!["me".to_string(), "org".to_string()])
    }

    fn create_repository(
        &self,
        owner: &str,
        name: &str,
        private: bool,
        _: &str,
    ) -> Result<String, String> {
        self.asked
            .lock()
            .expect("held")
            .push(format!("create {owner}/{name} private={private}"));
        Ok(format!("{owner}/{name}"))
    }

    fn make_folder(&self, root: &std::path::Path) -> Result<(), String> {
        self.asked
            .lock()
            .expect("held")
            .push(format!("folder {}", root.display()));
        Ok(())
    }

    fn init_repo(
        &self,
        _: &std::path::Path,
        slug: &str,
        branch: &str,
    ) -> Result<crate::ports::Initialized, String> {
        self.asked
            .lock()
            .expect("held")
            .push(format!("init {slug} {branch}"));
        Ok(crate::ports::Initialized {
            ready: true,
            report: "ready".to_string(),
        })
    }
}

/// An AI that answers the same text to every question.
struct Says(String);

impl crate::ports::Oneshot for Says {
    fn ask(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

/// A view on `alpha`, with a creation desk over `roll`, `forge` and an AI
/// answering `says`.
fn state_creating(roll: Arc<Roll>, forge: Arc<Forge>, says: &str) -> AppState {
    let desk = crate::creation_desk::CreationDesk::new(crate::creation_desk::Setup {
        register: roll.clone(),
        factory: "alpha".to_string(),
        foundry: forge,
        factories_dir: std::path::PathBuf::from("/factories"),
        integration_branch: "main_agent".to_string(),
        oneshot: Some(Arc::new(Says(says.to_string()))),
        model: "m".to_string(),
        draw_model: "d".to_string(),
        sessions: 2,
        mockups: None,
        kit: None,
        data: None,
        plant: None,
        pins: None,
    });
    AppState {
        factories: Some(roll),
        creation: Some(desk),
        factory: "alpha".to_string(),
        ..state_without_plant()
    }
}

async fn json_call(state: AppState, method: &str, uri: &str, body: &str) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn being_created(roll: &Roll, record: &crate::domain::creation::Creation) {
    roll.keep_creation("alpha", &serde_json::to_string(record).expect("json"))
        .expect("kept");
}

#[tokio::test]
async fn without_a_register_there_is_no_creation_to_read() {
    let (status, _) = get(state_without_plant(), "/api/creation").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_factory_not_being_created_reads_no_record_and_the_eleven_steps() {
    let state = state_creating(Arc::default(), Arc::default(), "");
    let (status, body) = get(state, "/api/creation").await;
    assert_eq!(status, StatusCode::OK);
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(view["record"].is_null(), "{body}");
    assert_eq!(view["steps"].as_array().map(Vec::len), Some(11));
    assert_eq!(view["branch"], "main_agent");
}

#[tokio::test]
async fn the_accounts_a_repository_may_be_created_under_are_listed() {
    let state = state_creating(Arc::default(), Arc::default(), "");
    let (status, body) = get(state, "/api/creation/owners").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"owners":["me","org"]}"#);
}

#[tokio::test]
async fn a_founding_refuses_a_bad_name_and_founds_a_good_one_to_its_record() {
    let roll = Arc::new(Roll::default());
    let forge = Arc::new(Forge::default());
    let state = state_creating(Arc::clone(&roll), Arc::clone(&forge), "");
    let (status, body) = json_call(
        state.clone(),
        "POST",
        "/api/creation/found",
        r#"{"name":"a b","owner":"me"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = json_call(
        state.clone(),
        "POST",
        "/api/creation/found",
        r#"{"name":"My-App","owner":"me","private":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let desk = state.creation.clone().expect("a desk");
    for _ in 0..200 {
        if desk
            .job()
            .is_some_and(|j| j.state != crate::creation_desk::State::Running)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let job = desk.job().expect("a job");
    assert_eq!(job.state, crate::creation_desk::State::Done, "{}", job.why);
    assert_eq!(job.factory, "my-app");
    assert!(
        !job.steps.iter().any(|s| s.contains("gaps")),
        "a ready repository leaves no gap: {:?}",
        job.steps
    );
    assert_eq!(job.report, "ready");
    assert_eq!(
        *forge.asked.lock().expect("held"),
        [
            "create me/My-App private=true",
            "folder /factories/my-app",
            "init me/My-App main_agent"
        ]
    );
    let record: crate::domain::creation::Creation = serde_json::from_str(
        &roll
            .creation("my-app")
            .expect("read")
            .expect("a record for the new factory"),
    )
    .expect("readable");
    assert_eq!(
        (record.name.as_str(), record.slug.as_str()),
        ("My-App", "me/My-App")
    );
    assert_eq!(record.step, crate::domain::creation::Step::Kind);
    // A second one in the same place is refused before anything runs.
    let (status, body) = json_call(
        state,
        "POST",
        "/api/creation/found",
        r#"{"name":"my-app","owner":"me"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("lives there already"), "{body}");
}

#[tokio::test]
async fn connecting_a_repository_a_factory_drives_already_is_refused() {
    let roll = Arc::new(Roll::of(vec![factory_in("beta", &std::env::temp_dir())]));
    let state = state_creating(roll, Arc::default(), "");
    let (status, body) = json_call(
        state.clone(),
        "POST",
        "/api/creation/connect",
        r#"{"url":"https://github.com/ME/beta"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("drives ME/beta already"), "{body}");
    let (status, _) = json_call(state, "POST", "/api/creation/connect", r#"{"url":"nope"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_step_opens_only_through_its_gate() {
    let roll = Arc::new(Roll::default());
    being_created(
        &roll,
        &crate::domain::creation::Creation::new("alpha", "me/alpha"),
    );
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) =
        json_call(state.clone(), "PUT", "/api/creation", r#"{"step":"brief"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "choose the kind of factory");
    let (status, body) = json_call(
        state.clone(),
        "PUT",
        "/api/creation",
        r#"{"kind":"vertical","brief":"a ledger","step":"brief"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["step"], "brief");
    assert_eq!(view["reachable"], "grilling");
    let (status, _) = json_call(state, "PUT", "/api/creation", r#"{"step":"repository"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_description_is_split_by_the_ai_into_the_brief_parts_the_page_reads() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), r#"{"who":"bakers"}"#);
    let (status, body) = json_call(
        state.clone(),
        "POST",
        "/api/creation/decompose",
        r#"{"description":"a ledger for bakers"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["description"], "a ledger for bakers");
    assert_eq!(view["record"]["brief"], "### Who uses it?\n\nbakers");
    assert_eq!(view["briefParts"][1]["id"], "who");
    assert_eq!(view["reachable"], "grilling");
    let (status, body) = json_call(
        state,
        "POST",
        "/api/creation/decompose",
        r#"{"description":" "}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "describe the application first");
}

#[tokio::test]
async fn a_grilling_round_and_a_scope_are_asked_of_the_ai_and_kept() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    record.brief = "a ledger".to_string();
    being_created(&roll, &record);
    let round = r#"{"questions":[{"title":"Who pays?","recommended":"the client","topic":"Billing"}],"screens":"- **Home** — the ledger"}"#;
    let state = state_creating(Arc::clone(&roll), Arc::default(), round);
    let (status, body) = post(state.clone(), "/api/creation/grill", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["questions"][0]["title"], "Who pays?");
    assert_eq!(view["record"]["questions"][0]["id"], 1);
    assert_eq!(view["record"]["questions"][0]["topic"], "Billing");
    // An open question closes the scope's gate.
    let (status, body) = post(state.clone(), "/api/creation/scope", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "answer every question, or delete it");
    let (status, _) = json_call(
        state.clone(),
        "PUT",
        "/api/creation",
        r#"{"questions":[{"id":1,"answer":"the client"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(state, "/api/creation/scope", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let kept: crate::domain::creation::Creation =
        serde_json::from_str(&roll.creation("alpha").expect("read").expect("kept")).expect("json");
    assert_eq!(
        kept.scope, "## Screens\n\n- **Home** — the ledger",
        "the AI's sections are the scope"
    );
    // The page reads the sections it splits the scope into.
    let (_, body) = get(state_creating(roll, Arc::default(), ""), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["scopeSections"][2]["id"], "screens");
    assert_eq!(view["scopeSections"][2]["required"], true);
    assert_eq!(view["scopeSections"][8]["id"], "notes");
}

#[tokio::test]
async fn the_creation_ends_only_once_the_factory_is_initialized() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/finish", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "initialize the factory first");
    record.issue = "https://github.com/me/alpha/issues/1".to_string();
    being_created(&roll, &record);
    let (status, _) = post(state.clone(), "/api/creation/finish", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(roll.creation("alpha").expect("read"), None);
    let (_, body) = get(state, "/api/creation").await;
    assert!(body.contains("\"record\":null"), "{body}");
}

#[tokio::test]
async fn regenerating_sends_the_creation_back_to_the_brief() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.mocked = Some("two pages".to_string());
    record.step = crate::domain::creation::Step::Rules;
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state, "/api/creation/regenerate", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["step"], "brief");
    assert!(view["record"]["mocked"].is_null());
    assert_eq!(view["record"]["regenerations"], 1);
}

#[tokio::test]
async fn the_page_carries_the_creation_script() {
    let (_, page) = get(state_without_plant(), "/").await;
    assert!(page.contains("/creation.js"), "{page}");
    let (status, script) = get(state_without_plant(), "/creation.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(script.contains("HarnessCreation"), "{script}");
    // The viewport picker the creation page and the mock-up's easel share.
    assert!(page.contains("/viewport.js"), "{page}");
    let (status, script) = get(state_without_plant(), "/viewport.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(script.contains("Viewport"), "{script}");
}

#[tokio::test]
async fn the_preparation_starts_from_the_brief_and_a_gate_refuses_without_one() {
    let roll = Arc::new(Roll::default());
    being_created(
        &roll,
        &crate::domain::creation::Creation::new("alpha", "me/alpha"),
    );
    let state = state_creating(Arc::clone(&roll), Arc::default(), "## Goal\nfirst");
    let (status, body) = post(state.clone(), "/api/creation/prepare", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "choose the kind of factory");

    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    record.brief = "a ledger".to_string();
    being_created(&roll, &record);
    let (status, body) = post(state.clone(), "/api/creation/prepare", "").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let answered: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(answered["job"]["work"], "prepare");
    // One job at a time: the second is refused while the first runs.
    let desk = state.creation.clone().expect("a desk");
    for _ in 0..200 {
        if desk
            .job()
            .is_some_and(|j| j.state != crate::creation_desk::State::Running)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let (status, body) = get(state, "/api/creation").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["job"]["state"], "done", "{body}");
    assert_eq!(
        view["record"]["scope"], "",
        "the scope waits for the grilling"
    );
    assert_eq!(
        view["record"]["step"], "kind",
        "a preparation moves no step"
    );
    assert_eq!(view["scopeStale"], false);
}

#[tokio::test]
async fn the_drawing_ahead_waits_for_the_scope() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    record.brief = "a ledger".to_string();
    record.grilled = true;
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/ahead", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "settle the proof of concept's scope");
    let (_, body) = get(state, "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["mockupStale"], false);
    assert_eq!(view["rulesStale"], false);
    assert!(view["job"].is_null(), "a refused drawing starts nothing");
}

#[tokio::test]
async fn the_micro_frontends_and_the_requirements_wait_for_the_mock_up() {
    let roll = Arc::new(Roll::default());
    being_created(
        &roll,
        &crate::domain::creation::Creation::new("alpha", "me/alpha"),
    );
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/componentize", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "choose the kind of factory");
    let (status, body) = post(state, "/api/creation/requirements", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "choose the kind of factory");
}

#[tokio::test]
async fn the_mockups_and_the_acceptance_wait_for_their_gates() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    record.brief = "b".to_string();
    record.scope = "## Screens\n\n- **Home**".to_string();
    record.questions = vec![crate::domain::creation::Question {
        id: 1,
        title: "t".to_string(),
        answer: "a".to_string(),
        ..crate::domain::creation::Question::default()
    }];
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/mockups", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "validate the design system");
    let (status, body) = post(state, "/api/creation/accept", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "validate the design system");
}

#[tokio::test]
async fn without_a_board_the_rules_status_says_nothing_was_judged() {
    let roll = Arc::new(Roll::default());
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = get(state, "/api/creation/rules/status").await;
    assert_eq!(status, StatusCode::OK);
    let read: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(read["run_url"], "");
    assert_eq!(read["rules"], serde_json::json!({}));
    assert_eq!(read["tasks"], serde_json::json!([]));
}

#[tokio::test]
async fn changed_rules_are_sent_only_once_the_factory_runs_and_they_are_validated() {
    use crate::domain::creation::{Creation, Criterion, Requirement};
    let roll = Arc::new(Roll::default());
    let rule = |id: u32, then: &str| Requirement {
        id,
        title: format!("Rule {id}"),
        criterion: Criterion::from_sentence(&format!("given a debt, when paid, then {then}")),
        ..Requirement::default()
    };
    let mut record = Creation::new("alpha", "me/alpha");
    record.requirements = vec![rule(1, "settled")];
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/rules/send", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "initialize the factory first");
    let (_, body) = get(state.clone(), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        view["rulesMoved"].is_null(),
        "nothing to send before Initialize"
    );

    record.rules_issue = "https://github.com/me/alpha/issues/3".to_string();
    record.rules_sent = vec![rule(1, "settled")];
    record.requirements = vec![rule(1, "closed"), rule(2, "kept")];
    being_created(&roll, &record);
    let (_, body) = get(state.clone(), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["rulesMoved"], serde_json::json!([[1, 2], []]));
    let (status, body) = post(state, "/api/creation/rules/send", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "validate the rules first");
}

#[tokio::test]
async fn the_rules_are_validated_once_complete_and_read_again_behind_their_gate() {
    use crate::domain::creation::{Creation, Requirement};
    let roll = Arc::new(Roll::default());
    let mut record = Creation::new("alpha", "me/alpha");
    record.requirements = vec![Requirement {
        id: 1,
        title: "Pay".to_string(),
        components: vec!["InvoiceList".to_string()],
        ..Requirement::default()
    }];
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/rules/validate", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "R1 has no criterion");
    let (_, body) = get(state.clone(), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["ruleGaps"], serde_json::json!(["R1 has no criterion"]));
    assert_eq!(view["marks"][1], serde_json::json!(["kind", "open"]));
    let (status, body) = post(state.clone(), "/api/creation/rules/1/reread", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "choose the kind of factory");

    record.requirements[0].criterion =
        crate::domain::creation::Criterion::from_sentence("given a debt, when paid, then settled");
    being_created(&roll, &record);
    let (status, body) = post(state, "/api/creation/rules/validate", "").await;
    assert_eq!(status, StatusCode::OK);
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["rules_validated"], true);
    assert_eq!(view["ruleGaps"], serde_json::json!([]));
}

#[tokio::test]
async fn the_guard_waits_for_its_gate_and_a_finding_is_applied_or_dismissed() {
    let roll = Arc::new(Roll::default());
    being_created(
        &roll,
        &crate::domain::creation::Creation::new("alpha", "me/alpha"),
    );
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/audit", "").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "choose the kind of factory")
    );
    for how in ["apply", "dismiss"] {
        let (status, body) = post(state.clone(), &format!("/api/creation/audit/1/{how}"), "").await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::BAD_REQUEST, "no finding 1"),
            "{how}"
        );
    }
    let (status, body) = post(state, "/api/creation/audit/1/burn", "").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "`burn`: apply or dismiss")
    );
}

#[tokio::test]
async fn the_data_step_draws_its_draft_takes_retouches_and_keeps_its_gates() {
    use crate::domain::creation::Creation;
    use crate::domain::data_draft::{Aggregate, DataDraft, Field, FieldRule, Table};
    let roll = Arc::new(Roll::default());
    let mut record = Creation::new("alpha", "me/alpha");
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (_, body) = get(state.clone(), "/api/creation/data-model").await;
    let reply: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(reply["enabled"], true);
    assert!(reply["model"].is_null());
    assert_eq!(reply["error"], "no data model drafted yet");
    let (status, body) = post(state.clone(), "/api/creation/data", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "choose the kind of factory");
    let (status, body) = post(state.clone(), "/api/creation/data/validate", "").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "no data model yet")
    );
    let (status, body) = json_call(state.clone(), "PUT", "/api/creation/data", "{}").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "no data model yet")
    );
    let (status, _) = post(
        state.clone(),
        "/api/creation/data/change",
        r#"{"request": "more"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    record.data = Some(DataDraft {
        tables: vec![Table {
            name: "line".to_string(),
            fields: vec![Field {
                name: "id".to_string(),
                kind: "uuid".to_string(),
                ..Field::default()
            }],
            ..Table::default()
        }],
        aggregates: vec![Aggregate {
            name: "Line".to_string(),
            fields: std::collections::BTreeMap::from([(
                "line.id".to_string(),
                FieldRule::default(),
            )]),
            ..Aggregate::default()
        }],
        from: crate::domain::data_draft::rules_print(&record),
        ..DataDraft::default()
    });
    being_created(&roll, &record);
    let (_, body) = get(state.clone(), "/api/creation/data-model").await;
    let reply: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(reply["error"].is_null());
    assert_eq!(reply["model"]["tables"][0]["name"], "line", "{body}");
    let (_, body) = get(state.clone(), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        view["dataGaps"],
        serde_json::json!(["Line: line.id has no criticality"])
    );
    assert_eq!(view["dataStale"], false);
    let (status, body) = json_call(
        state.clone(),
        "PUT",
        "/api/creation/data",
        r#"{"criticalities": [{"aggregate": "Line", "field": "line.id", "criticality": "critical"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = post(state, "/api/creation/data/validate", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["data"]["validated"], true);
    assert_eq!(view["dataGaps"], serde_json::json!([]));
}

#[tokio::test]
async fn the_ports_step_keeps_its_gate_takes_retouches_and_validates_whole_ports() {
    use crate::domain::creation::Creation;
    use crate::domain::ports_draft::{PortsDraft, database, ports_print};
    let roll = Arc::new(Roll::default());
    let mut record = Creation::new("alpha", "me/alpha");
    being_created(&roll, &record);
    let state = state_creating(Arc::clone(&roll), Arc::default(), "");
    let (status, body) = post(state.clone(), "/api/creation/ports", "").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "choose the kind of factory")
    );
    let (status, body) = json_call(state.clone(), "PUT", "/api/creation/ports", "{}").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "no port drafted yet")
    );
    let (_, body) = get(state.clone(), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        view["portsGaps"].is_null(),
        "nothing listed: no gap to show"
    );

    record.ports = Some(PortsDraft {
        ports: vec![database()],
        from: ports_print(&record),
        validated: false,
    });
    being_created(&roll, &record);
    let mail = r#"{"ports": [{"id": "mail", "purpose": "Receipts", "provider": "", "tech_kind": "rest", "tech_name": "postmark", "secrets": ["POSTMARK_TOKEN"]}]}"#;
    let (status, body) = json_call(state.clone(), "PUT", "/api/creation/ports", mail).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        view["record"]["ports"]["ports"][0]["id"], "database",
        "kept first"
    );
    let (_, body) = get(state.clone(), "/api/creation").await;
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        view["portsGaps"],
        serde_json::json!(["mail has no provider"])
    );
    assert_eq!(view["portsStale"], false);
    let (status, body) = post(state.clone(), "/api/creation/ports/validate", "").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::BAD_REQUEST, "mail has no provider")
    );
    let (status, body) = json_call(
        state.clone(),
        "PUT",
        "/api/creation/ports",
        &mail.replace(r#""provider": """#, r#""provider": "Postmark""#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = post(state, "/api/creation/ports/validate", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["ports"]["validated"], true);
    assert_eq!(view["portsGaps"], serde_json::json!([]));
}

/// A `Usage` kept in memory, for a test to read back.
#[derive(Default)]
struct Spent(std::sync::Mutex<Vec<crate::ports::Spend>>);

impl crate::ports::Usage for Spent {
    fn log(&self, spend: &crate::ports::Spend) -> Result<(), String> {
        self.0.lock().expect("held").push(spend.clone());
        Ok(())
    }

    fn all(&self) -> Result<Vec<crate::ports::Spend>, String> {
        Ok(self.0.lock().expect("held").clone())
    }
}

#[tokio::test]
async fn without_a_store_usage_reads_as_empty() {
    let (status, body) = get(state_without_plant(), "/api/usage").await;
    assert_eq!(status, StatusCode::OK);
    let summary: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(summary["totalUsd"], 0.0);
    assert_eq!(summary["calls"], 0);
}

#[tokio::test]
async fn usage_sums_what_was_logged_by_who() {
    let kept = Spent::default();
    kept.log(&crate::ports::Spend {
        at: "2026-10-12T00:00:00Z".to_string(),
        who: "creation".to_string(),
        model: "m".to_string(),
        mode: "write".to_string(),
        cost_usd: Some(0.5),
        duration_ms: Some(10),
        input_tokens: Some(1),
        output_tokens: Some(2),
    })
    .expect("logged");
    let state = AppState {
        usage: Some(Arc::new(kept)),
        ..state_without_plant()
    };
    let (status, body) = get(state, "/api/usage").await;
    assert_eq!(status, StatusCode::OK);
    let summary: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(summary["totalUsd"], 0.5);
    assert_eq!(summary["calls"], 1);
    assert_eq!(summary["byWho"][0]["who"], "creation");
}

/// A kit model that answers the tokens generation and any component
/// fragment with something minimal but valid.
struct KitModel;

impl crate::ports::Oneshot for KitModel {
    fn ask(&self, _model: &str, prompt: &str) -> Result<String, String> {
        if prompt.contains("design tokens") {
            return Ok(harness_architecture::kit::default_tokens_json());
        }
        let name = prompt.split('`').nth(1).unwrap_or("X");
        Ok(format!(
            "```html\n<div class=\"kit-{}\" style=\"color:var(--color-text, #111)\">{name}</div>\n```",
            name.to_ascii_lowercase()
        ))
    }
}

/// A product folder a `DirMockups` can serve, with a `main` branch so the
/// kit desk's `ensure` finds somewhere to start the design system from.
fn kit_product(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "harness-design-validate-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("main/mock")).expect("dir");
    // `ensure_branch` on the sample never makes `design-system`: it must be there already.
    std::fs::create_dir_all(dir.join("design-system/mock")).expect("dir");
    std::fs::write(
        dir.join("main/mock/pages.json"),
        r#"{"version":1,"pages":[],"components":[]}"#,
    )
    .expect("manifest");
    dir
}

/// Waits for `kit`'s jobs to show at least one, then for the catalogue to
/// carry that it is actually under way — never still at its generic
/// freshly-listed `Pending` (the catalogue's default for a base not even
/// added yet).
async fn settled_kit(kit: &crate::kit_desk::KitDesk, why: &str) {
    for _ in 0..200 {
        let reply = kit.reply().await.expect("read");
        if !reply.jobs.is_empty() {
            assert!(
                reply.catalog.iter().any(|c| c.checked
                    && matches!(
                        c.state,
                        harness_architecture::kit::ComponentState::Running
                            | harness_architecture::kit::ComponentState::Generated
                    )),
                "{why}: {:?}",
                reply.catalog
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{why}: never queued a single job");
}

/// A branch's mock-up with three steps on it — `b` built on `a`, `c` on
/// its own — and the pins `a` and `b` validated on their board.
fn undo_fixture() -> (std::path::PathBuf, AppState, Arc<Steps>, Arc<Pins>) {
    use crate::domain::steps::Step;
    use crate::ports::StepStore;
    let dir = std::env::temp_dir().join(format!("harness-undo-step-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("main/mock/components")).expect("dir");
    let file = |path: &str| dir.join("main/mock").join(path);
    std::fs::write(file("components/X.html"), "b's X").expect("fragment");
    std::fs::write(file("components/X2.html"), "the fork").expect("fork");
    std::fs::write(file("components/Y.html"), "c's Y").expect("other");
    let steps = Arc::new(Steps::default());
    let step = |pin: &str, files: &[(&str, Option<&str>, u64)]| {
        let mut s = Step {
            pin: pin.to_string(),
            board: "mockup--main--home".to_string(),
            ..Step::default()
        };
        for (path, text, seq) in files {
            s.keep(path, text.map(ToString::to_string), *seq);
        }
        s
    };
    StepStore::put(
        steps.as_ref(),
        "main",
        &step(
            "a",
            &[
                ("components/X.html", Some("old X"), 1),
                ("components/X2.html", None, 1),
            ],
        ),
    )
    .expect("a");
    StepStore::put(
        steps.as_ref(),
        "main",
        &step("b", &[("components/X.html", Some("a's X"), 2)]),
    )
    .expect("b");
    StepStore::put(
        steps.as_ref(),
        "main",
        &step("c", &[("components/Y.html", Some("old Y"), 3)]),
    )
    .expect("c");
    let pins = Arc::new(Pins::default());
    for id in ["a", "b"] {
        crate::ports::NoteStore::put(
            pins.as_ref(),
            "mockup--main--home",
            &crate::domain::annotations::Note {
                id: id.to_string(),
                text: "make it bolder".to_string(),
                kind: crate::domain::annotations::Kind::Request,
                state: crate::domain::annotations::RequestState::Validated,
                ..crate::domain::annotations::Note::default()
            },
        )
        .expect("pin");
    }
    let state = AppState {
        mockups: Some(Arc::new(crate::adapters::mockups::DirMockups::new(&dir))),
        steps: Arc::clone(&steps) as Arc<dyn StepStore>,
        pins: Arc::clone(&pins) as Arc<dyn crate::ports::NoteStore>,
        ..state_without_plant()
    };
    (dir, state, steps, pins)
}

#[tokio::test]
async fn a_pin_s_journal_tells_it_placed_written_and_proposed_with_its_conversation() {
    let state = state_without_plant();
    let uri = "/api/notes/mockup--main--home/p1";
    let pin = |text: &str| {
        format!(r#"{{"id":"p1","target":"","x":1,"y":2,"text":"{text}","kind":"request"}}"#)
    };
    for text in ["", "a rail", "a rail on the left"] {
        let (status, _) = send_with(state.clone(), "PUT", uri, &pin(text), &[]).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = post(
        state.clone(),
        "/api/notes/mockup--main--home/p1/journal",
        r#"{"kind":"proposed","said":"1 change on the page"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post(
        state.clone(),
        "/api/notes/mockup--main--home/p1/journal",
        r#"{"kind":"validated","said":"by a page"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a page tells proposals only"
    );
    let (status, body) = send_with(
        state,
        "GET",
        "/api/notes/mockup--main--home/p1/journal",
        "",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let got: serde_json::Value = serde_json::from_str(&body).expect("json");
    let told: Vec<(String, String)> = got["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap_or_default().to_string(),
                e["said"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert_eq!(
        told,
        [
            ("placed".to_string(), String::new()),
            ("words".to_string(), "a rail on the left".to_string()),
            ("proposed".to_string(), "1 change on the page".to_string()),
        ],
        "the words written on in one event"
    );
    assert_eq!(got["turns"], serde_json::json!([]));
}

#[tokio::test]
async fn a_rework_dropped_puts_every_file_back_and_kept_goes_in_the_history_and_the_pin_s_step() {
    let dir = std::env::temp_dir().join(format!("harness-rework-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("main/mock/pages")).expect("dir");
    let file = |path: &str| dir.join("main/mock").join(path);
    std::fs::write(file("pages/a.html"), "<main>before</main>").expect("page");
    let state = AppState {
        mockups: Some(Arc::new(crate::adapters::mockups::DirMockups::new(&dir))),
        ..state_without_plant()
    };
    let (board, pin) = ("mockup--main--a", "p1");

    // Dropped: the page written and a style added are put back.
    super::rework::take(&state, "main", board, pin)
        .await
        .expect("taken");
    std::fs::write(file("pages/a.html"), "<main>after</main>").expect("edited");
    std::fs::write(file("style.css"), ".rail{}").expect("added");
    assert_eq!(
        super::rework::settle(&state, board, pin, false).await,
        Ok(vec!["pages/a.html".to_string(), "style.css".to_string()])
    );
    assert_eq!(
        std::fs::read_to_string(file("pages/a.html")).expect("page"),
        "<main>before</main>"
    );
    assert!(!file("style.css").exists());
    assert_eq!(
        super::rework::settle(&state, board, pin, false).await,
        Ok(Vec::new()),
        "settled once"
    );

    // Kept: in the history, and in the pin's step for its Undo.
    super::rework::take(&state, "main", board, pin)
        .await
        .expect("taken again");
    std::fs::write(file("pages/a.html"), "<main>rail</main>").expect("edited");
    assert_eq!(
        super::rework::settle(&state, board, pin, true)
            .await
            .map(|p| p.len()),
        Ok(1)
    );
    assert_eq!(
        std::fs::read_to_string(file("pages/a.html")).expect("page"),
        "<main>rail</main>"
    );
    let entries = state.history.entries("main").expect("history");
    assert_eq!(
        entries.last().map(|e| e.what.as_str()),
        Some("a request reworked with the AI")
    );
    let steps = state.steps.steps("main").expect("steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(
        (steps[0].pin.as_str(), steps[0].files[0].text.as_deref()),
        (pin, Some("<main>before</main>"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A request to the router with headers of its own.
async fn send_with(
    state: AppState,
    method: &str,
    uri: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = router(state)
        .oneshot(request.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_mock_up_s_history_takes_back_a_pin_and_brings_it_again_unless_it_moved_outside() {
    let state = state_without_plant();
    let pin = |text: &str| {
        format!(r#"{{"id":"p1","target":"","x":1,"y":2,"text":"{text}","kind":"request"}}"#)
    };
    let uri = "/api/notes/mockup--main--home/p1";
    let at_main: &[(&str, &str)] = &[("x-harness-branch", "main")];
    let (status, _) = send_with(state.clone(), "PUT", uri, &pin(""), at_main).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send_with(state.clone(), "PUT", uri, &pin("bolder"), at_main).await;
    assert_eq!(status, StatusCode::OK);
    let (_, told) = send_with(
        state.clone(),
        "GET",
        "/api/mockup/history?branch=main",
        "",
        &[],
    )
    .await;
    let told: serde_json::Value = serde_json::from_str(&told).expect("json");
    assert_eq!(told["undo"]["what"], "a pin's words");
    assert_eq!(
        told["undo"]["boards"],
        serde_json::json!(["mockup--main--home"])
    );
    assert!(told["redo"].is_null());

    let text = |state: &AppState| {
        crate::ports::NoteStore::notes(state.pins.as_ref(), "mockup--main--home")
            .expect("pins")
            .into_iter()
            .find(|k| k.deleted_at.is_none())
            .map(|k| k.note.text)
    };
    let (status, body) = post(
        state.clone(),
        "/api/mockup/history/undo",
        r#"{"branch":"main"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(text(&state).as_deref(), Some(""), "the words taken back");
    let (status, _) = post(
        state.clone(),
        "/api/mockup/history/undo",
        r#"{"branch":"main"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(&state), None, "the pin taken back");
    let (status, _) = post(
        state.clone(),
        "/api/mockup/history/undo",
        r#"{"branch":"main"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "nothing left to undo");
    let (status, _) = post(
        state.clone(),
        "/api/mockup/history/redo",
        r#"{"branch":"main"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(&state).as_deref(), Some(""), "placed again");

    // Moved outside the history (no branch said): a redo would overwrite it.
    let (status, _) = send_with(state.clone(), "PUT", uri, &pin("mine"), &[]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(
        state.clone(),
        "/api/mockup/history/redo",
        r#"{"branch":"main"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("moved since"));
    assert_eq!(text(&state).as_deref(), Some("mine"));
}

#[tokio::test]
async fn a_pin_s_undo_puts_back_the_files_of_its_step_and_of_those_built_on_it() {
    let (dir, state, steps, pins) = undo_fixture();
    let file = |path: &str| dir.join("main/mock").join(path);
    let (status, body) = post(
        state.clone(),
        "/api/mockup/steps/undo",
        r#"{"branch":"main","pin":"a"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let got: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(got["pins"], serde_json::json!(["b", "a"]));
    assert_eq!(
        std::fs::read_to_string(file("components/X.html")).expect("X"),
        "old X"
    );
    assert!(
        !file("components/X2.html").exists(),
        "the fork it made is gone"
    );
    assert_eq!(
        std::fs::read_to_string(file("components/Y.html")).expect("Y"),
        "c's Y"
    );
    let pin_states: Vec<_> = crate::ports::NoteStore::notes(pins.as_ref(), "mockup--main--home")
        .expect("pins")
        .into_iter()
        .map(|k| k.note.state)
        .collect();
    assert_eq!(
        pin_states,
        [crate::domain::annotations::RequestState::Pending; 2],
        "the pins taken back are not proposed any more"
    );
    assert_eq!(
        StepStore::steps(steps.as_ref(), "main")
            .expect("steps")
            .iter()
            .map(|s| s.pin.as_str())
            .collect::<Vec<_>>(),
        ["c"]
    );
    let (status, body) = post(
        state.clone(),
        "/api/mockup/steps/undo",
        r#"{"branch":"main","pin":"nothing kept"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a pin with no step is taken back all the same"
    );
    assert!(body.contains("nothing kept"));
    let (status, _) = post(state, "/api/mockup/steps/undo", r#"{"branch":"main"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no pin");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn generating_tokens_queues_the_kit_on_a_candidate_not_yet_validated() {
    let dir = kit_product("candidate");
    let mockups: Arc<dyn crate::ports::Mockups> =
        Arc::new(crate::adapters::mockups::DirMockups::new(&dir));
    let kit = crate::kit_desk::KitDesk::new(crate::kit_desk::Setup {
        mockups,
        oneshot: Some(Arc::new(KitModel)),
        model: "m".to_string(),
        sessions: 2,
        pins: Arc::new(crate::kit_desk::tests::Shelf::default()),
        integration_branch: "main".to_string(),
    });
    let state = AppState {
        kit: Some(kit.clone()),
        ..state_without_plant()
    };
    let (status, _) = post(
        state,
        "/api/design/generate",
        r#"{"brief":"a plant, teal and cream"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Not validated yet: the simulated page still means something before
    // the human commits to this candidate, not only after.
    assert_eq!(
        kit.reply().await.expect("read").design.design.state,
        harness_architecture::kit::DesignState::Candidate
    );
    settled_kit(
        &kit,
        "generating a fresh candidate queues the kit on its own",
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn validating_queues_the_kit_too() {
    let dir = kit_product("validate");
    let mockups: Arc<dyn crate::ports::Mockups> =
        Arc::new(crate::adapters::mockups::DirMockups::new(&dir));
    let kit = crate::kit_desk::KitDesk::new(crate::kit_desk::Setup {
        mockups,
        oneshot: Some(Arc::new(KitModel)),
        model: "m".to_string(),
        sessions: 2,
        pins: Arc::new(crate::kit_desk::tests::Shelf::default()),
        integration_branch: "main".to_string(),
    });
    // The candidate is injected through the desk directly, never the
    // router: nothing has asked the kit to generate yet, so the route's
    // own queueing is what the rest of this test actually proves.
    kit.inject(harness_architecture::kit::default_tokens_json(), false)
        .await
        .expect("injected");
    assert!(
        kit.reply().await.expect("read").jobs.is_empty(),
        "nothing queued by the direct inject"
    );
    let state = AppState {
        kit: Some(kit.clone()),
        ..state_without_plant()
    };
    let (status, _) = post(state, "/api/design/validate", "").await;
    assert_eq!(status, StatusCode::OK);
    settled_kit(&kit, "validating queues the kit too").await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn injecting_tokens_queues_the_kit_too() {
    let dir = kit_product("inject");
    let mockups: Arc<dyn crate::ports::Mockups> =
        Arc::new(crate::adapters::mockups::DirMockups::new(&dir));
    let kit = crate::kit_desk::KitDesk::new(crate::kit_desk::Setup {
        mockups,
        oneshot: Some(Arc::new(KitModel)),
        model: "m".to_string(),
        sessions: 2,
        pins: Arc::new(crate::kit_desk::tests::Shelf::default()),
        integration_branch: "main".to_string(),
    });
    let state = AppState {
        kit: Some(kit.clone()),
        ..state_without_plant()
    };
    let body = serde_json::to_string(&serde_json::json!({
        "json": harness_architecture::kit::default_tokens_json(),
        "edit": false,
    }))
    .expect("json");
    let (status, response) = post(state, "/api/design/tokens", &body).await;
    assert_eq!(status, StatusCode::OK);
    let view: serde_json::Value = serde_json::from_str(&response).expect("json");
    assert_eq!(view["design"]["state"], "candidate", "{response}");
    settled_kit(&kit, "injecting tokens queues the kit too").await;
    let _ = std::fs::remove_dir_all(&dir);
}

// An interface example: uploaded as an image of its kind, then described
// through its own route; a body that is not one is refused.
#[tokio::test]
async fn an_interface_example_is_uploaded_and_described() {
    let dir = kit_product("example");
    let mockups: Arc<dyn crate::ports::Mockups> =
        Arc::new(crate::adapters::mockups::DirMockups::new(&dir));
    let kit = crate::kit_desk::KitDesk::new(crate::kit_desk::Setup {
        mockups,
        oneshot: None,
        model: "m".to_string(),
        sessions: 1,
        pins: Arc::new(crate::kit_desk::tests::Shelf::default()),
        integration_branch: "main".to_string(),
    });
    let state = AppState {
        kit: Some(kit.clone()),
        ..state_without_plant()
    };
    let (status, response) = post(
        state.clone(),
        "/api/design/asset?kind=example&name=nav.png",
        "png",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(response.contains("design/examples/nav.png"), "{response}");
    let body =
        r#"{"file":"examples/nav.png","note":"the side nav","scope":{"on":"page","name":"Login"}}"#;
    let (status, response) = post(state.clone(), "/api/design/example", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let view: serde_json::Value = serde_json::from_str(&response).expect("json");
    assert_eq!(view["design"]["examples"][0]["note"], "the side nav");
    assert_eq!(view["design"]["examples"][0]["scope"]["name"], "Login");
    let (status, _) = post(state, "/api/design/example", "{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A product with a board in memory: its `ports.json`, the tasks a person
/// holds, and every write the Ports pane makes.
#[derive(Default)]
struct PortsBoard {
    ports: Option<String>,
    human: Vec<harness_core::domain::Issue>,
    wrote: std::sync::Mutex<Vec<String>>,
}

impl PortsBoard {
    fn note(&self, line: String) {
        self.wrote.lock().expect("held").push(line);
    }
}

#[async_trait::async_trait(?Send)]
impl crate::ports::Product for PortsBoard {
    fn origin(&self) -> String {
        "me/alpha".to_string()
    }
    fn live_ref(&self) -> String {
        "main_agent".to_string()
    }
    async fn file(
        &self,
        path: &str,
        git_ref: &str,
    ) -> harness_core::domain::Outcome<Option<String>> {
        assert_eq!((path, git_ref), ("ports.json", "main_agent"));
        Ok(self.ports.clone())
    }
    async fn branches(&self, _: &str) -> harness_core::domain::Outcome<Vec<String>> {
        Ok(Vec::new())
    }
    async fn open_issue(
        &self,
        title: &str,
        _: &str,
        labels: &[&str],
    ) -> harness_core::domain::Outcome<String> {
        self.note(format!("open {title} [{}]", labels.join(",")));
        Ok("https://github.com/me/alpha/issues/30".to_string())
    }
    async fn human_tasks(&self) -> harness_core::domain::Outcome<Vec<harness_core::domain::Issue>> {
        Ok(self.human.clone())
    }
    async fn close_issue(&self, number: u64, comment: &str) -> harness_core::domain::Outcome<()> {
        self.note(format!("close #{number}: {comment}"));
        Ok(())
    }
    async fn to_decide(&self, number: u64, comment: &str) -> harness_core::domain::Outcome<()> {
        self.note(format!("decide #{number}: {comment}"));
        Ok(())
    }
}

/// A mail port on the integration branch, and an llm port asked for #11.
fn ports_board() -> PortsBoard {
    PortsBoard {
        ports: Some(
            r#"{"version": 1, "ports": [{"id": "mail", "purpose": "receipts", "provider": "Postmark",
                "tech": {"kind": "rest", "name": "postmark"}, "adapter": "crates/ports/mail/postmark",
                "fake": "crates/ports/mail", "secrets": ["POSTMARK_TOKEN"], "usedBy": ["billing.send"]}]}"#
                .to_string(),
        ),
        human: vec![harness_core::domain::Issue {
            number: 21,
            title: "Approve the port llm for #11: Summaries".to_string(),
            state: "open".to_string(),
            body: "> llm — Anthropic, per token, summaries".to_string(),
            ..harness_core::domain::Issue::default()
        }],
        ..PortsBoard::default()
    }
}

#[tokio::test]
async fn the_ports_pane_reads_the_ports_and_writes_only_the_human_s_three_gestures() {
    let (status, _) = get(state_without_plant(), "/api/ports").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nothing reads the product");
    let board = std::rc::Rc::new(ports_board());
    let shared = std::rc::Rc::clone(&board);
    tokio::task::LocalSet::new()
        .run_until(async move {
            let (desk, task) = crate::data_desk::open(
                shared as std::rc::Rc<dyn crate::ports::Product>,
                Duration::from_secs(60),
                jiff::tz::TimeZone::UTC,
            );
            let task = tokio::task::spawn_local(task);
            let mut with_board = state_without_plant();
            with_board.data = Some(desk);
            let (status, body) = get(with_board.clone(), "/api/ports").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let read: serde_json::Value = serde_json::from_str(&body).expect("json");
            assert_eq!(
                read["ports"][0]["usedBy"],
                serde_json::json!(["billing.send"])
            );
            assert_eq!(read["approvals"][0]["task"], 11);
            assert_eq!(read["missing"], false);

            let (status, body) = post(
                with_board.clone(),
                "/api/ports/approve",
                r#"{"approval": 9}"#,
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
            let (status, _) = post(with_board.clone(), "/api/ports/approve", "nope").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let (status, body) = post(
                with_board.clone(),
                "/api/ports/approve",
                r#"{"approval": 21}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let (status, body) = post(
                with_board.clone(),
                "/api/ports/refuse",
                r#"{"approval": 21, "why": "too dear"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let (status, body) = post(
                with_board.clone(),
                "/api/ports/change",
                r#"{"port": "mail", "provider": "Postmark"}"#,
            )
            .await;
            assert_eq!(
                (status, body.as_str()),
                (StatusCode::BAD_REQUEST, "Postmark already serves mail")
            );
            let (status, _) = post(
                with_board.clone(),
                "/api/ports/change",
                r#"{"port": "sms", "provider": "X"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            let (status, body) = post(
                with_board,
                "/api/ports/change",
                r#"{"port": "mail", "provider": "Amazon SES", "why": "cost"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(body.contains("issues/30"), "{body}");
            task.abort();
        })
        .await;
    let wrote = board.wrote.lock().expect("held").clone();
    assert_eq!(wrote.len(), 4, "{wrote:?}");
    assert!(
        wrote[0].starts_with("close #21: Approved from the plant's Ports pane"),
        "{wrote:?}"
    );
    assert_eq!(
        wrote[1],
        "decide #11: The port asked — `llm` — is refused: too dear. Decide how this task does without it, or cut it."
    );
    assert!(wrote[2].starts_with("close #21: Refused"), "{wrote:?}");
    assert_eq!(
        wrote[3],
        "open Change the provider of the mail port: Postmark → Amazon SES [harness:roadmap,harness:ready]"
    );
}
