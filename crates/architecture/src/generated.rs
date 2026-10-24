//! The files of a product a script writes, never a hand: what a merge
//! regenerates instead of resolving.
//!
//! One table ([`GENERATED`]) read by both sides that meet a conflict: the
//! harness's own catch-up of a milestone branch (`harness-workflows`'
//! `branching::catch_up_locally`) and the prompt of the repair session a
//! conflicting pull request gets (`pr_fix`'s conflict block). Written once
//! here so the two can never disagree on which file is whose.
//!
//! A conflict in a generated file is solved by taking the integration
//! branch's side and running the file's generators ([`Regenerator`]) on the
//! merged tree; a conflict anywhere else is a real one, merged by no script.
//! `mock/pages.json` is half of each: a hand writes its pages and
//! components, `mock/build.sh` writes each component's `builtFrom` digest —
//! a conflict there is generated only when the digests are all that differ
//! ([`digests_only`]).
//!
//! Pure: no I/O.

use crate::tree::{REGISTRY, stales_every_bundle};

/// What runs to rewrite a generated file, in the order [`Regenerator::ALL`]
/// lists: the registry before the lock that reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Regenerator {
    /// `bash scripts/register.sh`: the registry's manifest, from the unit
    /// folders.
    Register,
    /// `bash scripts/queries.sh`: the persisted queries' registry (contract C).
    Queries,
    /// `sh mock/build.sh`: the mock-up's bundles and their `builtFrom`
    /// digests (contract N).
    MockBuild,
    /// `cargo metadata --format-version 1`: `Cargo.lock`, for the merged
    /// manifests — with the cargo the product pins.
    CargoMetadata,
}

impl Regenerator {
    /// Every generator, in the order they run.
    pub const ALL: [Self; 4] = [
        Self::Register,
        Self::Queries,
        Self::MockBuild,
        Self::CargoMetadata,
    ];

    /// The program it runs.
    #[must_use]
    pub const fn program(self) -> &'static str {
        match self {
            Self::Register | Self::Queries => "bash",
            Self::MockBuild => "sh",
            Self::CargoMetadata => "cargo",
        }
    }

    /// Its arguments, from the repository's root.
    #[must_use]
    pub const fn args(self) -> &'static [&'static str] {
        match self {
            Self::Register => &["scripts/register.sh"],
            Self::Queries => &["scripts/queries.sh"],
            Self::MockBuild => &["mock/build.sh"],
            Self::CargoMetadata => &["metadata", "--format-version", "1"],
        }
    }

    /// The command line, as a session types it.
    #[must_use]
    pub fn command(self) -> String {
        format!("{} {}", self.program(), self.args().join(" "))
    }
}

/// How a conflict in a generated file is taken before its generators run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// The integration branch's whole file: the generators rewrite it all.
    Theirs,
    /// The integration branch's side of each conflict, only when every
    /// conflict differs by the `builtFrom` digests alone ([`digests_only`]);
    /// any other difference is a real conflict.
    DigestsOnly,
}

/// One kind of generated file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generated {
    /// Its path; a pattern `<folder>/*<suffix>` names every file of that
    /// folder (not below it) ending so.
    pub pattern: &'static str,
    /// How its conflict is taken.
    pub take: Take,
    /// What rewrites it once taken, in their own order.
    pub by: &'static [Regenerator],
}

/// The generated files of a product: what a merge regenerates instead of
/// resolving. The bundles' HTML fragments (`mock/components/*.html`) are
/// hand-written, and not here.
pub const GENERATED: [Generated; 5] = [
    Generated {
        pattern: "Cargo.lock",
        take: Take::Theirs,
        by: &[Regenerator::CargoMetadata],
    },
    Generated {
        pattern: REGISTRY,
        take: Take::Theirs,
        by: &[Regenerator::Register, Regenerator::CargoMetadata],
    },
    Generated {
        pattern: "queries/registry.json",
        take: Take::Theirs,
        by: &[Regenerator::Queries],
    },
    Generated {
        pattern: "mock/components/*.js",
        take: Take::Theirs,
        by: &[Regenerator::MockBuild],
    },
    Generated {
        pattern: "mock/pages.json",
        take: Take::DigestsOnly,
        by: &[Regenerator::MockBuild],
    },
];

impl Generated {
    /// Whether `path` is one of its files.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        match self.pattern.split_once('*') {
            None => path == self.pattern,
            Some((folder, suffix)) => path
                .strip_prefix(folder)
                .and_then(|name| name.strip_suffix(suffix))
                .is_some_and(|stem| !stem.is_empty() && !stem.contains('/')),
        }
    }
}

