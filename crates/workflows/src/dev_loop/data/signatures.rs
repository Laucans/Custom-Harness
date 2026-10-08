//! The shape of the code a task builds on: public signatures, without bodies.
//!
//! What it is **for**: sparing the session the need to open a whole source file
//! just to learn the API it must call.
//!
//! # What it actually did, measured
//!
//! One A/B pair on the `code` stage of issue #65 — same task, same model, the
//! index on in one arm and `--no-signatures` in the other. **It does not reduce
//! file reading. It changes which files get opened whole.**
//!
//! | characters read from files | with | without | |
//! | --- | --- | --- | --- |
//! | pre-existing code — what this targets | 173 253 | 189 854 | **−8.7%** |
//! | the session's own freshly written code | 85 119 | 65 088 | +31% |
//! | total | 269 920 | 264 127 | **+2.2%** |
//!
//! So the mechanism works on the material it aims at, and the total goes the
//! other way because the session spends more of its budget re-reading code **it
//! has just written** — which an index of git-tracked files can never carry. The
//! sharpest single case: `pack-runner.ts`, in the carried block, was never opened
//! whole with the index (7 targeted reads) and opened whole three times without
//! it.
//!
//! Three limits on that reading, and they matter more than the numbers:
//!
//! 1. **n = 1 against n = 1.** Two sessions, not a distribution. The same task
//!    measured twice has varied by ×2.4 before (`observations.md`).
//! 2. **The arms are not symmetric.** The control ran `--stages code` only, so
//!    there is no `technical-refinement` or `create-test` to compare — and
//!    `technical-refinement` is the stage where this index is structurally blind
//!    anyway, since the plan that names the files is what that stage writes.
//! 3. **Whole-file counts flip with the definition.** Neither arm used `cat`
//!    much; both paged files through `awk '{print NR": "$0}'` and `sed 's/^/|/'`.
//!    Counting only `cat`/`Read`: 11 with, 17 without. Counting those pagers too:
//!    29 with, 25 without. The character figures above are the honest summary
//!    because they do not depend on that call.
//!
//! The stage was also 45 turns and $1.34 cheaper with the index — a gap far
//! larger than a 2% difference in characters read, and therefore **not**
//! something these two runs let anyone attribute to file reading.
//!
//! # Why each language gets a different mechanism
//!
//! Measured, not assumed, and the answer is syntactic:
//!
//! - **Rust**: extracting text works — `domain/doctor.rs` goes from 7 382
//!   characters to 232, variants included. A `pub struct` or `pub enum` block
//!   holds only fields and variants; the methods live in a separate `impl`. So
//!   keeping a block keeps the shape and drops the bodies.
//! - **TypeScript**: the same approach fails. A `class` body holds its method
//!   **bodies** inline, so keeping the block keeps the implementation —
//!   `obsidian-double.ts` only went from 9 235 characters to 7 064, 23%, which
//!   buys nothing. Stripping those bodies by pattern means writing a TypeScript
//!   parser that works on the file it was tested against and breaks on the next
//!   one. `tsc --emitDeclarationOnly` already does it, by the compiler, in under
//!   two seconds, and it resolves inferred return types a text pass cannot see.
//!
//! So Rust is handled here, purely. TypeScript needs a subprocess, which is the
//! launcher's job — this module only says *which* ecosystem a checkout is.
//!
//! # What it is not
//!
//! Not a code graph, and not a map of the repository. It answers "what can I
//! call", for files the task already names. "Who calls this" and "where does
//! this live" are different questions, and the session's own `grep` answers
//! them.
//!
//! # Two known gaps in the Rust path
//!
//! Both **latent**: the only target measured so far is TypeScript, which goes
//! through `tsc` and has neither problem. Named here rather than left to be
//! rediscovered, because the prompt template promises a session more than this
//! half delivers.
//!
//! - **A signature spanning several lines keeps only its first line.**
//!   `signature_of` cuts at `" {"` on one line, so
//!   `pub fn extra_for(\n  a: &str,\n) -> String` is carried as `pub fn
//!   extra_for(` — the arguments, which are the point, are gone. 22 of this
//!   workspace's 311 public functions (7%) are written that way. The emitted
//!   fragment is at least visibly broken, so a session re-opens the file; the
//!   cost is a wasted slice of the budget, not a wrong call.
//! - **`pub const NAME: T = …;` is dropped entirely.** In `opens`,
//!   `trim_start_matches("const ")` runs *before* the loop that tests for
//!   `"const"`, so the word compared is the identifier. The strip is there to
//!   let `pub const fn` through, which it does; a bare `pub const` falls to
//!   `None`. `doctor::MISSING_DEPENDENCIES` is a real example — a marker other
//!   modules match on, absent from its own file's shape.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The toolchain a checkout is built with, as far as its manifests say.
///
/// Three cases on purpose: the third is not a failure. An ecosystem nobody
/// taught this module is handled by injecting nothing, which puts the session
/// exactly where it was before — reading for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ecosystem {
    /// Signatures come from `tsc --emitDeclarationOnly`, run by the launcher.
    TypeScript,
    /// Signatures are extracted here, from the source text.
    Rust,
    /// Nothing is injected, and the session reads as it always did.
    #[default]
    Unknown,
}

