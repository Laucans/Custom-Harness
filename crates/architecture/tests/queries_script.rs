//! `scripts/queries.sh` writes the same registry whatever tools it meets.
//!
//! The golden registry was written by the GNU path (GNU `sed` and `grep`,
//! bash 5, `sha256sum`). On macOS the script is also run with nothing but
//! the system's own tools — `/bin/bash` 3.2, BSD `/usr/bin/sed`, no
//! `sha256sum` — and must write it byte for byte: BSD sed once read `\s` as
//! a literal `s` and turned `notes` into `note` while exiting 0, so an exit
//! code alone proves nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const SCRIPT: &str = include_str!("../assets/queries.sh");
const GOLDEN: &str = include_str!("golden/queries-registry.json");

/// The product's files the golden registry was written from: field names
/// ending in `s`, tabs and runs of spaces around the `fields:` line, a
/// GraphQL operation name, a SQL file named by its stem, and a read found
/// in the system's folder.
const FIXTURE: [(&str, &str); 5] = [
    (
        "crates/session/capabilities/list-notes/capability.json",
        "{\"capability\":\"session.listNotes\",\"reads\":[\"queries/list_notes.graphql\",{\"file\":\"queries/status.graphql\"}]}\n",
    ),
    (
        "crates/session/capabilities/list-notes/queries/list_notes.graphql",
        "# fields: session.notes,  game.status ,scene.focus\nquery ListNotes($id: ID!) {\n  notes\n}\n",
    ),
    (
        "crates/session/capabilities/list-notes/queries/status.graphql",
        "\t#\tfields:\tsession.status\n  query   SessionStatus {\n  status\n}\n",
    ),
    (
        "crates/billing/capabilities/load-invoices/capability.json",
        "{\"capability\":\"billing.loadInvoices\",\"reads\":[\"load_invoices.sql\"]}\n",
    ),
    (
        "crates/billing/load_invoices.sql",
        "-- fields: invoice.dues, invoice.status\nselect dues, status from invoices;\n",
    ),
];

struct Dir {
    root: PathBuf,
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A throwaway product holding [`FIXTURE`] and the script.
fn product(tag: &str) -> Dir {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("harness-queries-{tag}-{nanos}"));
    for (path, text) in FIXTURE {
        let file = root.join(path);
        fs::create_dir_all(file.parent().expect("a parent")).expect("mkdir");
        fs::write(file, text).expect("fixture");
    }
    fs::create_dir_all(root.join("scripts")).expect("mkdir");
    fs::write(root.join("scripts/queries.sh"), SCRIPT).expect("script");
    Dir { root }
}

/// The script run by `bash` with `PATH` as given (`None`: the inherited one).
fn queries(root: &Path, bash: &str, path: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(bash);
    command
        .arg("scripts/queries.sh")
        .args(args)
        .current_dir(root);
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.output().expect("bash runs")
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Whether `jq` answers on the inherited `PATH`: the script's one
/// requirement no system ships everywhere.
fn jq_on_path() -> bool {
    Command::new("jq").arg("--version").output().is_ok()
}

#[test]
fn the_tools_on_path_write_the_golden_registry() {
    if !jq_on_path() {
        println!("jq is not installed: queries.sh is not run");
        return;
    }
    let product = product("path");
    let wrote = queries(&product.root, "bash", None, &[]);
    assert!(wrote.status.success(), "{}", said(&wrote));
    let registry = fs::read_to_string(product.root.join("queries/registry.json")).expect("written");
    assert_eq!(registry, GOLDEN);
    let checked = queries(&product.root, "bash", None, &["--check"]);
    assert!(checked.status.success(), "{}", said(&checked));
}

#[test]
fn macos_own_tools_write_the_golden_registry_byte_for_byte() {
    // Elsewhere there is no BSD sed nor bash 3.2 to prove anything against.
    if !cfg!(target_os = "macos") || !Path::new("/usr/bin/jq").exists() {
        println!("not macOS with /usr/bin/jq: the system-tools run is skipped");
        return;
    }
    let product = product("bsd");
    let system = Some("/usr/bin:/bin");
    let wrote = queries(&product.root, "/bin/bash", system, &[]);
    assert!(wrote.status.success(), "{}", said(&wrote));
    let registry = fs::read_to_string(product.root.join("queries/registry.json")).expect("written");
    assert_eq!(registry, GOLDEN);
    let checked = queries(&product.root, "/bin/bash", system, &["--check"]);
    assert!(checked.status.success(), "{}", said(&checked));
}

#[test]
fn a_query_without_a_fields_line_is_refused_and_named() {
    if !jq_on_path() {
        println!("jq is not installed: queries.sh is not run");
        return;
    }
    let product = product("blank");
    fs::write(
        product.root.join("crates/billing/load_invoices.sql"),
        "select dues from invoices;\n",
    )
    .expect("rewrite");
    let refused = queries(&product.root, "bash", None, &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", said(&refused));
    assert!(
        said(&refused).contains("::error file=crates/billing/load_invoices.sql::declares no"),
        "{}",
        said(&refused)
    );
    assert!(!product.root.join("queries/registry.json").exists());
}
