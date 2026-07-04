use std::fs;
use std::path::Path;

#[test]
fn cargo_manifest_declared_targets_have_tracked_files() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml is readable");
    let tracked_files = tracked_files();

    assert_tracked(&tracked_files, "src/lib.rs");

    for path in declared_example_paths(&manifest) {
        assert_tracked(&tracked_files, &path);
    }
}

#[test]
fn cargo_target_audit_records_current_target_graph() {
    let audit = fs::read_to_string("docs/CARGO_TARGET_CI_BASELINE.md")
        .expect("cargo target audit is readable");

    for required in [
        "src/lib.rs",
        "examples/setup_wallet.rs",
        "examples/redeem_single.rs",
        "examples/redeem_all.rs",
        "examples/split_merge.rs",
        "examples/redeem_magic.rs",
        "examples/diagnose_gs026.rs",
        "examples/diagnose_nonce.rs",
        "tests/auth_test.rs",
        "tests/builder_test.rs",
        "tests/client_test.rs",
        "tests/deposit_wallet_signing_test.rs",
        "tests/deposit_wallet_test.rs",
        "tests/integration_test.rs",
        "tests/operations_test.rs",
        "tests/source_matrix_test.rs",
    ] {
        assert!(audit.contains(required), "audit must list {required}");
    }

    assert!(
        audit.contains("No stale declared Cargo target was found"),
        "audit must record stale-target decision"
    );
}

#[test]
fn rust_validation_workflow_enforces_required_pr_gates_without_secrets() {
    let workflow = fs::read_to_string(".github/workflows/rust-validation.yml")
        .expect("rust validation workflow is readable");

    for required in [
        "pull_request:",
        "permissions:",
        "contents: read",
        "cargo fmt --all --check",
        "git diff --check",
        "cargo clippy --workspace --all-targets --all-features -- -D warnings",
        "cargo test --workspace --all-features",
        "cargo build --workspace --all-targets --all-features",
    ] {
        assert!(workflow.contains(required), "workflow must include {required}");
    }

    assert!(
        !workflow.contains("secrets."),
        "rust validation workflow must not reference GitHub secrets"
    );
}

fn declared_example_paths(manifest: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut in_example = false;

    for line in manifest.lines().map(str::trim) {
        if line == "[[example]]" {
            in_example = true;
            continue;
        }

        if line.starts_with('[') {
            in_example = false;
        }

        if in_example && line.starts_with("path") {
            let Some((_, value)) = line.split_once('=') else {
                continue;
            };
            let path = value.trim().trim_matches('"');
            paths.push(path.to_owned());
        }
    }

    paths
}

fn assert_tracked(tracked_files: &[String], path: &str) {
    assert!(
        tracked_files.iter().any(|tracked| tracked == path),
        "target path {path:?} must be tracked"
    );
}

fn tracked_files() -> Vec<String> {
    fs::read_to_string(".git/index")
        .ok()
        .and_then(|_| std::process::Command::new("git").args(["ls-files"]).output().ok())
        .map(|output| String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).collect())
        .unwrap_or_else(|| {
            [
                "src/lib.rs",
                "examples/setup_wallet.rs",
                "examples/redeem_single.rs",
                "examples/redeem_all.rs",
                "examples/split_merge.rs",
                "examples/redeem_magic.rs",
                "examples/diagnose_gs026.rs",
                "examples/diagnose_nonce.rs",
            ]
            .into_iter()
            .map(str::to_owned)
            .filter(|path| Path::new(path).exists())
            .collect()
        })
}
