//! The mock server's handlers, one file per unit: what every prompt that may
//! touch them says, written once.
//!
//! The mock-up's server (`mock/src/server.js`) imports `mock/handlers/<id>.js`
//! the first time `<id>` is read or commanded. The two legacy files every
//! branch used to append to, `mock/src/reads.js` and `mock/src/commands.js`,
//! made every catch-up conflict there, and an object literal's later key
//! silently won over an earlier one. `scripts/split-handlers.mjs` turns them
//! into handler files once per tree — by the migration, and by the
//! harness's catch-up on the side of a merge still on the legacy files; the
//! gate `mock-handlers` refuses an id defined twice and, once
//! `mock/handlers/` holds a handler file, a handler added to a legacy file.

/// Where a unit's mock handlers live, and what a session never does with them.
pub const RULE: &str = "A unit's mock handlers live in its own file, \
mock/handlers/<id>.js (its Capability or DataCapability id): `export const reads = \
{ '<id>': async (server, variables) => … }` or `export const commands = { '<id>': \
async (server, { payload, targetId, version, basedOn }) => … }`, a helper several \
files share in mock/handlers/_lib/; the mock server imports it at run time, so it \
needs no rebuild. Never edit another unit's handler file, and never add a handler to \
mock/src/reads.js or mock/src/commands.js (gate mock-handlers).";

/// The two legacy handler files a split removes.
pub const LEGACY: [&str; 2] = ["mock/src/commands.js", "mock/src/reads.js"];

/// The folder of the handler files, `_lib/` included.
pub const HANDLERS: &str = "mock/handlers/";

/// The script that splits a tree's legacy files into handler files,
/// deterministic: two trees with the same legacy files split the same.
pub const SPLITTER: &str = "scripts/split-handlers.mjs";

/// What a split takes from the tree that already split, before it runs:
/// the splitter itself, and the server it refuses to split without — it
/// stops while `mock/src/server.js` still imports a legacy file.
pub const SPLITTER_INPUTS: [&str; 2] = [SPLITTER, "mock/src/server.js"];

/// What a split commit stages, and nothing else: the handler files and the
/// legacy files they replace — never the inputs taken for the run.
pub const SPLIT_STAGED: [&str; 3] = ["mock/handlers", LEGACY[0], LEGACY[1]];

/// The words the splitter refuses an old node with: the toolchain's fault,
/// never the tree's.
pub const SPLITTER_TOO_OLD: &str = "or later is needed";

/// Whether `path` is one of the two legacy handler files.
#[must_use]
pub fn is_legacy(path: &str) -> bool {
    LEGACY.contains(&path)
}

/// Whether `path` is a handler file: a `.js` under [`HANDLERS`], a shared
/// helper of `_lib/` included, its README not.
#[must_use]
pub fn is_handler_file(path: &str) -> bool {
    // The server imports `<id>.js` exactly: an extension in another case
    // is no handler of it.
    path.strip_prefix(HANDLERS).is_some_and(|rest| {
        std::path::Path::new(rest)
            .extension()
            .is_some_and(|extension| extension == "js")
    })
}

