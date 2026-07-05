use std::fs;
use std::path::Path;

#[test]
fn crate_root_keeps_reviewed_deposit_wallet_surface() {
    let lib = fs::read_to_string("src/lib.rs").expect("crate root is readable");

    assert_no_wildcard_reexports("src/lib.rs", &lib);
    assert!(
        !contains_identifier(&lib, "build_wallet_batch_request_with_signature"),
        "crate root must not expose the removed infallible WALLET batch helper"
    );
    assert!(
        !contains_identifier(&lib, "DepositWalletBatchRequest"),
        "crate root must not expose raw WALLET submit DTO construction"
    );

    for required in [
        "DepositWalletRelayerClient",
        "DepositWalletRelayerUrl",
        "DepositWalletRequestContext",
        "DepositWalletCall",
        "RelayerKeyAuth",
        "RelayerSubmitResponse",
        "RelayerTransactionState",
        "try_build_wallet_batch_request_with_signature",
    ] {
        assert!(
            contains_identifier(&lib, required),
            "crate root reviewed surface is missing {required}"
        );
    }
}

#[test]
fn deposit_wallet_exports_are_explicit_and_not_clob_or_legacy_execute_paths() {
    let module = fs::read_to_string("src/deposit_wallet/mod.rs")
        .expect("deposit_wallet module is readable");

    assert_no_wildcard_reexports("src/deposit_wallet/mod.rs", &module);
    assert!(
        contains_identifier(&module, "try_build_wallet_batch_request_with_signature"),
        "deposit_wallet surface must advertise the fallible WALLET batch helper"
    );
    assert!(
        !contains_identifier(&module, "build_wallet_batch_request_with_signature"),
        "deposit_wallet surface must not restore the removed infallible helper"
    );

    for forbidden in ["pub mod clob", "pub use clob", "pub mod order", "pub use order"] {
        assert!(
            !module.contains(forbidden),
            "deposit_wallet module must not add CLOB SDK public surface: {forbidden}"
        );
    }
}

#[test]
fn wallet_submit_dto_fields_stay_crate_private() {
    let types = fs::read_to_string("src/deposit_wallet/types.rs")
        .expect("deposit_wallet types are readable");

    let batch_request = struct_block(&types, "DepositWalletBatchRequest");
    for field in [
        "tx_type",
        "from_address",
        "to",
        "nonce",
        "signature",
        "deposit_wallet_params",
    ] {
        assert!(
            batch_request.contains(&format!("pub(crate) {field}:")),
            "DepositWalletBatchRequest::{field} must stay crate-private"
        );
        assert!(
            !batch_request.contains(&format!("pub {field}:")),
            "DepositWalletBatchRequest::{field} must not become publicly constructible"
        );
    }

    let params = struct_block(&types, "DepositWalletParams");
    for field in ["deposit_wallet", "deadline", "calls"] {
        assert!(
            params.contains(&format!("pub(crate) {field}:")),
            "DepositWalletParams::{field} must stay crate-private"
        );
        assert!(
            !params.contains(&format!("pub {field}:")),
            "DepositWalletParams::{field} must not become publicly constructible"
        );
    }
}

#[test]
fn repository_does_not_grow_clob_sdk_modules_or_examples() {
    let operations = fs::read_to_string("src/operations/mod.rs")
        .expect("legacy operations module is readable");
    assert_no_wildcard_reexports("src/operations/mod.rs", &operations);

    for forbidden_path in [
        "src/clob.rs",
        "src/clob/mod.rs",
        "src/orders.rs",
        "src/orders/mod.rs",
        "examples/clob.rs",
        "examples/order.rs",
        "examples/cancel.rs",
    ] {
        assert!(
            !Path::new(forbidden_path).exists(),
            "{forbidden_path} would make this crate look like a CLOB SDK"
        );
    }

    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml is readable");
    for forbidden_example in ["name = \"clob", "name = \"order", "name = \"cancel"] {
        assert!(
            !manifest.contains(forbidden_example),
            "Cargo examples must not advertise CLOB order/cancel behavior"
        );
    }
}

#[test]
fn docs_record_semver_boundary_and_grep_audit_contract() {
    let readme = fs::read_to_string("README.md").expect("README is readable");
    let consumer = fs::read_to_string("docs/CONSUMER_INTEGRATION.md")
        .expect("consumer integration docs are readable");
    let decisions = fs::read_to_string("docs/DECISIONS.md").expect("decisions are readable");
    let checklist =
        fs::read_to_string("docs/REVIEW_CHECKLIST.md").expect("review checklist is readable");

    for (path, text, required) in [
        (
            "README.md",
            readme.as_str(),
            "Reviewed 0.2.0 Public API Boundary",
        ),
        (
            "docs/CONSUMER_INTEGRATION.md",
            consumer.as_str(),
            "Reviewed 0.2.0 public integration surface",
        ),
        (
            "docs/DECISIONS.md",
            decisions.as_str(),
            "ADR-0007: PBRSDK-4 Public API Boundary Audit",
        ),
        (
            "docs/REVIEW_CHECKLIST.md",
            checklist.as_str(),
            "Public API Boundary",
        ),
    ] {
        assert!(text.contains(required), "{path} is missing {required:?}");
    }

    for required in [
        "CLOB order/sign/cancel/post behavior remains out of this crate",
        "tests/public_api_boundary_test.rs",
        "cargo doc --workspace --all-features --no-deps",
        "grep -R",
    ] {
        assert!(
            consumer.contains(required)
                || decisions.contains(required)
                || checklist.contains(required),
            "public API boundary docs must include {required:?}"
        );
    }
}

fn assert_no_wildcard_reexports(path: &str, source: &str) {
    for line in source.lines().map(str::trim) {
        assert!(
            !(line.starts_with("pub use ") && line.contains("::*")),
            "{path} must not use wildcard public re-exports: {line}"
        );
    }
}

fn contains_identifier(source: &str, identifier: &str) -> bool {
    source
        .split(|character: char| !(character == '_' || character.is_ascii_alphanumeric()))
        .any(|token| token == identifier)
}

fn struct_block<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("pub struct {name}");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} struct should exist"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} struct should have a closing brace"));

    &rest[..end]
}
