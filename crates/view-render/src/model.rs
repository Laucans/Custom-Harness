//! The picture, as the renderer reads it.
//!
//! A mirror of the fields of `harness-view`'s `Snapshot` that the scene
//! draws — not the type itself: that crate pulls `tokio` with process
//! features, which has no place in a browser bundle. Every field defaults,
//! so a picture from a newer server still draws what this version knows.

use serde::Deserialize;

/// The project on the sign.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Project {
    /// The name.
    #[serde(default)]
    pub name: String,
    /// `owner/name`, or empty.
    #[serde(default)]
    pub slug: String,
    /// The repository's web page.
    #[serde(default)]
    pub url: String,
}

/// One chimney.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Chimney {
    /// `opus`, `sonnet`, `haiku`.
    #[serde(default)]
    pub model: String,
    /// A session on it is open.
    #[serde(default)]
    pub smoking: bool,
    /// Employees on it.
    #[serde(default)]
    pub runs: u32,
    /// Dollars on its stages.
    #[serde(default)]
    pub usd: f64,
}

/// Seen from outside.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Factory {
    /// One per model.
    #[serde(default)]
    pub chimneys: Vec<Chimney>,
    /// The watch polls.
    #[serde(default)]
    pub watching: bool,
    /// Nobody works.
    #[serde(default = "yes")]
    pub idle: bool,
    /// What the router saw.
    #[serde(default)]
    pub saw: Option<String>,
    /// A dispatch in flight.
    #[serde(default)]
    pub in_flight: Option<InFlight>,
}

const fn yes() -> bool {
    true
}

impl Default for Factory {
    /// A plant nobody has read yet is idle, not at work.
    fn default() -> Self {
        Self {
            chimneys: Vec::new(),
            watching: false,
            idle: true,
            saw: None,
            in_flight: None,
        }
    }
}

/// A dispatch the watch has not reported back on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct InFlight {
    /// The log folder.
    #[serde(default)]
    pub workflow: String,
    /// `milestone 17`.
    #[serde(default)]
    pub subject: Option<String>,
}

/// What a station is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StationState {
    /// Nothing passed.
    #[default]
    Idle,
    /// The product is here.
    Active,
    /// Reported.
    Done,
    /// Skipped.
    Skipped,
}

/// How a station looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A gate.
    #[default]
    Scanner,
    /// An LLM that writes.
    Builder,
    /// An LLM that judges.
    Inspector,
    /// A free context step.
    Printer,
    /// A free step acting on the code.
    Arm,
}

/// One station.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Station {
    /// Unique within its line.
    #[serde(default)]
    pub id: String,
    /// The sign.
    #[serde(default)]
    pub label: String,
    /// The look.
    #[serde(default)]
    pub kind: Kind,
    /// The stage, when it has one.
    #[serde(default)]
    pub stage: Option<String>,
    /// The model it opens.
    #[serde(default)]
    pub model: Option<String>,
    /// What it is doing.
    #[serde(default)]
    pub state: StationState,
}

/// One line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Line {
    /// The log folder.
    #[serde(default)]
    pub id: String,
    /// The name.
    #[serde(default)]
    pub title: String,
    /// What starts it.
    #[serde(default)]
    pub trigger: String,
    /// In belt order.
    #[serde(default)]
    pub stations: Vec<Station>,
    /// Runs logged.
    #[serde(default)]
    pub runs: u32,
    /// At work.
    #[serde(default)]
    pub active: bool,
    /// Its latest run.
    #[serde(default)]
    pub last_run: Option<LastRun>,
}

/// The latest run of a line, in a few words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LastRun {
    /// Seconds since its newest file.
    #[serde(default)]
    pub age_secs: Option<u64>,
}

/// A running process, drawn as a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Employee {
    /// `agent-loop/20261006-202608`.
    #[serde(default)]
    pub id: String,
    /// `Dev loop · #62`.
    #[serde(default)]
    pub name: String,
    /// The line.
    #[serde(default)]
    pub workflow: String,
    /// The station beside them.
    #[serde(default)]
    pub station: Option<String>,
    /// The stage.
    #[serde(default)]
    pub stage: Option<String>,
    /// The model.
    #[serde(default)]
    pub model: Option<String>,
    /// The round.
    #[serde(default)]
    pub round: Option<String>,
    /// Seconds since the last write.
    #[serde(default)]
    pub age_secs: Option<u64>,
    /// The last line written.
    #[serde(default)]
    pub last_line: String,
    /// Still at work.
    #[serde(default)]
    pub active: bool,
}

/// How far along a room is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomStatus {
    /// Real data.
    #[default]
    Live,
    /// Part of it.
    Draft,
    /// Barriers.
    Construction,
}

/// One room.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Room {
    /// 1 to 6.
    #[serde(default)]
    pub id: u8,
    /// The key.
    #[serde(default)]
    pub key: String,
    /// The name.
    #[serde(default)]
    pub name: String,
    /// One sentence.
    #[serde(default)]
    pub blurb: String,
    /// How far along.
    #[serde(default)]
    pub status: RoomStatus,
}

