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
    assert!(
        audit.contains("offline-safe body verified"),
        "audit must record offline-safe example evidence"
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

    // A pull request supplies the code every step after checkout runs, so the
    // checkout itself must be a reviewed commit and must leave no credential
    // behind, and the preflight must decide whether the tree is auditable
    // before a toolchain or Cargo command touches it.
    assert!(
        !workflow.contains("actions/checkout@v"),
        "checkout must be pinned to a commit, not a movable tag"
    );
    assert!(
        workflow.contains("persist-credentials: false"),
        "checkout must not leave a credential for later steps to reach"
    );

    let preflight = workflow
        .find("python3 -I scripts/preflight_build_integrity.py")
        .expect("workflow must run the build-integrity preflight");
    for later in ["rustup ", "cargo "] {
        let first = workflow.find(later).expect("workflow must run {later}");
        assert!(
            preflight < first,
            "the preflight must run before the first `{later}` command"
        );
    }

    // On a fresh checkout a bare `git diff --check` compares the working tree
    // with itself and passes whatever the pull request contains.
    assert!(
        workflow.contains("git diff --check ${{ github.event.pull_request.base.sha }}..."),
        "the whitespace check must name the base and head it compares"
    );
}

/// A reusable workflow that receives secrets executes code this repository does
/// not hold. Pinning this file's digest says nothing about what a branch points
/// at when the job runs, so the callee is pinned to a commit as well.
#[test]
fn secret_bearing_reusable_workflows_are_pinned_to_a_commit() {
    let workflow = fs::read_to_string(".github/workflows/grimoire.yml")
        .expect("grimoire workflow is readable");
    assert!(
        workflow.contains("secrets:"),
        "this test exists because the workflow passes secrets"
    );

    for line in workflow.lines() {
        let trimmed = line.trim();
        let Some(reference) = trimmed.strip_prefix("uses:") else {
            continue;
        };
        let Some((_, revision)) = reference.trim().rsplit_once('@') else {
            panic!("every `uses:` must name a revision: {trimmed}");
        };
        assert!(
            revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit()),
            "`uses:` must name a full commit SHA, found {revision:?}"
        );
    }
}

#[test]
fn security_audit_runs_only_on_trusted_triggers_without_persisted_checkout_credentials() {
    let workflow = fs::read_to_string(".github/workflows/security-audit.yml")
        .expect("security audit workflow is readable");

    for required in [
        "schedule:",
        "workflow_dispatch:",
        "permissions:",
        "contents: read",
        "persist-credentials: false",
        "python3 -I scripts/preflight_build_integrity.py",
        "taiki-e/install-action@67729d5c413db75907f0ad1e39bb04b9c868ff60",
        "tool: cargo-audit@0.22.2",
        "fallback: none",
        "cargo audit --deny warnings",
    ] {
        assert!(workflow.contains(required), "workflow must include {required}");
    }

    for forbidden in ["pull_request:", "secrets."] {
        assert!(
            !workflow.contains(forbidden),
            "security audit workflow must not include {forbidden}"
        );
    }
}

#[test]
fn examples_remain_offline_safe() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml is readable");

    for path in declared_example_paths(&manifest) {
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("example {path} must be readable: {error}");
        });

        for needle in [
            "dotenvy",
            "env::var",
            "LocalWallet",
            "Provider",
            "RelayClient",
            "DirectExecutor",
            "DataClient",
            ".execute(",
            ".execute_batch(",
            ".execute_sequential(",
            ".wait()",
            ".deploy()",
            ".setup_approvals()",
            "PRIVATE_KEY",
            "BUILDER_SECRET",
            "BUILDER_PASSPHRASE",
            "POLY_RELAYER_API_KEY",
            "POLYGON_RPC_URL",
            "POLYMARKET_PRIVATE_KEY",
        ] {
            assert!(
                !source.contains(needle),
                "example {path} must remain offline-safe and avoid {needle}"
            );
        }
    }
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