/// Which ecosystem these tracked files describe.
///
/// Pure: it compares a list of paths against known manifest names. By suffix,
/// not by file name — so a root `vendor-package.json` would decide, which no
/// checkout has yet had. Rust is tested before TypeScript because a Rust
/// repository may carry a `package.json` for tooling while a TypeScript one
/// never carries a `Cargo.toml` — so the rarer, more decisive marker wins.
///
/// Shallow only, at [`stack::MAX_DEPTH`](super::stack::MAX_DEPTH) separators —
/// today 2, so the root and **two** levels down, which is what
/// `crates/core/Cargo.toml` needs. A `Cargo.toml` six folders deep is a vendored
/// dependency, not what this checkout is.
#[must_use]
pub fn detect(tracked: &[String]) -> Ecosystem {
    let shallow = |name: &str| {
        tracked.iter().any(|path| {
            path.matches('/').count() <= super::stack::MAX_DEPTH && path.ends_with(name)
        })
    };
    if shallow("Cargo.toml") {
        return Ecosystem::Rust;
    }
    if shallow("tsconfig.json") || shallow("package.json") {
        return Ecosystem::TypeScript;
    }
    Ecosystem::Unknown
}

/// The extension a file of this ecosystem carries, for picking which to index.
#[must_use]
pub const fn source_suffix(of: Ecosystem) -> &'static str {
    match of {
        Ecosystem::Rust => ".rs",
        Ecosystem::TypeScript => ".ts",
        Ecosystem::Unknown => "",
    }
}

/// Lines that carry no signature and only cost tokens.
fn is_noise(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("//")
        || trimmed.starts_with("#[")
        || trimmed.is_empty()
}

/// Whether this line opens a public item, and whether its block is its shape.
///
/// `Some(true)` means the braces that follow **are** the signature — a struct's
/// fields, an enum's variants. `Some(false)` means the signature ends at the
/// brace, and what follows is a body to drop.
fn opens(line: &str) -> Option<bool> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("pub(crate) ").or_else(|| {
        trimmed
            .strip_prefix("pub ")
            .or_else(|| trimmed.strip_prefix("pub(super) "))
    })?;
    let word = rest
        .trim_start_matches("async ")
        .trim_start_matches("const ")
        .trim_start_matches("unsafe ")
        .trim_start_matches("extern ");
    for kind in ["struct", "enum", "union"] {
        if word.starts_with(kind) {
            // `pub struct Wrapper(String);` is a line, not a block.
            return Some(line.contains('{'));
        }
    }
    for kind in ["fn", "trait", "type", "const", "static", "mod", "use"] {
        if word.starts_with(kind) {
            return Some(false);
        }
    }
    None
}

/// The signature line alone: everything before the body or the terminator.
fn signature_of(line: &str) -> String {
    let cut = line.find(" {").unwrap_or(line.len());
    line[..cut].trim_end().trim_end_matches(';').to_string()
}

/// How far into the braces this line leaves us.
fn depth_after(line: &str, depth: i32) -> i32 {
    depth + i32::try_from(line.matches('{').count()).unwrap_or(0)
        - i32::try_from(line.matches('}').count()).unwrap_or(0)
}