/// Where an issue stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    /// Closed.
    Done,
    /// Waiting for the merge.
    Delivered,
    /// On a human.
    Human,
    /// Blocked.
    Blocked,
    /// Next up.
    Ready,
    /// Open, not ready.
    #[default]
    Todo,
}

/// An issue, enough to draw a card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Issue {
    /// The number.
    #[serde(default)]
    pub number: u64,
    /// The title.
    #[serde(default)]
    pub title: String,
    /// `open` / `closed`.
    #[serde(default)]
    pub state: String,
    /// Where it stands.
    #[serde(default)]
    pub status: IssueStatus,
}

/// A milestone with its tasks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Milestone {
    /// The milestone issue.
    #[serde(default)]
    pub issue: Issue,
    /// The tasks.
    #[serde(default)]
    pub tasks: Vec<Issue>,
}

/// The sign.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Board {
    /// Milestones.
    #[serde(default)]
    pub milestones: Vec<Milestone>,
}

/// A branch on a shelf.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Version {
    /// The branch.
    #[serde(default)]
    pub name: String,
    /// Its page.
    #[serde(default)]
    pub url: String,
    /// What it is.
    #[serde(default)]
    pub label: String,
}

/// The store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Versions {
    /// `main`.
    #[serde(default)]
    pub main: Version,
    /// The agents' branch.
    #[serde(default)]
    pub integration: Version,
    /// One per open milestone.
    #[serde(default)]
    pub milestones: Vec<Version>,
}

/// A window of the rate limit.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Window {
    /// Its name.
    #[serde(default)]
    pub name: String,
    /// How much is used, `0..=1`.
    #[serde(default)]
    pub utilization: f64,
}

/// The quota reading.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Quota {
    /// The windows.
    #[serde(default)]
    pub windows: Vec<Window>,
}

/// Token totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct Tokens {
    /// Input.
    #[serde(default)]
    pub input: u64,
    /// Output.
    #[serde(default)]
    pub output: u64,
    /// Cache read.
    #[serde(default)]
    pub cache_read: u64,
    /// Cache write.
    #[serde(default)]
    pub cache_write: u64,
}

/// The control room's numbers.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Costs {
    /// Dollars.
    #[serde(default)]
    pub total_usd: f64,
    /// Paid sessions.
    #[serde(default)]
    pub sessions: u32,
    /// Tokens.
    #[serde(default)]
    pub tokens: Tokens,
}

/// The whole picture.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Snapshot {
    /// The project.
    #[serde(default)]
    pub project: Project,
    /// Seen from outside.
    #[serde(default)]
    pub factory: Factory,
    /// At work.
    #[serde(default)]
    pub employees: Vec<Employee>,
    /// The lines.
    #[serde(default)]
    pub lines: Vec<Line>,
    /// The rooms.
    #[serde(default)]
    pub rooms: Vec<Room>,
    /// The sign.
    #[serde(default)]
    pub board: Option<Board>,
    /// The store.
    #[serde(default)]
    pub versions: Versions,
    /// The numbers.
    #[serde(default)]
    pub costs: Costs,
    /// The quota.
    #[serde(default)]
    pub quota: Option<Quota>,
    /// Shown live although over.
    #[serde(default)]
    pub demo: bool,
}

impl Snapshot {
    /// Reads a picture the server published. Unknown fields are ignored and
    /// missing ones default — a renderer must draw whatever it is given.
    ///
    /// # Errors
    ///
    /// The text is not a JSON object.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_with_only_a_project_still_parses() {
        let snap = Snapshot::parse(r#"{"project":{"name":"dnd_helper"},"unknown":[1,2]}"#)
            .expect("parses");
        assert_eq!(snap.project.name, "dnd_helper");
        assert!(snap.factory.idle);
        assert_eq!(snap.lines, [] as [Line; 0]);
    }

    #[test]
    fn the_server_s_enums_read_by_their_snake_case_names() {
        let snap = Snapshot::parse(
            r#"{"lines":[{"id":"agent-loop","stations":[{"id":"code","kind":"builder","state":"active","model":"sonnet"}]}],
                "rooms":[{"id":6,"key":"construction","status":"construction"}],
                "board":{"milestones":[{"issue":{"number":17,"state":"open"},"tasks":[{"number":61,"status":"delivered"}]}]}}"#,
        )
        .expect("parses");
        assert_eq!(snap.lines[0].stations[0].kind, Kind::Builder);
        assert_eq!(snap.lines[0].stations[0].state, StationState::Active);
        assert_eq!(snap.rooms[0].status, RoomStatus::Construction);
        let board = snap.board.expect("board");
        assert_eq!(board.milestones[0].tasks[0].status, IssueStatus::Delivered);
    }

    #[test]
    fn something_that_is_not_an_object_is_an_error_not_a_blank_plant() {
        assert!(Snapshot::parse("42").is_err());
        assert!(Snapshot::parse("not json").is_err());
    }
}
