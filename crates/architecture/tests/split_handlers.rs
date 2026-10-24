//! `scripts/split-handlers.mjs`, as `init-repo` installs it, run with node
//! on a throwaway product: the legacy `mock/src/reads.js` and
//! `mock/src/commands.js` become one file per id under `mock/handlers/`,
//! byte for byte, with `_lib/` for their helpers; a second run changes
//! nothing; two trees split the same; and what it cannot split faithfully
//! is refused with nothing written. Skipped, loudly, without node 22 or
//! later (the legacy files are ES modules without a package.json `type`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::content_of;

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const READS: &str = r"// What the mock server answers a Capability with.

let bridged = false;

/** The row, bridged once. */
export async function rowOf(server) {
  if (!bridged) bridged = true;
  return server.load('row');
}

// The labels of a status.
const LABELS = { prep: 'En préparation', live: 'En direct' };

export const READS = {
  // live.row: the one row, as the fixture says.
  'live.row': async (server) => {
    const row = await rowOf(server);
    return { ...row, label: LABELS[row.status] ?? null };
  },
  // live.label: method shorthand.
  async 'live.label'(server, { status }) {
    return LABELS[status] ?? null;
  },
};
";

const COMMANDS: &str = r#"// What the mock server does with a DataCapability's command.

import { rowOf } from './reads.js';

// The term without blanks, in lowercase, without accents.
const normalized = (term) => String(term ?? '').trim().toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '');

function apiType(url) {
  return /^https:\/\//.test(url) ? 'cloud' : null;
}