/// The kind of generated file `path` is, if it is one.
#[must_use]
pub fn generated(path: &str) -> Option<&'static Generated> {
    GENERATED.iter().find(|kind| kind.matches(path))
}

/// The generators the files of `paths` need, each once, in the order they
/// run; a path that is not generated needs none.
#[must_use]
pub fn regenerators<S: AsRef<str>>(paths: &[S]) -> Vec<Regenerator> {
    Regenerator::ALL
        .into_iter()
        .filter(|regenerator| {
            paths
                .iter()
                .filter_map(|path| generated(path.as_ref()))
                .any(|kind| kind.by.contains(regenerator))
        })
        .collect()
}

/// The generators a merge runs even when git merges it cleanly.
///
/// From the paths one side changed since the merge base (`changed`):
/// `mock/build.sh` when one of them stales every bundle
/// ([`stales_every_bundle`]: the mock server, the screen host, the build) —
/// through a clean merge, a component only the other side has keeps a
/// bundle and a `builtFrom` digest of the old file, and its `pages-mocked`
/// gate turns red. `None`, changes not all known, counts as such a change.
///
/// Both ways: asked of the side merged in first, then — only when that
/// side [`brings_bundles`] — of the receiving side, whose own change to
/// such a file stales the bundles arriving with digests of the other
/// side's file.
#[must_use]
pub fn staled_by<S: AsRef<str>>(changed: Option<&[S]>) -> Vec<Regenerator> {
    let staled =
        changed.is_none_or(|paths| paths.iter().any(|path| stales_every_bundle(path.as_ref())));
    if staled {
        vec![Regenerator::MockBuild]
    } else {
        Vec::new()
    }
}

/// Whether `changed` (one side's paths since the merge base) brings a
/// bundle or a `builtFrom` digest — a file `mock/build.sh` writes: what the
/// other side's change to the mock-up's build stales ([`staled_by`]).
#[must_use]
pub fn brings_bundles<S: AsRef<str>>(changed: &[S]) -> bool {
    regenerators(changed).contains(&Regenerator::MockBuild)
}

/// The lines a repair session reads when its branch conflicts: which files
/// are regenerated, how each is taken, and what runs afterwards — the same
/// table the harness's own catch-up follows.
#[must_use]
pub fn instructions() -> String {
    let mut lines = vec![
        "These files are generated, never resolved by hand — when one conflicts, take \
         the base's side and regenerate it:"
            .to_string(),
    ];
    for kind in &GENERATED {
        let take = match kind.take {
            Take::Theirs => format!("`git checkout --theirs -- {}`", kind.pattern),
            Take::DigestsOnly => {
                "when only `\"builtFrom\"` digests conflict, keep the base's side of those \
                 lines; any other conflict in it is merged by hand (the union of both sides' \
                 pages and components)"
                    .to_string()
            }
        };
        let by: Vec<String> = kind
            .by
            .iter()
            .map(|regenerator| format!("`{}`", regenerator.command()))
            .collect();
        lines.push(format!(
            "  - `{}`: {take}, then {}",
            kind.pattern,
            by.join(" then ")
        ));
    }
    let order: Vec<String> = Regenerator::ALL
        .iter()
        .map(|regenerator| format!("`{}`", regenerator.command()))
        .collect();
    lines.push(format!(
        "Run the generators once each, in this order — {} — only those whose files \
         conflicted, and stage what they wrote.",
        order.join(", ")
    ));
    lines.join("\n")
}

/// What opens, separates and closes a conflict, as `git` writes it.
const OURS: &str = "<<<<<<<";
const BASE: &str = "|||||||";
const SPLIT: &str = "=======";
const THEIRS: &str = ">>>>>>>";

/// Whether `line` is the marker `marker`: alone, or followed by a space and
/// the side's name.
fn is_marker(line: &str, marker: &str) -> bool {
    line.strip_prefix(marker)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

/// `line` with the value of a `"builtFrom"` digest replaced by `#`, or the
/// line as it is when it holds no such digest.
fn masked(line: &str) -> String {
    const KEY: &str = "\"builtFrom\":";
    let Some(at) = line.find(KEY) else {
        return line.to_string();
    };
    let (head, rest) = line.split_at(at + KEY.len());
    let value = rest.trim_start();
    let Some(quoted) = value.strip_prefix('"') else {
        return line.to_string();
    };
    let Some(end) = quoted.find('"') else {
        return line.to_string();
    };
    let (digest, tail) = quoted.split_at(end);
    if digest.is_empty() || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return line.to_string();
    }
    format!("{head} \"#{tail}")
}

