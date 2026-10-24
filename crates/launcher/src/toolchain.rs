//! The toolchain the harness's spawned processes run with, resolved in one
//! place: the product's scripts and commands the harness spawns
//! (`Processes`) and every Claude session (`ClaudeCliFactory`, whose
//! `SHELL` is bash 5, so a session's Bash tool runs bash, not zsh). `git`
//! and `gh` keep the harness's own environment.
//!
//! Composition-root code: it reads the process's environment (`PATH`,
//! `HOME`, `RUSTUP_HOME`, `CARGO_HOME`), the disk (the rustup toolchains,
//! the shims) and asks each tool its version through the adapters' one
//! spawning module. Nothing inside `core` or `workflows` knows it exists:
//! what it composes is handed to each adapter the launcher builds
//! ([`processes`], [`environment`]).
//!
//! The `PATH`, in order:
//!
//! 1. the shims, `<factory>/.llocal/toolchain/bin`: `sed` → `gsed` and
//!    `grep` → `ggrep` when Homebrew installed them, `bash` → the first
//!    bash 5 or newer, `node` → the first node 22 or newer — links, not
//!    folders, so no other tool of their folder moves ahead;
//! 2. the cargo of the channel the **product's** `rust-toolchain.toml` pins,
//!    from rustup's own toolchains; without it (no pin, a pin not
//!    installed, a pin that could not be read) rustup's proxies
//!    (`$CARGO_HOME/bin`) — never Homebrew's cargo ahead of them, and a
//!    cargo that is not rustup's is a problem that names its fix;
//! 3. then the inherited `PATH`, in its own order, every folder once.
//!
//! What it found is said once per start — the watch's, every run's — as
//! [`Event::ToolchainResolved`]: each tool with its version and where it
//! is, then the whole `PATH`. `harness toolchain` prints the same line.

use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use harness_adapters::shell::github::GhCli;
use harness_adapters::shell::process::{self, Processes};
use harness_core::domain::Slug;
use harness_core::domain::workspace::Workspace;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};

use crate::cli::ToolchainArgs;

/// Where Homebrew puts its tools: searched after the inherited `PATH` for
/// what is linked, so a watch started with a bare `PATH` still finds bash 5
/// and GNU sed. Never added to the `PATH` itself.
const WELL_KNOWN: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The oldest bash a session or a product's script may run: bash 3.2, the
/// one macOS ships, refuses `globstar`, associative arrays, `${x,,}`.
const BASH_AT_LEAST: u32 = 5;

/// The oldest node the mock-up's scripts run on: ESM syntax detection.
const NODE_AT_LEAST: u32 = 22;

/// The file that pins a product's Rust channel.
const TOOLCHAIN_FILE: &str = "rust-toolchain.toml";

/// The GNU tools Homebrew installs under a `g` prefix, by the name a script
/// calls them, with the formula that installs each.
const GNU: [(&str, &str, &str); 2] = [("sed", "gsed", "gnu-sed"), ("grep", "ggrep", "grep")];

thread_local! {
    /// The environment the last start took ([`take`]): what every adapter
    /// the launcher builds afterwards is handed. Per thread, not per
    /// process: the watch and a run are one thread (`current_thread`), and
    /// each test is its own, so a test's toolchain never reaches another's.
    static TAKEN: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// The variables the last start's toolchain gives a spawned process —
/// `PATH`, `SHELL`, `CLAUDE_CODE_SHELL` — for a `ClaudeCliFactory` the
/// launcher builds; empty before any start resolved one.
pub fn environment() -> Vec<(String, String)> {
    TAKEN.with(|taken| taken.borrow().clone())
}

/// The [`Process`](harness_core::ports::shell::process::Process) adapter,
/// carrying the last start's toolchain ([`environment`]).
pub fn processes() -> Processes {
    Processes::with_env(environment())
}

/// What the product says of its Rust channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    /// Its `rust-toolchain.toml` names this channel.
    Channel(String),
    /// It has no `rust-toolchain.toml`, or one that names no channel.
    Unpinned,
    /// The file could not be read: why, said as a problem — a failed read
    /// is never taken for "unpinned".
    Unreadable(String),
}

impl Pin {
    /// The pin a `rust-toolchain.toml`'s text names.
    fn of(toml: &str) -> Self {
        channel_of(toml).map_or(Self::Unpinned, Self::Channel)
    }

    /// The pin a read of the file found: the text, absent, or why not.
    fn read(read: Result<Option<String>, String>, place: &str) -> Self {
        match read {
            Ok(Some(toml)) => Self::of(&toml),
            Ok(None) => Self::Unpinned,
            Err(why) => Self::Unreadable(format!("cannot read {TOOLCHAIN_FILE} {place}: {why}")),
        }
    }

    /// The channel, when one is pinned.
    fn channel(&self) -> Option<&str> {
        match self {
            Self::Channel(channel) => Some(channel),
            Self::Unpinned | Self::Unreadable(_) => None,
        }
    }
}

/// What the resolver reads of the process, given rather than read, so a
/// test hands it folders of its own.
pub struct Inputs {
    /// The `PATH` the harness was started with.
    pub inherited_path: String,
    /// Folders searched after it for the linked tools ([`WELL_KNOWN`]).
    pub extra: Vec<PathBuf>,
    /// Where rustup keeps its toolchains' parent (`~/.rustup`).
    pub rustup_home: Option<PathBuf>,
    /// Where rustup keeps its proxies' parent (`~/.cargo`).
    pub cargo_home: Option<PathBuf>,
    /// The home folder, shown as `~` in the tools' places.
    pub home: Option<PathBuf>,
    /// What the product pins.
    pub pin: Pin,
    /// The folder the shims are written in.
    pub shims: PathBuf,
}

