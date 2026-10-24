//! Every Rust file `init-repo` installs is `rustfmt`-clean as installed: a
//! product's CI runs `cargo fmt --all -- --check`, and an asset that fails
//! it turns the integration branch red on the next upgrade — the assets are
//! no member of this workspace, so `cargo fmt --all` here never sees them.
//!
//! Needs `rustfmt` on the path; without it the test says so and passes.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use harness_architecture::assets::FILES;

/// A folder removed on drop.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn every_rust_asset_is_rustfmt_clean_under_a_product_s_default_config() {
    if Command::new("rustfmt").arg("--version").output().is_err() {
        eprintln!("rustfmt is not on the path: skipped");
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = Scratch(std::env::temp_dir().join(format!("harness-assets-fmt-{nanos}")));
    // Every asset written where it is installed: a `mod x;` finds its file.
    let rust: Vec<_> = FILES
        .iter()
        .filter(|asset| asset.path.ends_with(".rs"))
        .collect();
    assert!(!rust.is_empty(), "the skeletons install Rust files");
    for asset in &rust {
        let path = root.0.join(asset.path);
        fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        fs::write(&path, asset.content).expect("write");
    }
    // A product carries no `rustfmt.toml`: the defaults, nothing of this
    // machine's own configuration.
    let config = root.0.join("rustfmt.toml");
    fs::write(&config, "").expect("config");
    for asset in &rust {
        let out = Command::new("rustfmt")
            .args(["--check", "--edition", "2024", "--config-path"])
            .arg(&config)
            .arg(root.0.join(asset.path))
            .output()
            .expect("rustfmt runs");
        assert!(
            out.status.success(),
            "{} is not rustfmt-clean:\n{}{}",
            asset.path,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