/// The public shape of one Rust file: items, fields, variants, no bodies.
///
/// `impl` blocks are kept as their header plus the public methods inside, since
/// a method is where most of a type's API actually lives — an index that listed
/// `pub struct Workspace` and none of its methods would answer nothing.
///
/// Deliberately a scan and not a parse. It can be fooled — a brace inside a
/// string literal in a signature line would miscount — and that is the accepted
/// price of no dependency: the failure mode is a slightly wrong index, never a
/// panic and never a wrong compilation, because nothing here is compiled.
#[must_use]
pub fn rust_shape(source: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if is_noise(line) {
            continue;
        }
        let trimmed = line.trim_start();
        // An `impl` header, public or not: what it carries may be public.
        if trimmed.starts_with("impl ") || trimmed.starts_with("impl<") {
            let mut inner: Vec<String> = Vec::new();
            let mut depth = depth_after(line, 0);
            while depth > 0 {
                let Some(next) = lines.next() else { break };
                depth = depth_after(next, depth);
                if !is_noise(next) && opens(next) == Some(false) {
                    inner.push(format!("    {}", signature_of(next).trim()));
                }
            }
            // An impl with nothing public in it says nothing worth paying for.
            if !inner.is_empty() {
                // With its brace: the block has to read as Rust, since that is
                // what the session is about to pattern-match against.
                out.push(format!("{} {{", signature_of(line)));
                out.extend(inner);
                out.push("}".to_string());
            }
            continue;
        }
        match opens(line) {
            Some(true) => {
                // An enum's variants are public with it; a struct's fields are
                // only API when they say `pub`. Keeping private ones would both
                // cost tokens and mislead — a session could take `root` for
                // something it may set.
                let fields_only_if_public = trimmed.contains("struct ");
                let mut depth = depth_after(line, 0);
                let mut kept: Vec<String> = Vec::new();
                let mut hidden = 0;
                while depth > 0 {
                    let Some(next) = lines.next() else { break };
                    depth = depth_after(next, depth);
                    if is_noise(next) {
                        continue;
                    }
                    let inner = next.trim();
                    if depth == 0 && inner == "}" {
                        break;
                    }
                    if fields_only_if_public && !inner.starts_with("pub ") {
                        hidden += 1;
                        continue;
                    }
                    kept.push(next.trim_end().to_string());
                }
                out.push(line.trim_end().to_string());
                out.extend(kept);
                if hidden > 0 {
                    // Said rather than left out: a struct that looks fieldless
                    // reads as one you can build with `Self {}`.
                    out.push(format!("    /* {hidden} private field(s) */"));
                }
                out.push("}".to_string());
            }
            Some(false) => out.push(signature_of(line)),
            None => {}
        }
    }
    out.join("\n")
}

/// How much the carried signatures may weigh, in characters (~3k tokens at the
/// conventional four characters per token — not a tokenizer's answer).
///
/// Not the whole index: that measured 60 000 characters of `.d.ts` on the target
/// checkout, and a block that size is re-read on every turn. What pays is the
/// handful a task names.
///
/// **Raised from 6 000 after measuring it starve.** On #63 the budget cut in:
/// `place-scenes.ts` came to 3 157 characters and `src/core/index.ts` to 3 194,
/// so the two together did not fit and the prompt carried one of them. The issue
/// body had named `src/core/index.ts` three times, in full, and the session read
/// it anyway. A budget that drops what the task explicitly asked for is not a
/// budget, it is a bug.
///
/// **It is still not enough, and that is measured too.** Each entry costs
/// `35 + path + shape` characters plus a separator, so 12 000 holds **three**
/// files of ~3 200 — a fourth needs about 13 100. On #65's `code` stage the
/// carried block ran to 11 347 characters over 12 small files and still ended on
/// `the index hit its budget`, with two files the task named left out. Whether
/// raising it again pays is open: the same measurement found the index does not
/// reduce total file reading (see the module doc), so a bigger block is a bigger
/// certain cost against an uncertain saving.
pub const BUDGET: usize = 12_000;

/// One file's index, as the prompt carries it.
#[must_use]
pub fn block(path: &str, shape: &str) -> String {
    format!("<signatures path=\"{path}\">\n{shape}\n</signatures>")
}

/// Every file's public shape, with what the checkout is and what it tracks.
///
/// Built once per run — the TypeScript half costs a `tsc` subprocess, and a
/// second one per turn would pay for the same answer again. The *selection* is
/// per task and pure: [`carried`] picks out of this.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// What the checkout is built with.
    pub ecosystem: Ecosystem,
    /// Public shape by source path, for every file that had one.
    pub by_path: BTreeMap<String, String>,
    /// The checkout's tracked files, so a path in a body can be recognised.
    pub tracked: Vec<String>,
}