impl Inputs {
    /// This process's own: its `PATH`, `HOME`, `RUSTUP_HOME` (else
    /// `~/.rustup`), `CARGO_HOME` (else `~/.cargo`), the shims under the
    /// factory's `.llocal/`.
    fn of_process(here: &Path, pin: Pin) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let under_home = |variable: &str, folder: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|home| home.join(folder)))
        };
        Self {
            inherited_path: std::env::var("PATH").unwrap_or_default(),
            extra: WELL_KNOWN.iter().map(PathBuf::from).collect(),
            rustup_home: under_home("RUSTUP_HOME", ".rustup"),
            cargo_home: under_home("CARGO_HOME", ".cargo"),
            home,
            pin,
            shims: Workspace::new(here)
                .state_root()
                .join(".llocal/toolchain/bin"),
        }
    }
}

/// The tools found, the `PATH` composed, and what is wrong with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// One per tool, `name version (where)`: bash, cargo, sed, grep, node.
    tools: Vec<String>,
    /// The `PATH` every spawned process gets.
    path: String,
    /// The bash a session runs, when one is new enough.
    shell: Option<String>,
    /// What is missing or too old, each naming its fix.
    problems: Vec<String>,
}

impl Toolchain {
    /// The event that says it.
    pub fn event(&self) -> Event {
        Event::ToolchainResolved {
            tools: self.tools.clone(),
            path: self.path.clone(),
            problems: self.problems.clone(),
        }
    }

    /// What every spawned process gets on top of the harness's own
    /// environment: the `PATH`, and when a bash 5 was found, `SHELL` and
    /// Claude Code's own override of its shell detection
    /// (`CLAUDE_CODE_SHELL`), so a session's Bash tool runs it.
    pub fn environment(&self) -> Vec<(String, String)> {
        let mut env = vec![("PATH".to_string(), self.path.clone())];
        if let Some(shell) = &self.shell {
            env.push(("SHELL".to_string(), shell.clone()));
            env.push(("CLAUDE_CODE_SHELL".to_string(), shell.clone()));
        }
        env
    }
}

/// Resolves the toolchain from what `inputs` holds: writes the shims,
/// composes the `PATH`, then asks each tool the composed `PATH` resolves
/// its version — what is said is what a process will run, not what was
/// meant.
pub async fn resolve(inputs: &Inputs) -> Toolchain {
    let inherited = folders(&inputs.inherited_path);
    let searched = once_each(inherited.iter().chain(&inputs.extra).cloned());
    let mut problems = Vec::new();

    let pinned = pinned(inputs, &mut problems);
    // Without the pinned toolchain, rustup's proxies: they read a checkout's
    // own pin, or run rustup's default — and keep any other cargo behind.
    let proxies = proxies(inputs.cargo_home.as_deref(), &searched);
    let cargo_dir = pinned.or_else(|| proxies.clone());

    let links = [
        (GNU[0].0, which(GNU[0].1, &searched)),
        (GNU[1].0, which(GNU[1].1, &searched)),
        (
            "bash",
            first_at_least("bash", &searched, BASH_AT_LEAST).await,
        ),
        (
            "node",
            first_at_least("node", &searched, NODE_AT_LEAST).await,
        ),
    ];
    let shimmed = shim(&inputs.shims, &links, &mut problems);

    let prefix = [shimmed.then(|| inputs.shims.clone()), cargo_dir];
    let composed = once_each(prefix.into_iter().flatten().chain(inherited));
    let path = composed
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(":");

    let seen = Seen {
        home: inputs.home.as_deref(),
        shims: &inputs.shims,
    };
    let mut tools = Vec::new();
    let shell = bash_entry(
        which("bash", &composed).as_deref(),
        seen,
        &mut tools,
        &mut problems,
    )
    .await;
    let rustup = [inputs.rustup_home.clone(), proxies];
    cargo_entry(
        which("cargo", &composed).as_deref(),
        &inputs.pin,
        &rustup,
        seen,
        &mut tools,
        &mut problems,
    )
    .await;
    for (name, _, formula) in GNU {
        gnu_entry(
            name,
            formula,
            which(name, &composed).as_deref(),
            seen,
            &mut tools,
            &mut problems,
        )
        .await;
    }
    node_entry(
        which("node", &composed).as_deref(),
        seen,
        &mut tools,
        &mut problems,
    )
    .await;
    Toolchain {
        tools,
        path,
        shell,
        problems,
    }
}

/// The `bin` of rustup's toolchain for the channel the product pins, or the
/// problem that says why there is none: a channel not installed, a pin
/// that could not be read, a toolchain without clippy.
fn pinned(inputs: &Inputs, problems: &mut Vec<String>) -> Option<PathBuf> {
    let channel = match &inputs.pin {
        Pin::Channel(channel) => channel,
        Pin::Unreadable(why) => {
            problems.push(why.clone());
            return None;
        }
        Pin::Unpinned => return None,
    };
    let Some(dir) = inputs
        .rustup_home
        .as_deref()
        .and_then(|home| pinned_dir(home, channel))
    else {
        problems.push(format!(
            "the Rust toolchain {channel} the product pins is not installed: rustup toolchain \
             install {channel}"
        ));
        return None;
    };
    if !executable(&dir.join("cargo-clippy")) {
        problems.push(format!(
            "clippy is missing from the Rust toolchain {channel}: rustup component add clippy \
             --toolchain {channel}"
        ));
    }
    Some(dir)
}

/// The folder of rustup's proxies, the `cargo` that reads a checkout's
/// pin: `$CARGO_HOME/bin` (rustup's own installer), else the folder of the
/// `rustup` found, its links followed (Homebrew's keg).
fn proxies(cargo_home: Option<&Path>, searched: &[PathBuf]) -> Option<PathBuf> {
    let keg = which("rustup", searched)
        .and_then(|rustup| std::fs::canonicalize(rustup).ok())
        .and_then(|rustup| rustup.parent().map(Path::to_path_buf));
    cargo_home
        .map(|home| home.join("bin"))
        .into_iter()
        .chain(keg)
        .find(|bin| executable(&bin.join("cargo")))
}

/// How a tool's place is written: the home folder as `~`, a shim as
/// `shim→<its target>`.
#[derive(Clone, Copy)]
struct Seen<'a> {
    /// The home folder.
    home: Option<&'a Path>,
    /// The shims' folder.
    shims: &'a Path,
}

