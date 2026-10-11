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
            stamp: "s1".into(),
        })
    }
    fn file(&self, _branch: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        Ok((path == "pages/home.html").then(|| b"<h1>hi</h1>".to_vec()))
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
        Ok(self.known.iter().cloned().chain(enrolled).collect())
    }

    fn get(&self, id: &str) -> harness_core::domain::Outcome<Option<Factory>> {
        Ok(self.known.iter().find(|f| f.id == id).cloned())
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
        mockups: None,
        kit: None,
        data: None,
        plant: None,
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
async fn a_factory_not_being_created_reads_no_record_and_the_nine_steps() {
    let state = state_creating(Arc::default(), Arc::default(), "");
    let (status, body) = get(state, "/api/creation").await;
    assert_eq!(status, StatusCode::OK);
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(view["record"].is_null(), "{body}");
    assert_eq!(view["steps"].as_array().map(Vec::len), Some(9));
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
async fn a_grilling_round_and_a_scope_are_asked_of_the_ai_and_kept() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    record.brief = "a ledger".to_string();
    being_created(&roll, &record);
    let round = r#"{"questions":[{"title":"Who pays?","recommended":"the client"}]}"#;
    let state = state_creating(Arc::clone(&roll), Arc::default(), round);
    let (status, body) = post(state.clone(), "/api/creation/grill", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let view: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(view["record"]["questions"][0]["title"], "Who pays?");
    assert_eq!(view["record"]["questions"][0]["id"], 1);
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
    assert_eq!(kept.scope, round, "the AI's answer is the scope");
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
    record.step = crate::domain::creation::Step::Review;
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
}

#[tokio::test]
async fn the_mockups_and_the_acceptance_wait_for_their_gates() {
    let roll = Arc::new(Roll::default());
    let mut record = crate::domain::creation::Creation::new("alpha", "me/alpha");
    record.kind = Some(crate::domain::creation::Kind::Vertical);
    record.brief = "b".to_string();
    record.scope = "s".to_string();
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