/// What a session merging `base` does when the legacy handler files conflict.
///
/// They conflict as deleted there when the base split them. The branch
/// takes the split the way the harness's catch-up does
/// (`workflows::common::handler_split`): it splits the merge base apart
/// (the base's splitter and server taken for the run, committed nowhere it
/// keeps), splits its own tree the same way and commits the split files
/// only, then merges the base. Each handler both sides added since the
/// merge base — the split wrote it on both — merges three ways on the
/// merge base's own split with `git merge-file`; only what that leaves
/// conflicted is resolved by hand, under `mock/handlers/`.
#[must_use]
pub fn on_conflict(base: &str) -> String {
    let [splitter, server] = SPLITTER_INPUTS;
    let staged = SPLIT_STAGED.join(" ");
    format!(
        "If mock/src/reads.js or mock/src/commands.js conflict as deleted on `{base}`, \
         the base split its handlers into mock/handlers/. Take the split the way the \
         harness does instead of resolving them: `git merge --abort`, then commit or \
         `git stash` every change of your own still uncommitted — the steps below run \
         `git checkout --force` and `git reset --hard`, which drop it; `BRANCH=$(git \
         branch --show-current)`; `MB=$(git merge-base HEAD origin/{base})`. Split the \
         merge base apart: `git checkout --detach \"$MB\"`, `git show \
         origin/{base}:{splitter} > {splitter}`, `git show origin/{base}:{server} > \
         {server}`, `node {splitter}` (node 22 or later), `git add -A`, `git commit -m \
         'split of the merge base'`, `SPLIT_BASE=$(git rev-parse HEAD)` — if the \
         splitter refuses there, `SPLIT_BASE=` (empty). Split your own tree the same \
         way: `git checkout --force \"$BRANCH\"`, the same two `git show` lines, `node \
         {splitter}`, then commit the split alone — `git add -A -- {staged}`, `git \
         commit -m 'chore(mock): split the legacy handler files into \
         mock/handlers/<id>.js'` — and put the two inputs back: `git reset --hard`, \
         `git clean -f -- {splitter}`. If it refuses on your tree, stop and report its \
         message, merging nothing. Then `git merge origin/{base}`. A handler file both \
         sides added since the merge base conflicts add/add: `S=$(mktemp -d)` once — a \
         folder of your own, never a shared path another session writes too — then for \
         each such mock/handlers/<path>, `git show :2:<path> > \"$S/ours\"`, `git show \
         :3:<path> > \"$S/theirs\"`, `git show \"$SPLIT_BASE:<path>\" > \"$S/base\"` (an \
         empty file when it fails), `git merge-file -p \"$S/ours\" \"$S/base\" \
         \"$S/theirs\" > <path>`: exit 0 \
         is the merge — `git add <path>`; otherwise resolve what it left in that file by \
         hand. Resolve every other conflict file by file under mock/handlers/, never \
         back into the legacy files, and rebuild the bundles last (`sh mock/build.sh`)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_names_the_file_the_shapes_and_what_is_never_done() {
        for part in [
            "mock/handlers/<id>.js",
            "export const reads = { '<id>'",
            "export const commands = { '<id>'",
            "mock/handlers/_lib/",
            "Never edit another unit's handler file",
            "never add a handler to mock/src/reads.js or mock/src/commands.js",
            "gate mock-handlers",
        ] {
            assert!(RULE.contains(part), "{part}: {RULE}");
        }
    }

    #[test]
    fn a_conflict_on_the_legacy_files_splits_both_sides_and_merges_three_ways() {
        let said = on_conflict("milestone/6-media");
        let steps = [
            "conflict as deleted on `milestone/6-media`",
            "git merge --abort",
            "commit or `git stash` every change of your own still uncommitted",
            "`git checkout --force` and `git reset --hard`, which drop it",
            "BRANCH=$(git branch --show-current)",
            "MB=$(git merge-base HEAD origin/milestone/6-media)",
            "git checkout --detach \"$MB\"",
            "git show origin/milestone/6-media:scripts/split-handlers.mjs > scripts/split-handlers.mjs",
            "git show origin/milestone/6-media:mock/src/server.js > mock/src/server.js",
            "node scripts/split-handlers.mjs` (node 22 or later)",
            "SPLIT_BASE=$(git rev-parse HEAD)",
            "SPLIT_BASE=` (empty)",
            "git checkout --force \"$BRANCH\"",
            "git add -A -- mock/handlers mock/src/commands.js mock/src/reads.js",
            "git reset --hard",
            "git clean -f -- scripts/split-handlers.mjs",
            "stop and report its message, merging nothing",
            "git merge origin/milestone/6-media",
            "S=$(mktemp -d)",
            "git show :2:<path> > \"$S/ours\"",
            "git show :3:<path> > \"$S/theirs\"",
            "git show \"$SPLIT_BASE:<path>\" > \"$S/base\"",
            "git merge-file -p \"$S/ours\" \"$S/base\" \"$S/theirs\" > <path>",
            "exit 0 is the merge",
            "never back into the legacy files",
            "rebuild the bundles last",
        ];
        let mut from = 0;
        for step in steps {
            let at = said[from..]
                .find(step)
                .unwrap_or_else(|| panic!("{step}, in order: {said}"));
            from += at + step.len();
        }
        assert!(!said.contains("-s ours"), "one strategy, the harness's");
        assert!(!said.contains("/tmp/"), "no path shared across sessions");
    }

    #[test]
    fn the_legacy_files_and_the_handler_files_are_told_apart() {
        assert!(is_legacy("mock/src/commands.js") && is_legacy("mock/src/reads.js"));
        assert!(!is_legacy("mock/src/server.js") && !is_legacy("mock/handlers/reads.js"));
        for handler in [
            "mock/handlers/campaign.saveEntity.js",
            "mock/handlers/_lib/commands.js",
            "mock/handlers/a.js",
        ] {
            assert!(is_handler_file(handler), "{handler}");
        }
        for other in [
            "mock/handlers/README.md",
            "mock/handlers/.js",
            "mock/handlers/x.json",
            "mock/handlersX/a.js",
            "mock/src/commands.js",
            "mock/handlers/",
        ] {
            assert!(!is_handler_file(other), "{other}");
        }
        assert_eq!(SPLITTER_INPUTS[0], SPLITTER);
        assert!(
            SPLIT_STAGED
                .iter()
                .all(|path| !SPLITTER_INPUTS.contains(path))
        );
    }

    proptest::proptest! {
        #[test]
        fn a_handler_file_is_a_js_under_the_folder_and_nothing_else(
            name in "[A-Za-z_][A-Za-z0-9_.]{0,20}",
            dir in proptest::option::of("_lib|sub"),
            ext in "js|json|md|mjs",
            folder in "mock/handlers|mock/handlersX|mock/src|handlers",
        ) {
            let path = dir.as_ref().map_or_else(
                || format!("{folder}/{name}.{ext}"),
                |dir| format!("{folder}/{dir}/{name}.{ext}"),
            );
            proptest::prop_assert_eq!(
                is_handler_file(&path),
                folder == "mock/handlers" && ext == "js"
            );
            proptest::prop_assert!(!is_legacy(&path) || path.starts_with("mock/src/"));
        }
    }
}