impl Seen<'_> {
    /// Where `found` is, as the line says it.
    fn place(self, found: &Path) -> String {
        let target = self.target(found);
        if target == found {
            shown(found, self.home)
        } else {
            format!("shim→{}", shown(&target, self.home))
        }
    }

    /// What `found` runs: a shim's target, else itself.
    fn target(self, found: &Path) -> PathBuf {
        if found.parent() == Some(self.shims) {
            std::fs::read_link(found).unwrap_or_else(|_| found.to_path_buf())
        } else {
            found.to_path_buf()
        }
    }
}

/// bash's entry; the bash a session runs when it is new enough.
async fn bash_entry(
    bash: Option<&Path>,
    seen: Seen<'_>,
    tools: &mut Vec<String>,
    problems: &mut Vec<String>,
) -> Option<String> {
    let Some(bash) = bash else {
        tools.push("bash missing".to_string());
        problems.push(format!(
            "bash {BASH_AT_LEAST} or newer is missing: brew install bash"
        ));
        return None;
    };
    let version = version_of(bash).await;
    tools.push(format!(
        "bash {} ({})",
        version.as_deref().unwrap_or("?"),
        seen.place(bash)
    ));
    if version.as_deref().and_then(major) >= Some(BASH_AT_LEAST) {
        Some(seen.target(bash).display().to_string())
    } else {
        problems.push(format!(
            "bash {BASH_AT_LEAST} or newer is missing, {} answers: brew install bash",
            seen.place(bash)
        ));
        None
    }
}

/// cargo's entry, naming the pin; a problem when the cargo that runs is not
/// rustup's — under `rustup` (its toolchains, its proxies) — since only
/// rustup's honours a product's pin.
async fn cargo_entry(
    cargo: Option<&Path>,
    pin: &Pin,
    rustup: &[Option<PathBuf>],
    seen: Seen<'_>,
    tools: &mut Vec<String>,
    problems: &mut Vec<String>,
) {
    let pin = pin
        .channel()
        .map_or_else(|| ", unpinned".to_string(), |c| format!(", pinned {c}"));
    let Some(cargo) = cargo else {
        tools.push(format!("cargo missing (no cargo on PATH{pin})"));
        return;
    };
    let folder = cargo.parent().unwrap_or(cargo);
    tools.push(format!(
        "cargo {} ({}{pin})",
        version_of(cargo).await.as_deref().unwrap_or("?"),
        shown(folder, seen.home)
    ));
    if !rustup.iter().flatten().any(|root| folder.starts_with(root)) {
        problems.push(format!(
            "the cargo that runs, {}, is not rustup's and ignores the product's pin: install \
             rustup (https://rustup.rs)",
            shown(folder, seen.home)
        ));
    }
}

/// `sed`'s or `grep`'s entry: GNU, through a shim or not, or a problem.
async fn gnu_entry(
    name: &str,
    formula: &str,
    found: Option<&Path>,
    seen: Seen<'_>,
    tools: &mut Vec<String>,
    problems: &mut Vec<String>,
) {
    let Some(found) = found else {
        tools.push(format!("{name} missing"));
        problems.push(format!("GNU {name} is missing: brew install {formula}"));
        return;
    };
    let place = seen.place(found);
    // `(GNU sed)`, not merely `GNU`: macOS's grep says it is "BSD grep,
    // GNU compatible".
    let line = first_line(found).await;
    if line.contains(&format!("(GNU {name})")) {
        tools.push(format!(
            "{name} GNU {} ({place})",
            version_in(&line).as_deref().unwrap_or("?")
        ));
    } else {
        tools.push(format!("{name} not GNU ({place})"));
        problems.push(format!(
            "GNU {name} is missing, {place} is not it: brew install {formula}"
        ));
    }
}

/// node's entry, or a problem when it is missing or too old.
async fn node_entry(
    node: Option<&Path>,
    seen: Seen<'_>,
    tools: &mut Vec<String>,
    problems: &mut Vec<String>,
) {
    let Some(node) = node else {
        tools.push("node missing".to_string());
        problems.push(format!(
            "node {NODE_AT_LEAST} or newer is missing: brew install node"
        ));
        return;
    };
    let version = version_of(node).await;
    tools.push(format!(
        "node {} ({})",
        version.as_deref().unwrap_or("?"),
        seen.place(node)
    ));
    if version.as_deref().and_then(major) < Some(NODE_AT_LEAST) {
        problems.push(format!(
            "node {NODE_AT_LEAST} or newer is missing, {} answers: brew install node",
            seen.place(node)
        ));
    }
}

/// Writes each `(name, target)` of `links` as a link `dir/name` → `target`,
/// takes back a link whose target was not found; whether a link is left.
/// A link already right is left alone, a wrong one replaced atomically —
/// lanes start side by side.
fn shim(dir: &Path, links: &[(&str, Option<PathBuf>)], problems: &mut Vec<String>) -> bool {
    let mut any = false;
    for (name, target) in links {
        let link = dir.join(name);
        let Some(target) = target else {
            let _ = std::fs::remove_file(&link);
            continue;
        };
        if std::fs::read_link(&link).ok().as_ref() == Some(target) {
            any = true;
            continue;
        }
        let fresh = dir.join(format!(".{name}.{}", std::process::id()));
        let written = std::fs::create_dir_all(dir)
            .and_then(|()| {
                let _ = std::fs::remove_file(&fresh);
                std::os::unix::fs::symlink(target, &fresh)
            })
            .and_then(|()| std::fs::rename(&fresh, &link));
        match written {
            Ok(()) => any = true,
            Err(e) => problems.push(format!(
                "the shim {} cannot be written: {e}",
                link.display()
            )),
        }
    }
    any
}

/// The `bin` folder of rustup's toolchain for `channel` holding a cargo:
/// `<channel>` itself, else the first `<channel>-<host>`.
fn pinned_dir(rustup_home: &Path, channel: &str) -> Option<PathBuf> {
    let toolchains = rustup_home.join("toolchains");
    let mut names: Vec<String> = std::fs::read_dir(&toolchains)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name == channel || name.starts_with(&format!("{channel}-")))
        .collect();
    // The exact name first, then the hosts in a stable order.
    names.sort_by_key(|name| (name != channel, name.clone()));
    names
        .into_iter()
        .map(|name| toolchains.join(name).join("bin"))
        .find(|bin| executable(&bin.join("cargo")))
}

