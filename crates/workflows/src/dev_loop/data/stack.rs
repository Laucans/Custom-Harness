//! The repository's build and test configuration, verbatim.
//!
//! What it replaces: a session that opens blind and spends its first turns
//! reading `package.json`, `tsconfig.json`, the lint and the build config to
//! find out how to run the tests. Measured on issue #50, the `code` session
//! read `package.json` four times and four more config files twice each — the
//! same bytes re-entering a context that every later turn pays to re-read.
//!
//! **Verbatim, not summarized.** The same call `common::explore` makes for the
//! grounding documents, for the same reason: what a session needs here is the
//! exact name of a vitest project and the exact flags of a script. A paraphrase
//! that loses them sends the session to read the file anyway, and it has now
//! paid twice.
//!
//! **Derived every run, never stored.** The digest is cheap to rebuild — a
//! dozen small files off a checkout already on disk — so there is nothing to
//! cache and, by construction, nothing that can go stale. A kept digest would
//! need an invalidation rule, and a wrong one is worse than no digest at all: a
//! session trusts injected context over the repository, so a digest still
//! naming `npm test` after the repo moved on sends it to run nothing and
//! believe the suite passed.
//!
//! **In `dev_loop/` and not `common/`**: only this workflow starts a session
//! that builds. The refinement gets its repository context from
//! `common::explore`, and `common/` is for what two workflows need today.

use std::path::Path;

use harness_core::ports::shell::disk::Disk;
use harness_core::traces::Logbook;

/// How much a single file may contribute, in characters.
///
/// Per-file as well as total: one generous `package.json` in a monorepo would
/// otherwise spend the whole budget and leave the test and lint config out,
/// which are the two the session actually came for.
pub const FILE_BUDGET: usize = 4_000;

/// How much the whole digest may weigh, in characters (~3k tokens).
///
/// Sized against what it saves: the config files of the measured run came to
/// about 8k characters, and they were re-read enough times to cost more than
/// carrying them once.
pub const BUDGET: usize = 12_000;

/// How many files the digest may name.
pub const MAX_FILES: usize = 12;

/// How deep a match is still configuration for *this* checkout.
///
/// A `package.json` at the root configures the project; the twentieth one, six
/// folders down a monorepo, configures a package this task probably never
/// touches. Depth is a crude proxy, and deliberately so — the alternative is
/// guessing which package the task belongs to, which is the planner's job.
pub const MAX_DEPTH: usize = 2;

const CUT: &str = "\n[... truncated: this file ran over its share of the digest ...]";
const OVER: &str = "\n[... truncated: the configuration digest ran over its budget ...]";
const UNREADABLE: &str = "(tracked, but could not be read)";