impl Index {
    /// Whether there is anything to carry at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

/// The signatures for the files this body names, within budget.
///
/// Pure, and the reason the index is split from its use: picking is a decision
/// about one task, made at prompt time, and it must not re-run a compiler.
///
/// Empty when the body names nothing indexed — the normal case for a task that
/// creates files rather than extending them, and nothing is said about it: an
/// absent block asks the session for no gesture.
#[must_use]
pub fn carried(index: &Index, body: &str, budget: usize) -> String {
    if index.is_empty() {
        return String::new();
    }
    let suffix = source_suffix(index.ecosystem);
    let mut out = String::new();
    let mut left_out = 0;
    for path in paths_named_in(body, &index.tracked, suffix) {
        let Some(shape) = index.by_path.get(&path) else {
            continue;
        };
        if shape.trim().is_empty() {
            continue;
        }
        let next = block(&path, shape);
        if !out.is_empty() && out.len() + next.len() + 2 > budget {
            left_out += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&next);
    }
    if left_out > 0 {
        let _ = write!(
            out,
            "\n[... {left_out} more file(s) this task names are not listed: the \
             index hit its budget. Read them yourself if you need them ...]"
        );
    }
    out
}

/// The paths a task body names, in the order they first appear.
///
/// The Technical Implementation Plan lists every file it means to touch — this
/// reads them back out so the index covers what the task is about rather than
/// the whole repository, which measured 60 000 characters of `.d.ts` on the
/// target checkout and would be a block nobody can afford on every turn.
///
/// Deduplicated, and only paths that look like a tracked source file: the body
/// is prose, and `src/core/` or `package.json` appear in it as often as real
/// file names do.
///
/// # Three forms, because a plan does not write full paths
///
/// Measured on #63, which named every file the session then read, in three
/// spellings an exact match does not all catch: `src/core/index.ts` (the tracked
/// path), `core/testing/vault-store-contract.ts` (the `src/` left off), and
/// `pnj-actor.ts` / `vault-store.ts` (bare names). Requiring the full path
/// resolved only the first kind, and the session opened the rest itself.
///
/// So a token resolves if it is the tracked path, a tail of it on a segment
/// boundary, or a **unique** file name. Unique is what keeps this honest: when
/// more than one directory holds an `index.ts`, guessing which one a plan meant
/// would carry the wrong file's signatures — worse than carrying none, if the
/// session trusts it.
///
/// **A fourth form it does not handle**: a bare import specifier such as
/// `../vault-store`, which carries no suffix and so matches no tracked path.
/// Left unhandled on purpose — resolving module specifiers means resolving
/// `tsconfig` path aliases, and a plan that names a file usually names it.
#[must_use]
pub fn paths_named_in(body: &str, tracked: &[String], suffix: &str) -> Vec<String> {
    if suffix.is_empty() {
        return Vec::new();
    }
    let source: Vec<&String> = tracked
        .iter()
        .filter(|path| path.ends_with(suffix))
        .collect();
    // A file name that more than one tracked file answers to resolves to none.
    let mut by_name: BTreeMap<&str, Option<&String>> = BTreeMap::new();
    for path in &source {
        let name = path.rsplit('/').next().unwrap_or(path);
        by_name
            .entry(name)
            .and_modify(|slot| *slot = None)
            .or_insert(Some(path));
    }
    let resolve = |token: &str| -> Option<&String> {
        if token.is_empty() {
            return None;
        }
        if let Some(exact) = source.iter().find(|path| path.as_str() == token) {
            return Some(exact);
        }
        // A tail, on a segment boundary: `core/a.ts` resolves `src/core/a.ts`,
        // and never `my-core/a.ts`. Unique too, and for the same reason as the
        // file name below — `/index.ts` is the tail of half the repository, and
        // taking the first match would carry a file the plan never meant.
        let tail = format!("/{token}");
        let mut ending = source.iter().filter(|path| path.ends_with(&tail));
        if let Some(only) = ending.next()
            && ending.next().is_none()
        {
            return Some(only);
        }
        by_name.get(token).copied().flatten()
    };
    let mut found: Vec<String> = Vec::new();
    for word in body.split(|c: char| c.is_whitespace() || "`\"'(),;*[]".contains(c)) {
        let cleaned = word.trim_matches(|c: char| c == '.' || c == ':');
        if let Some(path) = resolve(cleaned)
            && !found.contains(path)
        {
            found.push(path.clone());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(of: &[&str]) -> Vec<String> {
        of.iter().map(|path| (*path).to_string()).collect()
    }

    #[test]
    fn a_cargo_manifest_says_rust() {
        assert_eq!(
            detect(&paths(&["Cargo.toml", "src/main.rs"])),
            Ecosystem::Rust
        );
    }

    #[test]
    fn a_tsconfig_says_typescript() {
        assert_eq!(
            detect(&paths(&["package.json", "tsconfig.json", "src/a.ts"])),
            Ecosystem::TypeScript
        );
    }

    #[test]
    fn rust_wins_over_a_package_json_kept_for_tooling() {
        // A Rust repository may carry a `package.json` for a docs site or a
        // commit hook; a TypeScript one never carries a `Cargo.toml`. The rarer
        // marker is the decisive one.
        assert_eq!(
            detect(&paths(&["Cargo.toml", "package.json"])),
            Ecosystem::Rust
        );
    }

    #[test]
    fn an_unknown_stack_is_a_case_not_a_failure() {
        // Injecting nothing puts the session exactly where it was before.
        assert_eq!(
            detect(&paths(&["main.zig", "build.zig"])),
            Ecosystem::Unknown
        );
        assert_eq!(source_suffix(Ecosystem::Unknown), "");
    }

    #[test]
    fn a_vendored_manifest_deep_in_the_tree_does_not_decide() {
        assert_eq!(
            detect(&paths(&["vendor/some/crate/Cargo.toml", "src/a.ts"])),
            Ecosystem::Unknown,
            "nothing shallow enough to say what this checkout is"
        );
    }

    #[test]
    fn an_enum_keeps_its_variants_because_they_are_its_signature() {
        // The regression this module exists for: a line-only extractor gave
        // `pub enum Repair` with no variants, which answers nothing.
        let shape = rust_shape(
            "/// A repair.\n\
             #[derive(Debug)]\n\
             pub enum Repair {\n\
             \x20   Nothing,\n\
             \x20   CleanWorkspace,\n\
             }\n",
        );
        assert!(shape.contains("pub enum Repair {"), "{shape}");
        assert!(shape.contains("Nothing,"), "{shape}");
        assert!(shape.contains("CleanWorkspace,"), "{shape}");
    }

    #[test]
    fn a_struct_keeps_its_public_fields() {
        let shape = rust_shape(
            "pub struct Scope {\n\
             \x20   /// The task.\n\
             \x20   pub task: Named,\n\
             \x20   pub siblings: Vec<Sibling>,\n\
             }\n",
        );
        assert!(shape.contains("pub task: Named,"), "{shape}");
        assert!(shape.contains("pub siblings: Vec<Sibling>,"), "{shape}");
        // The doc comment inside is noise once the shape is all that is wanted.
        assert!(!shape.contains("The task."), "{shape}");
        assert!(shape.ends_with('}'), "it reads as Rust: {shape}");
    }

    #[test]
    fn a_struct_hides_its_private_fields_but_counts_them() {
        // Keeping them cost tokens and misled: a session could take `root` for
        // something it may set. Saying how many is what stops the struct from
        // reading as one you can build with `Self {}`.
        let shape = rust_shape(
            "pub struct Workspace {\n\
             \x20   root: PathBuf,\n\
             \x20   state_root: PathBuf,\n\
             \x20   pub keep: bool,\n\
             }\n",
        );
        assert!(shape.contains("pub keep: bool,"), "{shape}");
        assert!(!shape.contains("root: PathBuf"), "{shape}");
        assert!(shape.contains("2 private field(s)"), "{shape}");
    }

    #[test]
    fn an_impl_block_keeps_its_brace_so_it_reads_as_rust() {
        let shape = rust_shape("impl Strategy {\n    pub fn parse() -> u8 { 1 }\n}\n");
        assert!(shape.starts_with("impl Strategy {"), "{shape}");
        assert!(shape.ends_with('}'), "{shape}");
    }

    #[test]
    fn a_function_loses_its_body_and_keeps_its_signature() {
        let shape = rust_shape(
            "pub fn repair_for(kind: &str, reason: &str) -> Repair {\n\
             \x20   let secret = 42;\n\
             \x20   Repair::Nothing\n\
             }\n",
        );
        assert_eq!(
            shape,
            "pub fn repair_for(kind: &str, reason: &str) -> Repair"
        );
        assert!(!shape.contains("42"), "no body leaks");
    }

    #[test]
    fn an_impl_keeps_its_header_and_its_public_methods() {
        // Most of a type's API lives in its methods: an index naming the struct
        // and none of them would answer nothing.
        let shape = rust_shape(
            "impl Workspace {\n\
             \x20   /// The logs.\n\
             \x20   pub fn logs(&self) -> PathBuf {\n\
             \x20       self.root.join(LOGS)\n\
             \x20   }\n\
             \x20   fn private(&self) {}\n\
             }\n",
        );
        assert!(shape.contains("impl Workspace"), "{shape}");
        assert!(shape.contains("pub fn logs(&self) -> PathBuf"), "{shape}");
        assert!(!shape.contains("self.root.join"), "no body leaks");
        assert!(
            !shape.contains("private"),
            "private methods are not the API"
        );
    }

    #[test]
    fn an_impl_with_nothing_public_is_not_carried() {
        let shape = rust_shape("impl Thing {\n    fn hidden(&self) {}\n}\n");
        assert!(shape.is_empty(), "{shape}");
    }

    #[test]
    fn a_private_item_is_never_carried() {
        let shape = rust_shape("fn helper() -> u8 {\n    7\n}\nstruct Hidden {\n    a: u8,\n}\n");
        assert!(shape.is_empty(), "{shape}");
    }

    #[test]
    fn a_tuple_struct_is_a_line_not_a_block() {
        let shape = rust_shape("pub struct Wrapper(String);\npub fn next() -> u8 { 1 }\n");
        assert!(shape.contains("pub struct Wrapper(String)"), "{shape}");
        assert!(shape.contains("pub fn next() -> u8"), "{shape}");
    }

    #[test]
    fn the_shape_is_a_fraction_of_the_source() {
        // A synthetic input, and the assertion says so: a >10x ratio on doc
        // lines plus one function. It does **not** measure `domain/doctor.rs` —
        // the comment here used to claim it did, and quoted a figure (277) that
        // the code has never produced for that file. The real pair is in the
        // module doc, and `real_checkouts` below is where this file touches the
        // actual repository.
        let source = "/// Doc line that costs tokens and says nothing callable.\n".repeat(40)
            + "pub fn kept() -> u8 {\n    let mut total = 0;\n    total += 1;\n    total\n}\n";
        let shape = rust_shape(&source);
        assert_eq!(shape, "pub fn kept() -> u8");
        assert!(shape.len() * 10 < source.len(), "a fraction, not a trim");
    }

    #[test]
    fn only_the_files_the_body_names_are_indexed() {
        let tracked = paths(&["src/core/place.ts", "src/core/pnj.ts", "src/other.ts"]);
        let body = "Edit `src/core/place.ts` and add a field. Do not touch src/other.ts yet.";
        let found = paths_named_in(body, &tracked, ".ts");
        assert_eq!(found, paths(&["src/core/place.ts", "src/other.ts"]));
        assert!(!found.contains(&"src/core/pnj.ts".to_string()));
    }

    #[test]
    fn a_path_is_named_once_however_often_the_body_repeats_it() {
        let tracked = paths(&["src/a.ts"]);
        let body = "src/a.ts, then src/a.ts again, and `src/a.ts`.";
        assert_eq!(paths_named_in(body, &tracked, ".ts").len(), 1);
    }

    #[test]
    fn the_four_spellings_63_actually_used_all_resolve() {
        // Every one of these appeared in #63's body, and the exact-match rule
        // caught only the first. The session read the other three itself.
        let tracked = paths(&[
            "src/core/index.ts",
            "src/core/testing/vault-store-contract.ts",
            "src/core/foundry/pnj-actor.ts",
            "src/core/vault-store.ts",
        ]);
        let body = "Export from `src/core/index.ts`. Honour \
                    core/testing/vault-store-contract.ts. Mirror pnj-actor.ts, \
                    and the port lives in vault-store.ts.";
        let found = paths_named_in(body, &tracked, ".ts");
        assert_eq!(found.len(), 4, "{found:?}");
        for path in &tracked {
            assert!(found.contains(path), "{path} missed");
        }
    }

    #[test]
    fn a_tail_only_matches_on_a_segment_boundary() {
        // `core/a.ts` must not resolve `my-core/a.ts`: a near-miss carries the
        // wrong file's signatures, which is worse than carrying none because the
        // session trusts what it is handed.
        let tracked = paths(&["src/my-core/a.ts"]);
        assert_eq!(
            paths_named_in("see core/a.ts", &tracked, ".ts"),
            [] as [std::string::String; 0]
        );
        let right = paths(&["src/core/a.ts"]);
        assert_eq!(
            paths_named_in("see core/a.ts", &right, ".ts"),
            paths(&["src/core/a.ts"])
        );
    }

    #[test]
    fn an_ambiguous_file_name_resolves_to_nothing() {
        // Half a dozen directories hold an `index.ts`. Guessing which one a plan
        // meant would hand the session the wrong file.
        let tracked = paths(&["src/core/index.ts", "src/plugin/index.ts"]);
        assert!(
            paths_named_in("re-export from index.ts", &tracked, ".ts").is_empty(),
            "a bare ambiguous name must resolve to none"
        );
        // Named in full, it resolves — the ambiguity was in the spelling, not
        // in the repository.
        assert_eq!(
            paths_named_in("re-export from src/core/index.ts", &tracked, ".ts"),
            paths(&["src/core/index.ts"])
        );
    }

    #[test]
    fn the_budget_fits_the_files_a_real_task_names() {
        // The regression that made the first measurement worthless: 6 000
        // characters could not hold `place-scenes.ts` (3 157) and
        // `src/core/index.ts` (3 194) together, so a file the body named three
        // times in full was dropped and the session read it anyway.
        let mut index = indexed(&[
            ("src/core/foundry/place-scenes.ts", &"x".repeat(3_157)),
            ("src/core/index.ts", &"y".repeat(3_194)),
        ]);
        // The real case was TypeScript, and the suffix is what picks the files.
        index.ecosystem = Ecosystem::TypeScript;
        let said = carried(
            &index,
            "edit src/core/foundry/place-scenes.ts and src/core/index.ts",
            BUDGET,
        );
        assert!(said.contains("src/core/index.ts"), "both must fit now");
        assert!(said.contains("place-scenes.ts"));
        assert!(!said.contains("hit its budget"), "{}", said.len());
    }

    #[test]
    fn prose_that_merely_mentions_a_folder_names_no_file() {
        let tracked = paths(&["src/core/place.ts"]);
        let body = "Everything under src/core/ is yours, see the plugin folder too.";
        assert_eq!(
            paths_named_in(body, &tracked, ".ts"),
            [] as [std::string::String; 0]
        );
    }

    fn indexed(of: &[(&str, &str)]) -> Index {
        Index {
            ecosystem: Ecosystem::Rust,
            by_path: of
                .iter()
                .map(|(path, shape)| ((*path).to_string(), (*shape).to_string()))
                .collect(),
            tracked: paths(&of.iter().map(|(path, _)| *path).collect::<Vec<_>>()),
        }
    }

    #[test]
    fn only_the_signatures_the_task_names_are_carried() {
        let index = indexed(&[
            ("src/a.rs", "pub fn alpha()"),
            ("src/b.rs", "pub fn beta()"),
        ]);
        let said = carried(&index, "extend `src/a.rs` with a field", BUDGET);
        assert!(said.contains("pub fn alpha()"), "{said}");
        assert!(!said.contains("beta"), "a file nobody named costs nothing");
        assert!(said.contains("<signatures path=\"src/a.rs\">"), "{said}");
    }

    #[test]
    fn a_task_that_names_nothing_indexed_carries_no_block() {
        // The normal case for a task that creates files rather than extending
        // them. Nothing is said: an absent block asks for no gesture.
        let index = indexed(&[("src/a.rs", "pub fn alpha()")]);
        assert_eq!(carried(&index, "write a brand new module", BUDGET), "");
    }

    #[test]
    fn an_empty_index_carries_nothing_even_when_files_are_named() {
        let index = Index::default();
        assert_eq!(carried(&index, "edit src/a.rs", BUDGET), "");
    }

    #[test]
    fn going_over_the_budget_is_said_rather_than_silently_dropped() {
        let big = "pub fn wide()".repeat(400);
        let index = indexed(&[("src/a.rs", &big), ("src/b.rs", &big), ("src/c.rs", &big)]);
        let said = carried(&index, "touch src/a.rs src/b.rs src/c.rs", BUDGET);
        assert!(said.len() <= BUDGET + 200, "{}", said.len());
        assert!(said.contains("hit its budget"), "{said}");
        assert!(said.contains("Read them yourself"), "{said}");
    }

    #[test]
    fn a_file_whose_shape_is_empty_is_not_carried_as_an_empty_block() {
        // A file with nothing public gives an empty shape, and an empty
        // `<signatures>` block reads as a broken injection.
        let index = indexed(&[("src/a.rs", "   ")]);
        assert_eq!(carried(&index, "edit src/a.rs", BUDGET), "");
    }

    #[test]
    fn an_unknown_ecosystem_names_no_file_to_index() {
        let tracked = paths(&["main.zig"]);
        assert_eq!(
            paths_named_in("see main.zig", &tracked, ""),
            [] as [std::string::String; 0]
        );
    }
}

#[cfg(test)]
mod real_checkouts {
    //! Detection against the two checkouts the harness actually drives: its own
    //! repository (Rust) and the target (TypeScript). The lists are the shallow
    //! part of each `git ls-files`, so a change of layout that fooled detection
    //! would fail here rather than in a paid run.
    use super::*;

    #[test]
    fn the_harness_own_repository_is_recognised_as_rust() {
        let tracked: Vec<String> = [
            "Cargo.toml",
            "CLAUDE.md",
            "rust-toolchain.toml",
            "crates/core/Cargo.toml",
            "crates/core/src/lib.rs",
        ]
        .iter()
        .map(|p| (*p).to_string())
        .collect();
        assert_eq!(detect(&tracked), Ecosystem::Rust);
        assert_eq!(source_suffix(Ecosystem::Rust), ".rs");
    }

    #[test]
    fn the_target_repository_is_recognised_as_typescript() {
        let tracked: Vec<String> = [
            ".github/workflows/ci.yml",
            "package.json",
            "tsconfig.json",
            "vitest.config.mts",
            "src/core/place.ts",
        ]
        .iter()
        .map(|p| (*p).to_string())
        .collect();
        assert_eq!(detect(&tracked), Ecosystem::TypeScript);
        assert_eq!(source_suffix(Ecosystem::TypeScript), ".ts");
    }

    #[test]
    fn a_real_task_body_selects_the_files_its_plan_names() {
        // Shortened from #62's Technical Implementation Plan, which named every
        // file it meant to touch — that is what makes the selection possible.
        let tracked: Vec<String> = [
            "src/core/foundry/place-scenes.ts",
            "src/core/vault-layout.ts",
            "src/core/place.ts",
            "src/plugin/main.ts",
        ]
        .iter()
        .map(|p| (*p).to_string())
        .collect();
        let body = "### Step 1\n- [ ] `src/core/foundry/place-scenes.ts`: add \
                    `export function sceneFor(place: Place)`.\n- It reads the \
                    prefix from src/core/vault-layout.ts.\nProof: npm test.";
        let found = paths_named_in(body, &tracked, ".ts");
        assert_eq!(
            found,
            vec![
                "src/core/foundry/place-scenes.ts".to_string(),
                "src/core/vault-layout.ts".to_string()
            ]
        );
        // The two files the plan never mentions cost nothing.
        assert!(!found.contains(&"src/plugin/main.ts".to_string()));
    }
}

#[cfg(test)]
mod turned_off {
    //! That an empty index carries nothing, which is what `--no-signatures`
    //! relies on: the flag builds no index, and everything downstream must then
    //! behave exactly as it did before the index existed.
    use super::*;

    #[test]
    fn the_default_index_is_empty_and_carries_nothing() {
        // `--no-signatures` passes `Index::default()` rather than threading a
        // boolean through four layers. This is the test that makes that safe.
        let off = Index::default();
        assert!(off.is_empty());
        assert_eq!(off.ecosystem, Ecosystem::Unknown);
        assert_eq!(
            carried(
                &off,
                "edit src/core/place.ts and src/plugin/main.ts",
                BUDGET
            ),
            ""
        );
    }
}