/// The first `name` in `dirs` whose version is at least `at_least`.
async fn first_at_least(name: &str, dirs: &[PathBuf], at_least: u32) -> Option<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(name);
        if executable(&candidate)
            && version_of(&candidate).await.as_deref().and_then(major) >= Some(at_least)
        {
            return Some(candidate);
        }
    }
    None
}

/// The first executable `name` in `dirs`, as a process would find it.
fn which(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|candidate| executable(candidate))
}

/// Whether `path` is a file someone may execute.
fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The first line `binary --version` prints, on either pipe.
async fn first_line(binary: &Path) -> String {
    let Ok(ran) = process::run(
        &binary.display().to_string(),
        &["--version".to_string()],
        Path::new("/"),
    )
    .await
    else {
        return String::new();
    };
    let text = if ran.stdout.trim().is_empty() {
        ran.stderr
    } else {
        ran.stdout
    };
    text.lines().next().unwrap_or_default().trim().to_string()
}

/// The version `binary --version` names.
async fn version_of(binary: &Path) -> Option<String> {
    version_in(&first_line(binary).await)
}

/// The first version in a `--version` line: `5.2.37` of `GNU bash, version
/// 5.2.37(1)-release`, `v22.14.0` of node's, `4.9` of `gsed (GNU sed) 4.9`.
pub fn version_in(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let (lead, rest) = token
            .strip_prefix('v')
            .map_or(("", token), |rest| ("v", rest));
        let number: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let number = number.trim_end_matches('.');
        number
            .starts_with(|c: char| c.is_ascii_digit())
            .then(|| format!("{lead}{number}"))
    })
}

/// The major number of a version: `22` of `v22.14.0`.
pub fn major(version: &str) -> Option<u32> {
    version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
}

/// The channel a `rust-toolchain.toml` pins: `channel = "1.99.1"`.
pub fn channel_of(toml: &str) -> Option<String> {
    toml.lines().find_map(|line| {
        let value = line.trim().strip_prefix("channel")?.trim_start();
        let value = value.strip_prefix('=')?.trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let (channel, _) = value.strip_prefix(quote)?.split_once(quote)?;
        (!channel.trim().is_empty()).then(|| channel.trim().to_string())
    })
}

/// The folders of a `PATH`, empty entries dropped.
fn folders(path: &str) -> Vec<PathBuf> {
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Every folder once, in the order first met.
fn once_each(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !kept.contains(&dir) {
            kept.push(dir);
        }
    }
    kept
}

/// `path`, with the home folder written `~`.
fn shown(path: &Path, home: Option<&Path>) -> String {
    home.and_then(|home| path.strip_prefix(home).ok())
        .map_or_else(
            || path.display().to_string(),
            |rest| format!("~/{}", rest.display()),
        )
}

/// Resolves this process's toolchain for a product pinned as `pin`, makes
/// it every adapter the launcher builds from now on ([`environment`]), and
/// says it in `log`.
pub async fn take(here: &Path, pin: Pin, log: &Logbook) -> Toolchain {
    let toolchain = resolve(&Inputs::of_process(here, pin)).await;
    TAKEN.with(|taken| *taken.borrow_mut() = toolchain.environment());
    log.event(&toolchain.event());
    toolchain
}

/// The [`Process`](harness_core::ports::shell::process::Process) adapter
/// for a checkout whose pin is the one its tree holds as a command runs —
/// a merge just made — not the one a start read: rustup's proxies ahead of
/// any other cargo (resolved as for an unpinned product), so the
/// `rust-toolchain.toml` of the folder `cargo` runs in names the channel,
/// never Homebrew's cargo. Takes nothing: the watch's own toolchain stays
/// every other adapter's.
pub async fn by_the_tree(here: &Path) -> Processes {
    let toolchain = resolve(&Inputs::of_process(here, Pin::Unpinned)).await;
    Processes::with_env(toolchain.environment())
}

/// [`take`] at a run's start, the product's pin read from the checkout the
/// run mounted at `root` — its `run.log` says the tools it ran with.
pub async fn at_run_start(here: &Path, root: &Path, log: &Logbook) {
    take(here, pin_in(root), log).await;
}

/// The pin of the checkout at `root`: an absent file is unpinned, any other
/// failure to read it is said.
fn pin_in(root: &Path) -> Pin {
    let file = root.join(TOOLCHAIN_FILE);
    let read = match std::fs::read_to_string(&file) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    };
    Pin::read(read, &format!("in {}", root.display()))
}

/// [`take`] at the watch's start, the product's pin read on GitHub, on the
/// integration branch: the watch mounts no checkout of its own.
pub async fn at_watch_start(here: &Path, target_repo_url: &str, branch: &str, log: &Logbook) {
    let read = product_file(here, target_repo_url, branch).await;
    take(here, Pin::read(read, &on(target_repo_url, branch)), log).await;
}

/// Where the product's pin is read on GitHub, as a problem says it.
fn on(target_repo_url: &str, branch: &str) -> String {
    let repo = if target_repo_url.is_empty() {
        "this checkout's origin"
    } else {
        target_repo_url
    };
    format!("on {branch} of {repo}")
}

/// The product's `rust-toolchain.toml` on `branch`: its text, `None` when
/// it has none, or why it could not be read.
async fn product_file(
    here: &Path,
    target_repo_url: &str,
    branch: &str,
) -> Result<Option<String>, String> {
    let gh: Box<dyn GitHub> = if target_repo_url.is_empty() {
        Box::new(GhCli::new(here))
    } else {
        let slug = Slug::parse(target_repo_url)
            .ok_or_else(|| format!("{target_repo_url} names no GitHub repository"))?;
        Box::new(GhCli::for_slug(&slug))
    };
    gh.file_text(TOOLCHAIN_FILE, branch)
        .await
        .map_err(|halt| halt.reason().to_string())
}

