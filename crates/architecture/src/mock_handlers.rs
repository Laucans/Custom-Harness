//! The mock server's handlers, one file per unit: what every prompt that may
//! touch them says, written once.
//!
//! The mock-up's server (`mock/src/server.js`) imports `mock/handlers/<id>.js`
//! the first time `<id>` is read or commanded. The two legacy files every
//! branch used to append to, `mock/src/reads.js` and `mock/src/commands.js`,
//! made every catch-up conflict there, and an object literal's later key
//! silently won over an earlier one. `scripts/split-handlers.mjs` turns them
//! into handler files once per tree; the gate `mock-handlers` refuses an id
//! defined twice and, once `mock/handlers/` holds a handler file, a handler
//! added to a legacy file.

/// Where a unit's mock handlers live, and what a session never does with them.
pub const RULE: &str = "A unit's mock handlers live in its own file, \
mock/handlers/<id>.js (its Capability or DataCapability id): `export const reads = \
{ '<id>': async (server, variables) => … }` or `export const commands = { '<id>': \
async (server, { payload, targetId, version, basedOn }) => … }`, a helper several \
files share in mock/handlers/_lib/; the mock server imports it at run time, so it \
needs no rebuild. Never edit another unit's handler file, and never add a handler to \
mock/src/reads.js or mock/src/commands.js (gate mock-handlers).";

/// What the split commit may change: `-s ours` records it as merged, so
/// anything else it carried would be dropped without a word.
const SPLIT_PATHS: &str = "mock/handlers/**, mock/src/reads.js, mock/src/commands.js, \
mock/components/** and mock/pages.json";

/// What a session merging `base` does when the legacy handler files conflict.
///
/// They conflict as deleted there when the base split them. The branch
/// takes the split the way the base did: it checks the split commit carries
/// the split only (one parent, [`SPLIT_PATHS`]) — or stops, since `-s ours`
/// would drop anything else — merges the commit before the split, records
/// the split commit as merged while splitting its own tree (the splitter is
/// deterministic, so that tree is the true merge), then merges the base,
/// whose merge base now holds the handler files the split wrote. A handler
/// the base added to its legacy file before splitting itself (a milestone
/// that migrated) is in no merge base as a file: it conflicts add/add, and
/// the side that changed it from the legacy text at the old merge base wins.
#[must_use]
pub fn on_conflict(base: &str) -> String {
    format!(
        "If mock/src/reads.js or mock/src/commands.js conflict as deleted on `{base}`, \
         the base split its handlers into mock/handlers/. Take the split the way it did \
         instead of resolving them: `git merge --abort`; `TIP=$(git rev-parse HEAD)`; \
         `SPLIT=$(git log -1 --format=%H --diff-filter=D origin/{base} -- \
         mock/src/commands.js mock/src/reads.js)`. Check it first: unless `git rev-list \
         --parents -n 1 \"$SPLIT\"` names exactly one parent and `git diff --name-only \
         \"$SPLIT^\" \"$SPLIT\"` lists only {SPLIT_PATHS}, stop and report it, merging \
         nothing — the next step records SPLIT as merged with `-s ours`, which silently \
         drops anything else it changed. Then merge the commit before it, still on the \
         legacy layout — `git merge \"$SPLIT^\"`; record the split as merged while \
         splitting your own tree — `git merge -s ours --no-commit \"$SPLIT\"`, `git \
         checkout \"$SPLIT\" -- scripts/split-handlers.mjs mock/src/server.js`, `node \
         scripts/split-handlers.mjs` (node 22 or later), `sh mock/build.sh`, then commit; \
         then `git merge origin/{base}`. A handler the split wrote and only one side \
         changed merges by itself. A handler `{base}` added to its legacy file before it \
         split itself is in no merge base as a file, so it conflicts add/add on \
         mock/handlers/<id>.js even when one side alone changed it: compare both sides \
         with <id>'s handler in the legacy file at `git merge-base \"$TIP\" origin/{base}` \
         (`git show <that sha>:mock/src/commands.js`, or reads.js), keep the side that \
         changed it, and merge by hand only when both did. Resolve every other conflict \
         file by file under mock/handlers/, never back into the legacy files, and rebuild \
         the bundles last."
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
    fn a_conflict_on_the_legacy_files_migrates_the_branch_parent_first() {
        let said = on_conflict("milestone/6-media");
        let steps = [
            "conflict as deleted on `milestone/6-media`",
            "git merge --abort",
            "TIP=$(git rev-parse HEAD)",
            "--diff-filter=D origin/milestone/6-media -- mock/src/commands.js mock/src/reads.js)",
            "git rev-list --parents -n 1 \"$SPLIT\"` names exactly one parent",
            "git diff --name-only \"$SPLIT^\" \"$SPLIT\"` lists only mock/handlers/**, mock/src/reads.js, mock/src/commands.js, mock/components/** and mock/pages.json",
            "stop and report it, merging nothing",
            "git merge \"$SPLIT^\"",
            "git merge -s ours --no-commit \"$SPLIT\"",
            "checkout \"$SPLIT\" -- scripts/split-handlers.mjs mock/src/server.js`",
            "node scripts/split-handlers.mjs",
            "sh mock/build.sh",
            "git merge origin/milestone/6-media",
            "conflicts add/add on mock/handlers/<id>.js",
            "git merge-base \"$TIP\" origin/milestone/6-media",
            "keep the side that changed it, and merge by hand only when both did",
            "rebuild the bundles last",
        ];
        let mut from = 0;
        for step in steps {
            let at = said[from..]
                .find(step)
                .unwrap_or_else(|| panic!("{step}, in order: {said}"));
            from += at + step.len();
        }
    }
}