/// Whether two sides of a conflict differ by their `builtFrom` digests
/// alone.
fn same_but_digests(ours: &[&str], theirs: &[&str]) -> bool {
    ours.len() == theirs.len() && ours.iter().zip(theirs).all(|(a, b)| masked(a) == masked(b))
}

/// Where a reading of conflict markers stands.
enum Side {
    /// Outside any conflict.
    Out,
    /// In our side.
    Ours,
    /// In the common ancestor's side (`diff3`, `zdiff3`): skipped.
    Base,
    /// In their side.
    Theirs,
}

/// The text of a conflicted `mock/pages.json` with each conflict resolved
/// to the integration branch's side — when every conflict differs by the
/// `builtFrom` digests alone, which `mock/build.sh` then rewrites.
///
/// `None` when a conflict differs in anything else (a page, a component, a
/// prop: a hand's), when the markers do not close, or when the text holds
/// no conflict at all (nothing a digest explains). Both marker styles: the
/// plain one, and `diff3`'s with the ancestor between `|||||||` and
/// `=======`.
#[must_use]
pub fn digests_only(conflicted: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    let mut ours: Vec<&str> = Vec::new();
    let mut theirs: Vec<&str> = Vec::new();
    let mut side = Side::Out;
    let mut conflicts = 0_usize;
    for line in conflicted.lines() {
        side = match side {
            Side::Out if is_marker(line, OURS) => Side::Ours,
            Side::Out => {
                out.push(line);
                Side::Out
            }
            Side::Ours if is_marker(line, BASE) => Side::Base,
            Side::Ours | Side::Base if is_marker(line, SPLIT) => Side::Theirs,
            Side::Ours => {
                ours.push(line);
                Side::Ours
            }
            Side::Base => Side::Base,
            Side::Theirs if is_marker(line, THEIRS) => {
                if !same_but_digests(&ours, &theirs) {
                    return None;
                }
                out.append(&mut theirs);
                ours.clear();
                conflicts += 1;
                Side::Out
            }
            Side::Theirs => {
                theirs.push(line);
                Side::Theirs
            }
        };
    }
    if !matches!(side, Side::Out) || conflicts == 0 {
        return None;
    }
    let mut text = out.join("\n");
    if conflicted.ends_with('\n') {
        text.push('\n');
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_base_change_to_what_every_bundle_is_built_from_rebuilds_them_whatever_merges() {
        for path in ["mock/src/server.js", "mock/src/screen.js", "mock/build.mjs"] {
            assert_eq!(
                staled_by(Some(&["README.md", path])),
                [Regenerator::MockBuild],
                "{path}"
            );
        }
        assert_eq!(
            staled_by(Some(&["mock/handlers/table.move.js", "Cargo.lock"])),
            []
        );
        assert_eq!(staled_by::<&str>(Some(&[])), []);
        assert_eq!(
            staled_by::<&str>(None),
            [Regenerator::MockBuild],
            "changes not all known"
        );
    }

    #[test]
    fn a_side_brings_bundles_with_a_component_or_the_digests_never_with_a_fragment() {
        assert!(brings_bundles(&["README.md", "mock/components/Dice.js"]));
        assert!(brings_bundles(&["mock/pages.json"]));
        assert!(!brings_bundles(&[
            "mock/components/Dice.html",
            "mock/src/server.js",
            "Cargo.lock"
        ]));
        assert!(!brings_bundles::<&str>(&[]));
    }

    #[test]
    fn bundles_are_generated_fragments_are_not() {
        assert!(generated("mock/components/Battlemap.js").is_some());
        assert!(generated("mock/components/Battlemap.html").is_none());
        assert!(generated("mock/componentsX.js").is_none());
        assert!(generated("mock/components/.js").is_none(), "no name");
        assert!(generated("mock/components/a/B.js").is_none(), "not below");
        assert!(generated("x/mock/components/B.js").is_none());
        assert_eq!(
            generated("mock/pages.json").map(|kind| kind.take),
            Some(Take::DigestsOnly)
        );
        assert_eq!(
            generated("Cargo.lock").map(|kind| kind.by),
            Some(&[Regenerator::CargoMetadata][..])
        );
        assert!(generated(REGISTRY).is_some());
        assert!(generated("queries/registry.json").is_some());
        assert!(generated("crates/server/src/lib.rs").is_none());
        assert!(generated("mock/src/commands.js").is_none());
        assert!(generated("crates/x/Cargo.lock").is_none());
    }

    #[test]
    fn every_generator_is_an_installed_asset_or_cargo() {
        for regenerator in Regenerator::ALL {
            if regenerator == Regenerator::CargoMetadata {
                assert_eq!(regenerator.program(), "cargo");
            } else {
                let script = regenerator.args()[0];
                assert!(
                    crate::assets::content_of(script).is_some(),
                    "{script} is installed by init-repo"
                );
            }
        }
        assert_eq!(Regenerator::Register.command(), "bash scripts/register.sh");
        assert_eq!(Regenerator::Queries.command(), "bash scripts/queries.sh");
        assert_eq!(Regenerator::MockBuild.command(), "sh mock/build.sh");
        assert_eq!(
            Regenerator::CargoMetadata.command(),
            "cargo metadata --format-version 1"
        );
    }

    #[test]
    fn the_generators_run_once_each_registry_before_lock() {
        assert_eq!(
            regenerators(&["Cargo.lock", REGISTRY, "mock/pages.json"]),
            [
                Regenerator::Register,
                Regenerator::MockBuild,
                Regenerator::CargoMetadata
            ]
        );
        assert_eq!(
            regenerators(&["mock/components/A.js", "mock/components/B.js"]),
            [Regenerator::MockBuild]
        );
        assert_eq!(
            regenerators(&["queries/registry.json"]),
            [Regenerator::Queries]
        );
        assert!(
            regenerators(&["crates/a/src/lib.rs"]).is_empty(),
            "nothing expected"
        );
        assert!(regenerators::<&str>(&[]).is_empty(), "nothing expected");
    }

    #[test]
    fn the_instructions_name_every_file_and_generator_in_order() {
        let said = instructions();
        for kind in &GENERATED {
            assert!(said.contains(&format!("`{}`", kind.pattern)), "{said}");
        }
        assert!(
            said.contains("`git checkout --theirs -- Cargo.lock`"),
            "{said}"
        );
        assert!(
            said.contains(
                "`bash scripts/register.sh`, `bash scripts/queries.sh`, `sh mock/build.sh`, \
                 `cargo metadata --format-version 1`"
            ),
            "{said}"
        );
        assert!(
            said.contains("`mock/pages.json`: when only `\"builtFrom\"` digests conflict"),
            "{said}"
        );
        assert!(
            said.contains(&format!(
                "`{REGISTRY}`: `git checkout --theirs -- {REGISTRY}`, then `bash \
                 scripts/register.sh` then `cargo metadata --format-version 1`"
            )),
            "{said}"
        );
    }

    #[test]
    fn a_digest_only_conflict_takes_the_base_side() {
        let text = "{\n  \"components\": [\n    {\n      \"name\": \"A\",\n<<<<<<< HEAD\n      \"builtFrom\": \"aaaa\"\n=======\n      \"builtFrom\": \"bbbb\"\n>>>>>>> origin/main_agent\n    }\n  ]\n}\n";
        assert_eq!(
            digests_only(text).as_deref(),
            Some(
                "{\n  \"components\": [\n    {\n      \"name\": \"A\",\n      \"builtFrom\": \"bbbb\"\n    }\n  ]\n}\n"
            )
        );
        let diff3 = "x\n<<<<<<< ours\n\"builtFrom\": \"01\",\n||||||| base\n\"builtFrom\": \"ff\",\n=======\n\"builtFrom\": \"02\",\n>>>>>>> theirs\ny";
        assert_eq!(
            digests_only(diff3).as_deref(),
            Some("x\n\"builtFrom\": \"02\",\ny")
        );
    }

    #[test]
    fn any_other_difference_is_a_hand_s() {
        let page = "<<<<<<< HEAD\n  \"name\": \"A\"\n=======\n  \"name\": \"B\"\n>>>>>>> m\n";
        assert_eq!(digests_only(page), None);
        let longer =
            "<<<<<<< HEAD\n\"builtFrom\": \"aa\"\n=======\n\"builtFrom\": \"bb\"\n,\n>>>>>>> m\n";
        assert_eq!(digests_only(longer), None);
        let unclosed = "<<<<<<< HEAD\n\"builtFrom\": \"aa\"\n=======\n\"builtFrom\": \"bb\"\n";
        assert_eq!(digests_only(unclosed), None);
        assert_eq!(digests_only("{}\n"), None, "no conflict: nothing to say");
        let not_hex =
            "<<<<<<< a\n\"builtFrom\": \"zz\"\n=======\n\"builtFrom\": \"yy\"\n>>>>>>> b\n";
        assert_eq!(digests_only(not_hex), None, "only a digest is masked");
        let not_a_marker = "<<<<<<<<\n";
        assert_eq!(digests_only(not_a_marker), None);
    }

    #[test]
    fn a_digest_is_masked_only_when_it_is_one() {
        assert_eq!(
            masked("      \"builtFrom\": \"d3da2e468006f0ca\","),
            "      \"builtFrom\": \"#\","
        );
        assert_eq!(masked("\"builtFrom\":\"0a\""), "\"builtFrom\": \"#\"");
        assert_eq!(masked("\"builtFrom\": \"\""), "\"builtFrom\": \"\"");
        assert_eq!(masked("\"builtFrom\": 12"), "\"builtFrom\": 12");
        assert_eq!(masked("\"builtFrom\": \"ab"), "\"builtFrom\": \"ab");
        assert_eq!(masked("\"name\": \"ab\""), "\"name\": \"ab\"");
        assert!(is_marker("<<<<<<<", OURS) && is_marker("<<<<<<< HEAD", OURS));
        assert!(!is_marker("<<<<<<<HEAD", OURS) && !is_marker(" <<<<<<<", OURS));
    }

    /// A line of a manifest that is no marker.
    fn line() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-zA-Z0-9 {}\\[\\],:\"]{0,24}",
            "[0-9a-f]{1,16}".prop_map(|hex| format!("      \"builtFrom\": \"{hex}\"")),
        ]
        .prop_filter("no marker", |l| {
            ![OURS, BASE, SPLIT, THEIRS]
                .iter()
                .any(|marker| l.starts_with(marker))
        })
    }

    /// A side of a conflict: its lines, and the same lines with every digest
    /// changed.
    fn sides() -> impl Strategy<Value = (Vec<String>, Vec<String>)> {
        proptest::collection::vec(("[0-9a-f]{1,16}", "[0-9a-f]{1,16}", any::<bool>()), 1..4)
            .prop_map(|digests| {
                digests
                    .into_iter()
                    .map(|(a, b, comma)| {
                        let tail = if comma { "," } else { "" };
                        (
                            format!("  \"builtFrom\": \"{a}\"{tail}"),
                            format!("  \"builtFrom\": \"{b}\"{tail}"),
                        )
                    })
                    .unzip()
            })
    }

    proptest! {
        #[test]
        fn masked_digests_resolve_exactly_digest_only_conflicts(
            before in proptest::collection::vec(line(), 0..5),
            (ours, theirs) in sides(),
            middle in proptest::collection::vec(line(), 0..5),
            (ours2, theirs2) in sides(),
            diff3 in any::<bool>(),
            broken in any::<bool>(),
        ) {
            let hunk = |ours: &[String], theirs: &[String], text: &mut Vec<String>| {
                text.push("<<<<<<< HEAD".to_string());
                text.extend(ours.iter().cloned());
                if diff3 {
                    text.push("||||||| base".to_string());
                    text.push("  \"name\": \"anything\"".to_string());
                }
                text.push("=======".to_string());
                text.extend(theirs.iter().cloned());
                text.push(">>>>>>> origin/main_agent".to_string());
            };
            let mut text = before.clone();
            hunk(&ours, &theirs, &mut text);
            text.extend(middle.iter().cloned());
            let mut theirs2 = theirs2;
            if broken {
                theirs2.push("  \"props\": {}".to_string());
            }
            hunk(&ours2, &theirs2, &mut text);
            let joined = format!("{}\n", text.join("\n"));
            let resolved = digests_only(&joined);
            if broken {
                prop_assert_eq!(resolved, None);
            } else {
                let mut expected = before;
                expected.extend(theirs);
                expected.extend(middle);
                expected.extend(theirs2);
                let resolved = resolved.expect("digests only");
                prop_assert_eq!(&resolved, &format!("{}\n", expected.join("\n")));
                prop_assert!(!resolved.contains(OURS) && !resolved.contains(THEIRS));
            }
        }

        #[test]
        fn any_text_reads_without_a_panic(text in "(<<<<<<< |=======|>>>>>>> |\\|\\|\\|\\|\\|\\|\\||[a-z\": ]{0,12}|\n){0,30}") {
            let _ = digests_only(&text);
        }
    }
}