/// `harness toolchain`: the line a start would say, for the owner and the
/// view — the channel given, else the product's on GitHub, else (no
/// product named, or none found there) this checkout's own
/// `rust-toolchain.toml`. Whether it has no problem.
pub async fn by_hand(args: &ToolchainArgs, here: &Path) -> (String, bool) {
    let pin = match &args.channel {
        Some(channel) => Pin::Channel(channel.clone()),
        None => match product_file(here, &args.target_repo_url, &args.branch).await {
            Ok(Some(toml)) => Pin::of(&toml),
            Err(why) if !args.target_repo_url.is_empty() => {
                Pin::read(Err(why), &on(&args.target_repo_url, &args.branch))
            }
            Ok(None) | Err(_) => pin_in(here),
        },
    };
    let toolchain = resolve(&Inputs::of_process(here, pin)).await;
    (
        toolchain.event().line().unwrap_or_default(),
        toolchain.problems.is_empty(),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use harness_core::ports::shell::process::Process;
    use harness_core::traces::{Sink, Verbosity};
    use proptest::prelude::*;

    use super::*;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(tag: &str) -> Dir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("harness-toolchain-{tag}-{nanos}"));
        std::fs::create_dir_all(&root).expect("mkdir");
        Dir(root)
    }

    /// A tool in `dir` that prints `said` when asked its version.
    fn tool(dir: &Path, name: &str, said: &str) -> PathBuf {
        std::fs::create_dir_all(dir).expect("mkdir");
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho '{said}'\n")).expect("tool");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    /// A machine like the owner's: the system's old tools first, Homebrew's
    /// after, nvm's node 22 between, rustup's pinned toolchain apart.
    struct Machine {
        root: Dir,
        system: PathBuf,
        nvm: PathBuf,
        brew: PathBuf,
        rustup: PathBuf,
    }

    fn machine(tag: &str) -> Machine {
        let root = dir(tag);
        let system = root.0.join("usr/bin");
        let nvm = root.0.join("nvm/v22/bin");
        let brew = root.0.join("brew/bin");
        let rustup = root.0.join("rustup");
        tool(
            &system,
            "bash",
            "GNU bash, version 3.2.57(1)-release (arm64-apple-darwin24)",
        );
        tool(&system, "sed", "sed: illegal option -- -");
        tool(
            &system,
            "grep",
            "grep (BSD grep, GNU compatible) 2.6.0-FreeBSD",
        );
        tool(&system, "node", "v12.22.10");
        tool(&nvm, "node", "v22.14.0");
        tool(
            &brew,
            "bash",
            "GNU bash, version 5.2.37(1)-release (aarch64-apple-darwin24)",
        );
        tool(&brew, "cargo", "cargo 1.98.1 (Homebrew)");
        tool(&brew, "gsed", "gsed (GNU sed) 4.9");
        tool(&brew, "ggrep", "ggrep (GNU grep) 3.11");
        tool(&brew, "node", "v26.0.0");
        let pinned = rustup.join("toolchains/1.99.1-aarch64-apple-darwin/bin");
        tool(&pinned, "cargo", "cargo 1.99.1 (aa11bb22 2026-09-01)");
        tool(&pinned, "cargo-clippy", "clippy 0.1.99");
        Machine {
            root,
            system,
            nvm,
            brew,
            rustup,
        }
    }

    fn inputs(m: &Machine, channel: Option<&str>) -> Inputs {
        Inputs {
            inherited_path: format!(
                "{}::{}:{}",
                m.system.display(),
                m.nvm.display(),
                m.brew.display()
            ),
            extra: Vec::new(),
            rustup_home: Some(m.rustup.clone()),
            cargo_home: None,
            home: Some(m.root.0.clone()),
            pin: channel.map_or(Pin::Unpinned, |c| Pin::Channel(c.to_string())),
            shims: m.root.0.join("factory/.llocal/toolchain/bin"),
        }
    }

    #[tokio::test]
    async fn the_pinned_cargo_gnu_tools_node_22_and_bash_5_come_first() {
        let m = machine("whole");
        let toolchain = resolve(&inputs(&m, Some("1.99.1"))).await;
        let shims = m.root.0.join("factory/.llocal/toolchain/bin");
        let pinned = m.rustup.join("toolchains/1.99.1-aarch64-apple-darwin/bin");
        assert_eq!(
            toolchain.path,
            [&shims, &pinned, &m.system, &m.nvm, &m.brew]
                .iter()
                .map(|dir| dir.display().to_string())
                .collect::<Vec<_>>()
                .join(":"),
            "the inherited order kept, every folder once, the empty entry dropped"
        );
        assert_eq!(
            toolchain.tools,
            [
                "bash 5.2.37 (shim→~/brew/bin/bash)".to_string(),
                "cargo 1.99.1 (~/rustup/toolchains/1.99.1-aarch64-apple-darwin/bin, pinned 1.99.1)"
                    .to_string(),
                "sed GNU 4.9 (shim→~/brew/bin/gsed)".to_string(),
                "grep GNU 3.11 (shim→~/brew/bin/ggrep)".to_string(),
                "node v22.14.0 (shim→~/nvm/v22/bin/node)".to_string(),
            ]
        );
        assert!(toolchain.problems.is_empty(), "{:?}", toolchain.problems);
        assert_eq!(
            toolchain.environment(),
            [
                ("PATH".to_string(), toolchain.path.clone()),
                (
                    "SHELL".to_string(),
                    m.brew.join("bash").display().to_string()
                ),
                (
                    "CLAUDE_CODE_SHELL".to_string(),
                    m.brew.join("bash").display().to_string()
                ),
            ]
        );
        for (name, target) in [
            ("sed", m.brew.join("gsed")),
            ("grep", m.brew.join("ggrep")),
            ("bash", m.brew.join("bash")),
            ("node", m.nvm.join("node")),
        ] {
            assert_eq!(
                std::fs::read_link(shims.join(name)).expect("a shim"),
                target
            );
        }
        // Linked, not prefixed: Homebrew's cargo stays where the inherited
        // `PATH` put it, behind the pinned one.
        assert!(
            toolchain
                .path
                .find(&m.brew.display().to_string())
                .is_some_and(|at| at > toolchain.path.find("rustup").unwrap_or(usize::MAX)),
            "{}",
            toolchain.path
        );
        // A second start finds the shims right and leaves them.
        assert_eq!(resolve(&inputs(&m, Some("1.99.1"))).await, toolchain);
    }

    #[tokio::test]
    async fn a_pinned_channel_not_installed_is_a_problem_naming_its_fix_not_a_silent_fallback() {
        let m = machine("unpinned");
        let toolchain = resolve(&inputs(&m, Some("1.99.2"))).await;
        let not_rustup = "the cargo that runs, ~/brew/bin, is not rustup's and ignores the \
                          product's pin: install rustup (https://rustup.rs)";
        assert_eq!(
            toolchain.problems,
            [
                "the Rust toolchain 1.99.2 the product pins is not installed: rustup toolchain \
              install 1.99.2",
                not_rustup
            ]
        );
        assert!(
            toolchain
                .tools
                .contains(&"cargo 1.98.1 (~/brew/bin, pinned 1.99.2)".to_string()),
            "what really runs is said: {:?}",
            toolchain.tools
        );
        assert_eq!(
            toolchain.event().line().map(|l| l.contains("problems:")),
            Some(true)
        );
        // No pin at all, no rustup: Homebrew's cargo runs, and it is said.
        let free = resolve(&inputs(&m, None)).await;
        assert_eq!(free.problems, [not_rustup]);
        assert!(
            free.tools
                .contains(&"cargo 1.98.1 (~/brew/bin, unpinned)".to_string())
        );
    }

    #[tokio::test]
    async fn without_the_pinned_toolchain_rustup_s_proxies_come_before_any_other_cargo() {
        let m = machine("proxies");
        let proxies = m.root.0.join("cargo/bin");
        tool(&proxies, "cargo", "cargo 1.99.1 (aa11bb22 2026-09-01)");
        let with_proxies = |pin| Inputs {
            cargo_home: Some(m.root.0.join("cargo")),
            pin,
            ..inputs(&m, None)
        };
        let free = resolve(&with_proxies(Pin::Unpinned)).await;
        assert!(free.problems.is_empty(), "{:?}", free.problems);
        assert!(
            free.tools
                .contains(&"cargo 1.99.1 (~/cargo/bin, unpinned)".to_string()),
            "{:?}",
            free.tools
        );
        let shims = m.root.0.join("factory/.llocal/toolchain/bin");
        assert!(
            free.path
                .starts_with(&format!("{}:{}:", shims.display(), proxies.display())),
            "{}",
            free.path
        );
        // A pin that could not be read is said, and the proxies run.
        let unread = resolve(&with_proxies(Pin::Unreadable(
            "cannot read rust-toolchain.toml on main_agent of o/p: HTTP 502".to_string(),
        )))
        .await;
        assert_eq!(
            unread.problems,
            ["cannot read rust-toolchain.toml on main_agent of o/p: HTTP 502"]
        );
        assert_eq!(unread.path, free.path);
        // The pinned toolchain, when installed, before the proxies.
        let pinned = resolve(&with_proxies(Pin::Channel("1.99.1".to_string()))).await;
        assert!(pinned.problems.is_empty(), "{:?}", pinned.problems);
        assert!(!pinned.path.contains(&proxies.display().to_string()));
    }

    #[tokio::test]
    async fn homebrew_s_rustup_keeps_its_proxies_in_its_keg_found_through_its_link() {
        let m = machine("keg");
        let keg = m.root.0.join("cellar/rustup/1.29.1/bin");
        tool(&keg, "rustup", "rustup 1.29.1");
        tool(&keg, "cargo", "cargo 1.99.1 (aa11bb22 2026-09-01)");
        std::os::unix::fs::symlink(keg.join("rustup"), m.brew.join("rustup")).expect("ln");
        let free = resolve(&inputs(&m, None)).await;
        assert!(free.problems.is_empty(), "{:?}", free.problems);
        // Links followed all the way: the temporary folder's own, too.
        let keg = std::fs::canonicalize(&keg).expect("the keg");
        assert!(
            free.tools
                .contains(&format!("cargo 1.99.1 ({}, unpinned)", keg.display())),
            "{:?}",
            free.tools
        );
        // `$CARGO_HOME/bin` first, when it holds the proxies.
        let home = m.root.0.join("cargo/bin");
        tool(&home, "cargo", "cargo 1.99.1 (aa11bb22 2026-09-01)");
        assert_eq!(
            proxies(Some(&m.root.0.join("cargo")), std::slice::from_ref(&m.brew)),
            Some(home)
        );
        assert_eq!(
            proxies(
                Some(&m.root.0.join("nowhere")),
                std::slice::from_ref(&m.brew)
            ),
            Some(keg)
        );
        assert_eq!(proxies(None, std::slice::from_ref(&m.system)), None);
    }

    #[tokio::test]
    async fn a_pinned_toolchain_without_clippy_says_how_to_add_it() {
        let m = machine("clippy");
        std::fs::remove_file(
            m.rustup
                .join("toolchains/1.99.1-aarch64-apple-darwin/bin/cargo-clippy"),
        )
        .expect("rm");
        let toolchain = resolve(&inputs(&m, Some("1.99.1"))).await;
        assert_eq!(
            toolchain.problems,
            [
                "clippy is missing from the Rust toolchain 1.99.1: rustup component add clippy \
              --toolchain 1.99.1"
            ]
        );
    }

    #[tokio::test]
    async fn without_gnu_tools_nor_new_ones_each_gap_is_said_with_its_fix() {
        let m = machine("bare");
        for gone in ["gsed", "ggrep", "bash", "node"] {
            std::fs::remove_file(m.brew.join(gone)).expect("rm");
        }
        std::fs::remove_file(m.nvm.join("node")).expect("rm");
        // A shim left by an earlier start whose tool is gone is taken back.
        let shims = m.root.0.join("factory/.llocal/toolchain/bin");
        std::fs::create_dir_all(&shims).expect("mkdir");
        std::os::unix::fs::symlink(m.brew.join("gsed"), shims.join("sed")).expect("ln");
        std::os::unix::fs::symlink(m.brew.join("bash"), shims.join("bash")).expect("ln");
        let toolchain = resolve(&inputs(&m, Some("1.99.1"))).await;
        assert!(
            !toolchain.path.contains("toolchain/bin"),
            "{}",
            toolchain.path
        );
        assert!(std::fs::symlink_metadata(shims.join("sed")).is_err());
        assert!(std::fs::symlink_metadata(shims.join("bash")).is_err());
        assert_eq!(
            toolchain.tools,
            [
                "bash 3.2.57 (~/usr/bin/bash)".to_string(),
                "cargo 1.99.1 (~/rustup/toolchains/1.99.1-aarch64-apple-darwin/bin, pinned 1.99.1)"
                    .to_string(),
                "sed not GNU (~/usr/bin/sed)".to_string(),
                "grep not GNU (~/usr/bin/grep)".to_string(),
                "node v12.22.10 (~/usr/bin/node)".to_string(),
            ]
        );
        assert_eq!(
            toolchain.problems,
            [
                "bash 5 or newer is missing, ~/usr/bin/bash answers: brew install bash",
                "GNU sed is missing, ~/usr/bin/sed is not it: brew install gnu-sed",
                "GNU grep is missing, ~/usr/bin/grep is not it: brew install grep",
                "node 22 or newer is missing, ~/usr/bin/node answers: brew install node",
            ]
        );
        assert_eq!(
            toolchain.environment(),
            [("PATH".to_string(), toolchain.path)],
            "no SHELL without a bash 5"
        );
    }

    #[tokio::test]
    async fn tools_absent_altogether_are_each_said_missing() {
        let empty = dir("empty");
        let toolchain = resolve(&Inputs {
            inherited_path: empty.0.display().to_string(),
            extra: Vec::new(),
            rustup_home: None,
            cargo_home: None,
            home: None,
            pin: Pin::Unpinned,
            shims: empty.0.join("shims"),
        })
        .await;
        assert_eq!(
            toolchain.tools,
            [
                "bash missing",
                "cargo missing (no cargo on PATH, unpinned)",
                "sed missing",
                "grep missing",
                "node missing"
            ]
        );
        assert_eq!(
            toolchain.problems,
            [
                "bash 5 or newer is missing: brew install bash",
                "GNU sed is missing: brew install gnu-sed",
                "GNU grep is missing: brew install grep",
                "node 22 or newer is missing: brew install node",
            ]
        );
        assert_eq!(toolchain.path, empty.0.display().to_string());
    }

    #[tokio::test]
    async fn a_gnu_tool_first_on_path_needs_no_shim() {
        // Linux: `sed` is GNU's, no `gsed` anywhere.
        let m = machine("linux");
        std::fs::remove_file(m.brew.join("gsed")).expect("rm");
        tool(&m.system, "sed", "sed (GNU sed) 4.8");
        let toolchain = resolve(&inputs(&m, Some("1.99.1"))).await;
        assert!(
            toolchain
                .tools
                .contains(&"sed GNU 4.8 (~/usr/bin/sed)".to_string())
        );
        assert!(toolchain.problems.is_empty(), "{:?}", toolchain.problems);
    }

    #[test]
    fn rustup_s_toolchain_is_found_by_its_channel_never_a_neighbour_s() {
        let root = dir("rustup");
        for name in [
            "1.99-aarch64-apple-darwin",
            "1.99.10-x",
            "stable-aarch64-apple-darwin",
        ] {
            tool(
                &root.0.join("toolchains").join(name).join("bin"),
                "cargo",
                "cargo",
            );
        }
        std::fs::create_dir_all(root.0.join("toolchains/1.99.1-empty/bin")).expect("mkdir");
        assert_eq!(pinned_dir(&root.0, "1.99.1"), None, "1.99.10 is not 1.99.1");
        assert_eq!(
            pinned_dir(&root.0, "stable"),
            Some(root.0.join("toolchains/stable-aarch64-apple-darwin/bin"))
        );
        tool(&root.0.join("toolchains/stable/bin"), "cargo", "cargo");
        assert_eq!(
            pinned_dir(&root.0, "stable"),
            Some(root.0.join("toolchains/stable/bin")),
            "the exact name first"
        );
        assert_eq!(pinned_dir(&root.0.join("nowhere"), "stable"), None);
    }

    #[test]
    fn a_file_without_the_execute_bit_is_no_tool() {
        let root = dir("bits");
        let path = tool(&root.0, "bash", "x");
        assert!(executable(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(!executable(&path));
        assert!(!executable(&root.0), "a folder is no tool");
    }

    #[test]
    fn versions_are_read_from_each_tool_s_own_line() {
        assert_eq!(
            version_in("GNU bash, version 5.2.37(1)-release (aarch64-apple-darwin24.0.0)")
                .as_deref(),
            Some("5.2.37")
        );
        assert_eq!(version_in("v22.14.0").as_deref(), Some("v22.14.0"));
        assert_eq!(version_in("gsed (GNU sed) 4.9").as_deref(), Some("4.9"));
        assert_eq!(
            version_in("cargo 1.99.1 (aa11bb22 2026-09-01)").as_deref(),
            Some("1.99.1")
        );
        assert_eq!(version_in("vim and v. x."), None);
        assert_eq!(version_in("rustc 7. (x)").as_deref(), Some("7"));
        assert_eq!(version_in(""), None);
        assert_eq!(major("v22.14.0"), Some(22));
        assert_eq!(major("5.2.37"), Some(5));
        assert_eq!(major("x"), None);
    }

    #[test]
    fn the_channel_is_read_from_the_toolchain_file() {
        let product = "# pinned\n[toolchain]\nchannel = \"1.99.1\"\ncomponents = [\"clippy\"]\n";
        assert_eq!(channel_of(product).as_deref(), Some("1.99.1"));
        assert_eq!(
            channel_of("channel='stable' # here").as_deref(),
            Some("stable")
        );
        assert_eq!(
            channel_of("  channel   =   \"nightly\"").as_deref(),
            Some("nightly")
        );
        assert_eq!(channel_of("channels = \"x\""), None);
        assert_eq!(channel_of("channel = 1.99"), None, "unquoted");
        assert_eq!(channel_of("channel = \"\""), None);
        assert_eq!(channel_of("channel = \"open"), None);
        assert_eq!(channel_of("[toolchain]\nprofile = \"minimal\""), None);
    }

    #[test]
    fn a_pin_is_a_channel_none_or_a_read_that_failed_never_mistaken_for_none() {
        let pinned = "[toolchain]\nchannel = \"1.99.1\"\n";
        assert_eq!(Pin::of(pinned), Pin::Channel("1.99.1".to_string()));
        assert_eq!(Pin::of("[toolchain]\n"), Pin::Unpinned);
        assert_eq!(
            Pin::read(Ok(Some(pinned.to_string())), "x"),
            Pin::Channel("1.99.1".to_string())
        );
        assert_eq!(Pin::read(Ok(None), "x"), Pin::Unpinned);
        assert_eq!(
            Pin::read(
                Err("HTTP 502".to_string()),
                &on("https://github.com/o/p", "main_agent")
            ),
            Pin::Unreadable(
                "cannot read rust-toolchain.toml on main_agent of https://github.com/o/p: HTTP 502"
                    .to_string()
            )
        );
        assert_eq!(on("", "b"), "on b of this checkout's origin");
        assert_eq!(Pin::Channel("x".to_string()).channel(), Some("x"));
        assert_eq!(Pin::Unpinned.channel(), None);
        assert_eq!(Pin::Unreadable("why".to_string()).channel(), None);
    }

    #[test]
    fn a_checkout_s_pin_absent_is_unpinned_and_unreadable_is_said() {
        let root = dir("pin");
        assert_eq!(pin_in(&root.0), Pin::Unpinned);
        std::fs::write(root.0.join(TOOLCHAIN_FILE), "channel = \"stable\"").expect("pin");
        assert_eq!(pin_in(&root.0), Pin::Channel("stable".to_string()));
        std::fs::remove_file(root.0.join(TOOLCHAIN_FILE)).expect("rm");
        // A folder where the file should be: read, and failed.
        std::fs::create_dir_all(root.0.join(TOOLCHAIN_FILE)).expect("mkdir");
        let Pin::Unreadable(why) = pin_in(&root.0) else {
            panic!("a failed read is said");
        };
        assert!(
            why.starts_with(&format!(
                "cannot read rust-toolchain.toml in {}: ",
                root.0.display()
            )),
            "{why}"
        );
    }

    #[test]
    fn a_link_outside_the_shims_is_shown_as_itself() {
        let root = dir("seen");
        let shims = root.0.join("shims");
        let elsewhere = root.0.join("brew");
        let real = tool(&root.0.join("cellar"), "gsed", "gsed (GNU sed) 4.9");
        for dir in [&shims, &elsewhere] {
            std::fs::create_dir_all(dir).expect("mkdir");
            std::os::unix::fs::symlink(&real, dir.join("sed")).expect("ln");
        }
        let seen = Seen {
            home: Some(&root.0),
            shims: &shims,
        };
        assert_eq!(seen.place(&shims.join("sed")), "shim→~/cellar/gsed");
        assert_eq!(seen.target(&shims.join("sed")), real);
        assert_eq!(seen.place(&elsewhere.join("sed")), "~/brew/sed");
        assert_eq!(seen.target(&elsewhere.join("sed")), elsewhere.join("sed"));
        // A file in the shims that is no link runs as itself.
        let plain = tool(&shims, "node", "v22.0.0");
        assert_eq!(seen.target(&plain), plain);
    }

    proptest! {
        #[test]
        fn any_quoted_channel_reads_back(channel in "[a-z0-9][a-z0-9.-]{0,20}") {
            let toml = format!("[toolchain]\nchannel = \"{channel}\"\n");
            prop_assert_eq!(channel_of(&toml), Some(channel));
        }

        #[test]
        fn any_line_reads_without_a_panic(line in ".{0,60}") {
            let _ = channel_of(&line);
            let _ = version_in(&line);
        }

        #[test]
        fn a_dotted_version_reads_back_with_its_major(
            major_number in 0_u32..1000,
            minor in 0_u32..1000,
            v in proptest::bool::ANY,
        ) {
            let lead = if v { "v" } else { "" };
            let version = format!("{lead}{major_number}.{minor}");
            prop_assert_eq!(version_in(&format!("tool {version} (x)")), Some(version.clone()));
            prop_assert_eq!(major(&version), Some(major_number));
        }
    }

    #[derive(Default)]
    struct Lines(RefCell<Vec<String>>);

    impl Sink for Lines {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[tokio::test]
    async fn a_run_s_start_says_its_toolchain_and_every_process_takes_it() {
        let here = dir("run");
        let checkout = here.0.join("checkout");
        std::fs::create_dir_all(&checkout).expect("mkdir");
        // A channel no machine has: the line says the product's pin was read.
        std::fs::write(
            checkout.join(TOOLCHAIN_FILE),
            "[toolchain]\nchannel = \"0.0.0-harness-test\"\n",
        )
        .expect("pin");
        let lines = Rc::new(Lines::default());
        let log = Logbook::new(Rc::clone(&lines) as Rc<dyn Sink>, Verbosity::Normal);
        at_run_start(&here.0, &checkout, &log).await;
        let said = lines.0.borrow().join("\n");
        assert!(said.contains("toolchain: bash"), "{said}");
        assert!(
            said.contains("rustup toolchain install 0.0.0-harness-test"),
            "{said}"
        );
        let path = said
            .split("PATH=")
            .nth(1)
            .and_then(|rest| rest.split(" — ").next())
            .expect("the PATH is said")
            .to_string();
        let ran = processes()
            .run(
                "sh",
                &["-c".to_string(), "printf %s \"$PATH\"".to_string()],
                Path::new("/"),
                std::time::Duration::from_secs(10),
            )
            .await
            .expect("ran");
        assert_eq!(ran.stdout, path, "the PATH said is the PATH a process gets");
        assert_eq!(environment().first(), Some(&("PATH".to_string(), path)));
        // Another thread — another test — never sees it.
        let elsewhere = std::thread::spawn(environment).join().expect("joined");
        assert!(elsewhere.is_empty(), "{elsewhere:?}");
    }
}