export const COMMANDS = {
  // live.move: the row moves; a name with { or ' in a template stays one string.
  'live.move': async (server, { payload }) => {
    const row = await rowOf(server);
    if (!normalized(payload.name)) return ['name-required'];
    row.name = `${normalized(payload.name)} {${'}'}`.replace(/['"{]/g, '');
    return undefined;
  }, // the trailing comment of live.move

  // ai.save: a cloud URL only.
  'ai.save': async (server, { payload }) => (apiType(String(payload.url || '')) ? undefined : ['not-cloud'])
};
"#;

/// The node on the path, when it is 22 or later.
fn node_recent() -> bool {
    let Ok(out) = Command::new("node").arg("--version").output() else {
        eprintln!("split_handlers: node is not on the path, skipped");
        return false;
    };
    let version = String::from_utf8_lossy(&out.stdout);
    let major: u32 = version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    if major < 22 {
        eprintln!("split_handlers: node {version} is older than 22, skipped");
    }
    major >= 22
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// A product with the harness's server and splitter, the fixtures and the
/// two legacy files, each replaced by `with` when it names it.
fn product(case: &str, with: &[(&str, &str)]) -> Dir {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "split-handlers-{case}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("mkdir");
    for path in ["mock/src/server.js", "scripts/split-handlers.mjs"] {
        write(&root, path, content_of(path).expect("installed"));
    }
    write(&root, "mock/src/screen.js", "export const screen = 1;\n");
    write(
        &root,
        "mock/fixtures/row.json",
        r#"{"status":"live","name":"x"}"#,
    );
    write(&root, "mock/src/reads.js", READS);
    write(&root, "mock/src/commands.js", COMMANDS);
    for (path, text) in with {
        write(&root, path, text);
    }
    Dir(root)
}

/// Runs the splitter in `root`: whether it succeeded, and what it said.
fn split(root: &Path) -> (bool, String) {
    let out = Command::new("node")
        .arg("scripts/split-handlers.mjs")
        .current_dir(root)
        .output()
        .expect("node runs");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Every file under `mock/`, by path.
fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).expect("a folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("under the root")
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, fs::read_to_string(&path).expect("text"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &root.join("mock"), &mut out);
    out
}

const MOVE: &str = r#"import { normalized } from './_lib/commands.js';
import { rowOf } from './_lib/reads.js';

export const commands = {
  // live.move: the row moves; a name with { or ' in a template stays one string.
  'live.move': async (server, { payload }) => {
    const row = await rowOf(server);
    if (!normalized(payload.name)) return ['name-required'];
    row.name = `${normalized(payload.name)} {${'}'}`.replace(/['"{]/g, '');
    return undefined;
  }, // the trailing comment of live.move
};
"#;

const LABEL: &str = r"import { LABELS } from './_lib/reads.js';

export const reads = {
  // live.label: method shorthand.
  async 'live.label'(server, { status }) {
    return LABELS[status] ?? null;
  },
};
";

#[test]
fn the_legacy_files_become_one_file_per_id_byte_for_byte_and_only_once() {
    if !node_recent() {
        return;
    }
    let one = product("split", &[]);
    let (ok, said) = split(&one.0);
    assert!(ok, "{said}");
    assert!(
        said.contains("split 2 read(s) and 2 command(s) into 6 file(s)"),
        "{said}"
    );
    let files = tree(&one.0);
    assert!(!files.contains_key("mock/src/reads.js"), "{files:?}");
    assert!(!files.contains_key("mock/src/commands.js"), "{files:?}");
    assert_eq!(files["mock/handlers/live.move.js"], MOVE);
    assert_eq!(files["mock/handlers/live.label.js"], LABEL);
    let save = &files["mock/handlers/ai.save.js"];
    assert!(
        save.starts_with("import { apiType } from './_lib/commands.js';\n\nexport const commands = {\n  // ai.save: a cloud URL only.\n"),
        "{save}"
    );
    assert!(save.ends_with("['not-cloud']),\n};\n"), "{save}");
    let lib = &files["mock/handlers/_lib/commands.js"];
    for part in [
        "import { rowOf } from './reads.js';",
        "export const normalized = ",
        "export function apiType(url)",
    ] {
        assert!(lib.contains(part), "{part}: {lib}");
    }
    assert!(!lib.contains("COMMANDS = {"), "{lib}");
    let lib = &files["mock/handlers/_lib/reads.js"];
    for part in [
        "export let bridged = false;",
        "export async function rowOf(server)",
        "export const LABELS = ",
    ] {
        assert!(lib.contains(part), "{part}: {lib}");
    }
    // Again on a split tree: nothing to do, nothing changed.
    let (ok, said) = split(&one.0);
    assert!(ok && said.contains("nothing to split"), "{said}");
    assert_eq!(tree(&one.0), files);
    // Another tree with the same legacy files splits the same.
    let two = product("again", &[]);
    assert!(split(&two.0).0);
    assert_eq!(tree(&two.0), files);
}

/// Runs the splitter on a product carrying `with`: it refuses, says `why`,
/// and leaves every file as it was.
fn refused(case: &str, with: &[(&str, &str)], why: &str) {
    let dir = product(case, with);
    let before = tree(&dir.0);
    let (ok, said) = split(&dir.0);
    assert!(!ok, "{case}: {said}");
    assert!(said.contains("refused, nothing written"), "{case}: {said}");
    assert!(said.contains(why), "{case}: {why} — {said}");
    assert_eq!(tree(&dir.0), before, "{case}: nothing written");
}

#[test]
fn what_cannot_be_split_faithfully_is_refused_with_nothing_written() {
    if !node_recent() {
        return;
    }
    let reads_with = |entry: &str| {
        READS.replace(
            "export const READS = {\n",
            &format!("export const READS = {{\n{entry}\n"),
        )
    };
    let assigns = reads_with("  'live.reset': async () => { bridged = false; },");
    refused(
        "assigns",
        &[("mock/src/reads.js", &assigns)],
        "live.reset assigns the top-level `bridged` of its file",
    );
    let bumps = reads_with("  'live.count': async () => ++bridged,");
    refused(
        "bumps",
        &[("mock/src/reads.js", &bumps)],
        "live.count assigns the top-level `bridged`",
    );
    let computed = reads_with("  [LABELS.prep]: async () => 1,");
    refused(
        "computed",
        &[("mock/src/reads.js", &computed)],
        "a computed key",
    );
    let spread = reads_with("  ...LABELS,");
    refused(
        "spread",
        &[("mock/src/reads.js", &spread)],
        "a spread entry",
    );
    let getter = reads_with("  get 'live.get'() { return 1; },");
    refused(
        "getter",
        &[("mock/src/reads.js", &getter)],
        "a getter or a setter",
    );
    let shorthand = reads_with("  rowOf,");
    refused(
        "shorthand",
        &[("mock/src/reads.js", &shorthand)],
        "the shorthand entry `rowOf`",
    );
    let twice = reads_with("  'live.label': async () => 'the later one wins',");
    refused(
        "twice",
        &[("mock/src/reads.js", &twice)],
        "defines live.label twice (lines 15, 22)",
    );
    let itself = reads_with("  'live.all': async () => Object.keys(READS),");
    refused(
        "itself",
        &[("mock/src/reads.js", &itself)],
        "live.all uses READS",
    );
    refused(
        "old-server",
        &[(
            "mock/src/server.js",
            "import { COMMANDS } from './commands.js';\nexport const x = COMMANDS;\n",
        )],
        "mock/src/server.js still imports a legacy handler file",
    );
    refused(
        "already-there",
        &[(
            "mock/handlers/live.label.js",
            "export const reads = { 'live.label': async () => 'mine' };\n",
        )],
        "mock/handlers/live.label.js is already there, with other content",
    );
    // Guarded assignments the self-check's empty payload never reaches.
    let guarded = |line: &str| {
        reads_with(&format!(
            "  'live.set': async (server, {{ id, ids }}) => {{ if (!id) return null; {line} return id; }},"
        ))
    };
    for (case, line) in [
        ("destructures", "[bridged] = [id];"),
        ("destructures-object", "({ bridged } = { bridged: id });"),
        ("nested", "[[bridged]] = [[id]];"),
        ("for-of", "for (bridged of ids) {}"),
        ("for-in", "for (bridged in ids) {}"),
    ] {
        refused(
            case,
            &[("mock/src/reads.js", &guarded(line))],
            "live.set assigns the top-level `bridged` of its file",
        );
    }
    let this = reads_with(
        "  async 'live.base'() { return 1; },\n  async 'live.twice'(server, { id }) { if (!id) return null; return this['live.base'](); },",
    );
    refused(
        "this",
        &[("mock/src/reads.js", &this)],
        "live.twice uses `this`",
    );
    // A local destructuring declaration is no assignment to the top level.
    let local = reads_with(
        "  'live.local': async (server, { id }) => { const [bridged] = [id]; let { LABELS } = { LABELS: 1 }; return bridged; },",
    );
    let dir = product("local", &[("mock/src/reads.js", &local)]);
    let (ok, said) = split(&dir.0);
    assert!(ok, "a local declaration splits: {said}");
    // A helper reached through eval escapes the import scan: the source is the
    // same, the answers are not, and the self-check sees it.
    let sneaky = COMMANDS.replace(
        "export const COMMANDS = {\n",
        "export const COMMANDS = {\n  'live.sneaky': async () => (eval('normalized')('É') ? undefined : ['empty']),\n",
    );
    refused(
        "eval",
        &[("mock/src/commands.js", &sneaky)],
        "the split mock server answers otherwise than the legacy one for command live.sneaky",
    );
}

#[test]
fn an_older_node_is_told_rather_than_crashing() {
    if !node_recent() {
        return;
    }
    let dir = product("version", &[]);
    let script = dir.0.join("scripts/split-handlers.mjs");
    let url = format!("file://{}", script.display());
    let out = Command::new("node")
        .args([
            "--input-type=module",
            "-e",
            &format!(
                "import {{ nodeIsRecent }} from '{url}'; console.log(nodeIsRecent('20.11.1'), nodeIsRecent('22.0.0'), nodeIsRecent('26.1.0'))"
            ),
        ])
        .output()
        .expect("node runs");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "false true true",
        "{out:?}"
    );
    let text = fs::read_to_string(&script).expect("the script");
    for modern in ["?.", "??", "Object.hasOwn", "||=", "&&="] {
        let outside_strings = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| !line.contains('\'') && !line.contains('`'));
        assert!(
            outside_strings.clone().all(|line| !line.contains(modern)),
            "the script parses on an old node: {modern}"
        );
    }
}

/// A legacy file written without semicolons: a helper beside the literal,
/// a second import, a handler that returns early on an empty payload.
const BARE: &str = r"// What the mock server does with a command, without semicolons.

import { rowOf } from './reads.js'
import { cap } from './limits.js'
const LIMIT = 3
let calls = 0
const tooMany = (n) => n > cap(LIMIT)
export const COMMANDS = {
  'a.add': async (server, { payload }) => {
    if (!payload.items) return ['items-required']
    if (tooMany(payload.items.length)) return ['too-many']
    await rowOf(server)
  },
}
";

#[test]
fn a_file_without_semicolons_keeps_every_helper_its_handlers_use() {
    if !node_recent() {
        return;
    }
    let dir = product(
        "bare",
        &[
            ("mock/src/commands.js", BARE),
            ("mock/src/limits.js", "export const cap = (n) => n\n"),
        ],
    );
    let (ok, said) = split(&dir.0);
    assert!(ok, "{said}");
    let files = tree(&dir.0);
    let add = &files["mock/handlers/a.add.js"];
    assert!(
        add.starts_with(
            "import { tooMany } from './_lib/commands.js';\nimport { rowOf } from './_lib/reads.js';\n\n"
        ),
        "{add}"
    );
    let lib = &files["mock/handlers/_lib/commands.js"];
    for part in [
        "import { rowOf } from './reads.js'\n",
        "import { cap } from '../../src/limits.js'\n",
        "export const LIMIT = 3\n",
        "export let calls = 0\n",
        "export const tooMany = (n) => n > cap(LIMIT)\n",
    ] {
        assert!(lib.contains(part), "{part}: {lib}");
    }
    // The guarded path the self-check's empty payload never reaches runs.
    let url = format!("file://{}", dir.0.join("mock/handlers/a.add.js").display());
    let out = Command::new("node")
        .args([
            "--input-type=module",
            "-e",
            &format!(
                "import {{ commands }} from '{url}'; console.log(JSON.stringify(await commands['a.add']({{}}, {{ payload: {{ items: [1, 2, 3, 4] }} }})))"
            ),
        ])
        .output()
        .expect("node runs");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"["too-many"]"#,
        "{out:?}"
    );
}

#[test]
fn a_handler_file_a_merge_already_brought_beside_the_legacy_one_splits() {
    if !node_recent() {
        return;
    }
    let clean = product("clean", &[]);
    assert!(split(&clean.0).0);
    let files = tree(&clean.0);
    let merged = product("merged", &[("mock/handlers/live.label.js", LABEL)]);
    let (ok, said) = split(&merged.0);
    assert!(ok, "{said}");
    assert_eq!(tree(&merged.0), files, "the same split, the file kept");
}