/// How a known configuration file is recognised.
enum How {
    /// The file name, exactly.
    Named(&'static str),
    /// The file name begins with this — `vitest.config` catches `.ts` and
    /// `.mts` without naming every extension an ecosystem invents.
    Starting(&'static str),
    /// The path begins with this, for a folder that is itself configuration.
    Under(&'static str),
}

/// The configuration files worth carrying, most telling first.
///
/// **The order is the priority**: a file's rank is the first rule it matches,
/// and the digest fills by rank, so what survives a tight budget is the
/// manifest and the test runner rather than a formatter's preferences.
///
/// Deliberately a table and not a session. Which file is configuration is
/// nearly deterministic, and a list of names is reproducible, free, and cannot
/// name a file that does not exist. An unknown stack falls back to the manifest
/// rules below or, failing those, to the session reading for itself — which is
/// what it does today.
const KNOWN: &[How] = &[
    // The manifest: what the project is, and the scripts that run it.
    How::Named("package.json"),
    How::Named("Cargo.toml"),
    How::Named("pyproject.toml"),
    How::Named("go.mod"),
    How::Named("composer.json"),
    How::Named("Gemfile"),
    How::Named("mix.exs"),
    How::Named("pubspec.yaml"),
    How::Named("deno.json"),
    // How it is driven.
    How::Named("Makefile"),
    How::Named("justfile"),
    How::Named("Taskfile.yml"),
    // The test runner: the one answer a session cannot guess.
    //
    // `vitest.` and not `vitest.config`: the measured repository also carries a
    // `vitest.build.config.mts`, which the narrower rule missed — and a second
    // runner configuration is exactly the thing a session cannot infer.
    How::Starting("vitest."),
    How::Starting("jest.config"),
    How::Starting("playwright.config"),
    How::Starting("pytest.ini"),
    How::Starting("tox.ini"),
    How::Starting("karma.conf"),
    // The compiler and the bundler.
    How::Starting("tsconfig"),
    How::Starting("vite.config"),
    How::Starting("esbuild.config"),
    How::Starting("rollup.config"),
    How::Starting("webpack.config"),
    How::Starting("next.config"),
    How::Starting("svelte.config"),
    How::Starting("nuxt.config"),
    How::Starting("babel.config"),
    How::Named("rust-toolchain.toml"),
    How::Named(".nvmrc"),
    How::Named("CMakeLists.txt"),
    How::Named("pom.xml"),
    How::Starting("build.gradle"),
    // The gates a delivery has to pass.
    How::Under(".github/workflows/"),
    How::Starting("eslint.config"),
    How::Starting(".eslintrc"),
    How::Starting("biome.json"),
    How::Starting("ruff.toml"),
    How::Starting(".golangci"),
    How::Starting("clippy.toml"),
    How::Starting(".prettierrc"),
];

/// Files a rule above would catch and that carry nothing a session can use.
///
/// A lock file is the resolved output of the manifest: tens of thousands of
/// lines that answer no question the manifest did not already answer.
const NEVER: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "Cargo.lock",
    "composer.lock",
    "Gemfile.lock",
    "poetry.lock",
    "go.sum",
];

/// The file name, or the whole path when there is no separator.
fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// How deep the path sits — `0` at the root of the checkout.
fn depth_of(path: &str) -> usize {
    path.matches('/').count()
}

/// Which rule catches this path, if any. The index is the priority.
fn rank_of(path: &str) -> Option<usize> {
    let name = name_of(path);
    if NEVER.contains(&name) {
        return None;
    }
    KNOWN.iter().position(|how| match how {
        How::Named(exact) => name == *exact,
        How::Starting(head) => name.starts_with(head),
        // A folder of configuration is configuration at any depth it is found:
        // `.github/` only ever sits at the root of a repository.
        How::Under(folder) => path.starts_with(folder),
    })
}

/// The configuration files among the tracked ones, by priority then path.
///
/// Pure, and that is what makes it testable without a repository: it reads a
/// list of names and returns a shorter one. Nothing here touches a disk.
///
/// Sorted so the digest is byte-identical from one run to the next on an
/// unchanged checkout — a prompt that reshuffles its own blocks would miss the
/// prompt cache it is here to exploit.
#[must_use]
pub fn detect(tracked: &[String]) -> Vec<String> {
    let mut found: Vec<(usize, &String)> = tracked
        .iter()
        .filter(|path| depth_of(path) <= MAX_DEPTH)
        .filter_map(|path| rank_of(path).map(|rank| (rank, path)))
        .collect();
    found.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    found
        .into_iter()
        .take(MAX_FILES)
        .map(|(_, path)| path.clone())
        .collect()
}

/// The largest char boundary at or below `at`, so a cut never splits a
/// character.
///
/// Slicing a `&str` at an arbitrary byte **panics** when that byte falls inside
/// a multi-byte character, and a configuration file is arbitrary text: one
/// accented comment or one emoji straddling the budget would take the run down
/// after every session of the round was paid for. Floor and not ceiling so the
/// result can never exceed the budget it is enforcing.
fn boundary_at_or_below(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// One file as the session sees it, cut to its share.
fn block(path: &str, text: &str) -> String {
    let trimmed = text.trim();
    let body = if trimmed.len() <= FILE_BUDGET {
        trimmed.to_string()
    } else {
        let cut = boundary_at_or_below(trimmed, FILE_BUDGET);
        format!("{}{CUT}", &trimmed[..cut])
    };
    format!("<file path=\"{path}\">\n{body}\n</file>")
}

/// The detected files, read verbatim, within budget.
///
/// Empty when nothing was detected — an unknown stack is said by saying
/// nothing, and the caller renders the absence. Returning a sentence here
/// would put prose where a prompt expects files.
#[must_use]
pub fn digest(root: &Path, disk: &dyn Disk, paths: &[String], log: Option<&Logbook>) -> String {
    let mut out = String::new();
    let mut left_out = 0;
    for path in paths {
        let text = disk
            .read_to_string(&root.join(path))
            .unwrap_or_else(|| UNREADABLE.to_string());
        let next = block(path, &text);
        // `+ 2` for the blank line that will join it.
        if !out.is_empty() && out.len() + next.len() + 2 > BUDGET {
            left_out += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&next);
    }
    if left_out > 0 {
        if let Some(log) = log {
            log.warn(&format!(
                "the configuration digest left {left_out} file(s) out of its \
                 {BUDGET}-character budget — the session will read them itself \
                 if it needs them"
            ));
        }
        out.push_str(OVER);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use harness_core::domain::Outcome;

    use super::*;

    fn paths(of: &[&str]) -> Vec<String> {
        of.iter().map(|path| (*path).to_string()).collect()
    }

    /// A disk holding the files a test names, under `/repo`.
    struct Held(HashMap<PathBuf, String>);

    impl Held {
        fn of(files: &[(&str, &str)]) -> Self {
            Self(
                files
                    .iter()
                    .map(|(path, text)| (PathBuf::from("/repo").join(path), (*text).to_string()))
                    .collect(),
            )
        }
    }

    impl Disk for Held {
        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.0.get(path).cloned()
        }
        fn exists(&self, _path: &Path) -> bool {
            unreachable!()
        }
        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn dir_names(&self, _path: &Path) -> Vec<String> {
            unreachable!()
        }
        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            unreachable!()
        }
    }

    fn digested(files: &[(&str, &str)]) -> String {
        let held = Held::of(files);
        let found = detect(&paths(
            &files.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
        ));
        digest(Path::new("/repo"), &held, &found, None)
    }

    #[test]
    fn a_file_is_carried_verbatim_under_its_path() {
        let said = digested(&[(
            "package.json",
            "{\n  \"scripts\": { \"test\": \"vitest run\" }\n}",
        )]);
        assert!(said.contains("<file path=\"package.json\">"), "{said}");
        // The exact command survives: a paraphrase would send the session to
        // read the file anyway.
        assert!(said.contains("\"test\": \"vitest run\""), "{said}");
        assert!(said.ends_with("</file>"), "{said}");
    }

    #[test]
    fn an_unknown_stack_digests_to_nothing_at_all() {
        // Said by saying nothing: the caller renders the absence, and a
        // sentence here would put prose where the prompt expects files.
        assert!(digested(&[("main.zig", "pub fn main() void {}")]).is_empty());
    }

    #[test]
    fn a_tracked_file_that_cannot_be_read_is_said_so_not_skipped() {
        // Skipping silently would let a session believe the digest is complete.
        let held = Held::of(&[]);
        let said = digest(Path::new("/repo"), &held, &paths(&["package.json"]), None);
        assert!(said.contains(UNREADABLE), "{said}");
    }

    #[test]
    fn one_huge_file_does_not_spend_the_whole_budget() {
        let said = digested(&[
            ("package.json", &"x".repeat(FILE_BUDGET * 2)),
            ("vitest.config.ts", "export default {}"),
        ]);
        assert!(said.contains(CUT.trim()), "the big one is cut");
        // And the file the session actually came for is still there.
        assert!(said.contains("export default {}"), "{said}");
    }

    #[test]
    fn a_multibyte_character_on_the_budget_does_not_panic() {
        // Slicing a `&str` mid-character panics. A config file is arbitrary
        // text, and this repository's content is French: one accented comment
        // landing on the budget would take the run down after every session of
        // the round was paid for.
        let straddling = format!("{}é{}", "a".repeat(FILE_BUDGET - 1), "b".repeat(200));
        assert!(
            !straddling.is_char_boundary(FILE_BUDGET),
            "the test must actually straddle the budget"
        );
        let said = digested(&[("package.json", &straddling)]);
        assert!(said.contains(CUT.trim()), "it was cut");
        // Cut below the budget, never above: the floor is what keeps the
        // guarantee the budget exists to make.
        assert!(said.contains(&"a".repeat(FILE_BUDGET - 1)));
        assert!(!said.contains('é'), "the split character is dropped whole");
    }

    #[test]
    fn an_emoji_straddling_the_budget_survives_too() {
        // Four bytes rather than two: the loop has to walk back more than once.
        let straddling = format!("{}🚀{}", "a".repeat(FILE_BUDGET - 2), "b".repeat(50));
        let said = digested(&[("package.json", &straddling)]);
        assert!(said.contains(CUT.trim()));
        assert!(!said.contains('🚀'));
    }

    #[test]
    fn going_over_budget_is_stated_in_the_digest() {
        let files: Vec<(String, String)> = (0..6)
            .map(|n| (format!("dir{n}/package.json"), "y".repeat(FILE_BUDGET)))
            .collect();
        let borrowed: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        let said = digested(&borrowed);
        assert!(said.len() <= BUDGET + OVER.len(), "{}", said.len());
        assert!(said.contains(OVER.trim()), "the shortfall is said out loud");
    }

    #[test]
    fn the_manifest_and_the_test_runner_are_found_among_the_source() {
        let found = detect(&paths(&[
            "src/core/vault-service.ts",
            "vitest.config.mts",
            "package.json",
            "README.md",
        ]));
        assert_eq!(found, paths(&["package.json", "vitest.config.mts"]));
    }

    #[test]
    fn the_manifest_comes_before_the_formatter() {
        // The order is the priority, and a tight budget keeps the head: a
        // session can work without knowing the quote style, not without
        // knowing how to run the tests.
        let found = detect(&paths(&[
            "eslint.config.mjs",
            "vitest.config.ts",
            "package.json",
        ]));
        assert_eq!(
            found,
            paths(&["package.json", "vitest.config.ts", "eslint.config.mjs"])
        );
    }

    #[test]
    fn a_lock_file_is_never_carried() {
        // It matches no rule by name, but it sits beside the manifest and is
        // the one file that would swallow the whole budget.
        for lock in NEVER {
            assert!(detect(&paths(&[lock])).is_empty(), "{lock}");
        }
    }

    #[test]
    fn a_config_buried_in_a_monorepo_is_left_to_the_session() {
        let found = detect(&paths(&[
            "package.json",
            "packages/ui/package.json",
            "packages/ui/deep/nested/package.json",
        ]));
        assert_eq!(found, paths(&["package.json", "packages/ui/package.json"]));
    }

    #[test]
    fn the_ci_workflows_are_configuration_wherever_they_sit() {
        // `.github/workflows/ci.yml` is three levels down and still the file
        // that says what a delivery must pass.
        let found = detect(&paths(&[".github/workflows/ci.yml"]));
        assert_eq!(found, paths(&[".github/workflows/ci.yml"]));
    }

    #[test]
    fn an_unknown_stack_detects_nothing_rather_than_guessing() {
        let found = detect(&paths(&["main.zig", "build.zig.zon", "src/thing.zig"]));
        assert!(found.is_empty());
    }

    #[test]
    fn no_more_files_than_the_cap() {
        let many: Vec<String> = (0..40).map(|n| format!("dir{n}/package.json")).collect();
        assert_eq!(detect(&many).len(), MAX_FILES);
    }

    #[test]
    fn the_order_does_not_depend_on_how_git_listed_the_files() {
        // A digest that reshuffled its own blocks would miss the prompt cache
        // it exists to exploit.
        let one = detect(&paths(&[
            "vitest.config.ts",
            "package.json",
            "tsconfig.json",
        ]));
        let other = detect(&paths(&[
            "tsconfig.json",
            "package.json",
            "vitest.config.ts",
        ]));
        assert_eq!(one, other);
    }

    #[test]
    fn the_measured_repository_gives_exactly_the_files_its_session_re_read() {
        // The non-source files tracked by the repository the harness drove on
        // milestone 16, verbatim. The `code` session there read `package.json`
        // four times and four other config files twice each; this pins which
        // of them the digest now carries, and in what order.
        let found = detect(&paths(&[
            ".gitattributes",
            ".github/workflows/ci.yml",
            ".gitignore",
            ".nvmrc",
            ".prettierignore",
            ".prettierrc.json",
            "esbuild.config.mjs",
            "eslint.config.mjs",
            "manifest.json",
            "package-lock.json",
            "package.json",
            "src/core/place.ts",
            "tsconfig.json",
            "vitest.build.config.mts",
            "vitest.config.mts",
        ]));
        assert_eq!(
            found,
            paths(&[
                "package.json",
                "vitest.build.config.mts",
                "vitest.config.mts",
                "tsconfig.json",
                "esbuild.config.mjs",
                ".nvmrc",
                ".github/workflows/ci.yml",
                "eslint.config.mjs",
                ".prettierrc.json",
            ])
        );
        // The lock file is tracked, matches nothing, and would have swallowed
        // the budget whole.
        assert!(!found.contains(&"package-lock.json".to_string()));
    }

    #[test]
    fn several_rust_and_python_stacks_are_recognised() {
        assert_eq!(detect(&paths(&["Cargo.toml"])), paths(&["Cargo.toml"]));
        assert_eq!(
            detect(&paths(&["pyproject.toml"])),
            paths(&["pyproject.toml"])
        );
        assert_eq!(detect(&paths(&["go.mod"])), paths(&["go.mod"]));
    }
}
