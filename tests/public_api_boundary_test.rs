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
        "DepositWalletDeploymentPolicy",
        "DepositWalletDeploymentStatus",
        "DepositWalletReadiness",
        "DepositWalletRequestContext",
        "DepositWalletCall",
        "DepositWalletDryRunEvidence",
        "DepositWalletSubmitReceipt",
        "DryRunCallSummary",
        "RelayerKeyAuth",
        "RelayerMutationMode",
        "RelayerMutationOperation",
        "RelayerMutationPermit",
        "RelayerPollOutcome",
        "RelayerPollPolicy",
        "RelayerReadPermit",
        "RelayerSubmitOutcome",
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
fn http_client_exposes_only_the_reviewed_read_and_mutation_methods() {
    let http =
        fs::read_to_string("src/deposit_wallet/http.rs").expect("HTTP module source is readable");
    let capability = fs::read_to_string("src/deposit_wallet/http/capability.rs")
        .expect("read capability source is readable");
    let read =
        fs::read_to_string("src/deposit_wallet/http/read.rs").expect("read source is readable");
    let deployed = fs::read_to_string("src/deposit_wallet/http/deployed.rs")
        .expect("deployed read source is readable");
    let execute = fs::read_to_string("src/deposit_wallet/http/execute.rs")
        .expect("WALLET batch execution source is readable");
    let lifecycle = fs::read_to_string("src/deposit_wallet/http/lifecycle.rs")
        .expect("deployment lifecycle source is readable");
    let mutation = fs::read_to_string("src/deposit_wallet/http/mutation.rs")
        .expect("mutation capability source is readable");
    let polling = fs::read_to_string("src/deposit_wallet/http/polling.rs")
        .expect("polling source is readable");
    let submit = fs::read_to_string("src/deposit_wallet/http/submit.rs")
        .expect("submit source is readable");
    let read_surface = format!("{read}\n{deployed}");
    let production_http_surface = production_http_surface(&http);

    assert_no_wildcard_reexports("src/deposit_wallet/http.rs", &http);
    assert!(
        http.contains("pub use capability::RelayerReadPermit;"),
        "HTTP module must explicitly re-export RelayerReadPermit"
    );
    assert!(
        http.contains("pub use mutation::{"),
        "HTTP module must explicitly re-export reviewed mutation types"
    );
    assert!(
        http.contains("pub use lifecycle::{"),
        "HTTP module must explicitly re-export reviewed deployment lifecycle types"
    );
    assert!(
        http.contains("mod execute;"),
        "HTTP module must include the reviewed WALLET batch execution path"
    );
    assert!(
        http.contains("mod polling;"),
        "HTTP module must include the reviewed bounded polling path"
    );
    assert!(
        http.contains("pub use polling::{RelayerPollOutcome, RelayerPollPolicy};"),
        "HTTP module must explicitly re-export the reviewed polling types"
    );
    assert!(!http.contains("pub use clock"), "clock must remain internal");

    for required in [
        "DepositWalletDeploymentPolicy",
        "DepositWalletDeploymentStatus",
        "DepositWalletDryRunEvidence",
        "DepositWalletReadiness",
        "DepositWalletSubmitReceipt",
        "DryRunCallSummary",
        "RelayerMutationMode",
        "RelayerMutationOperation",
        "RelayerMutationPermit",
        "RelayerSubmitOutcome",
    ] {
        assert!(
            contains_identifier(&http, required),
            "HTTP module reviewed mutation re-export is missing {required}"
        );
    }

    for required in [
        "pub struct RelayerReadPermit",
        "pub fn for_owner",
        "pub fn owner",
        "pub fn chain_id",
    ] {
        assert!(
            capability.contains(required),
            "read capability surface is missing {required}"
        );
    }

    for method in [
        "get_wallet_nonce",
        "get_transaction_for_owner",
        "is_deposit_wallet_deployed",
    ] {
        assert!(
            read_surface.contains(&format!("pub async fn {method}(")),
            "reviewed production read surface is missing {method}"
        );
    }
    assert_eq!(
        read_surface.matches("permit: &RelayerReadPermit").count(),
        4,
        "3 public read methods plus 1 crate-internal expected-type helper must all require RelayerReadPermit"
    );
    assert!(
        !read_surface.contains("pub async fn submit"),
        "read and deployed modules must not expose mutation submit methods"
    );

    for required in [
        "pub enum DepositWalletDeploymentPolicy",
        "pub enum DepositWalletDeploymentStatus",
        "pub enum DepositWalletReadiness",
        "pub async fn ensure_deposit_wallet_deployment(",
        "pub async fn check_deposit_wallet_deployment_readiness(",
    ] {
        assert!(
            lifecycle.contains(required),
            "deployment lifecycle surface is missing {required}"
        );
    }
    assert_eq!(
        lifecycle.matches("pub async fn ").count(),
        2,
        "the deployment lifecycle module must expose exactly the two reviewed methods"
    );
    let ensure_signature = function_signatures(
        &lifecycle,
        "pub async fn ensure_deposit_wallet_deployment",
    );
    assert_eq!(ensure_signature.len(), 1);
    assert!(
        ensure_signature[0].contains("policy: DepositWalletDeploymentPolicy")
            && ensure_signature[0].contains("read_permit: &RelayerReadPermit")
            && ensure_signature[0]
                .contains("mutation_permit: Option<&RelayerMutationPermit>"),
        "deployment entry must require an explicit policy and read permit, with optional scoped mutation authority"
    );
    let readiness_signature = function_signatures(
        &lifecycle,
        "pub async fn check_deposit_wallet_deployment_readiness",
    );
    assert_eq!(readiness_signature.len(), 1);
    assert!(
        readiness_signature[0].contains("read_permit: &RelayerReadPermit"),
        "deployment readiness must remain read-permit-bound"
    );
    assert!(
        !derive_attributes_for_enum(&lifecycle, "DepositWalletDeploymentPolicy")
            .contains("Default")
            && !lifecycle.contains("impl Default for DepositWalletDeploymentPolicy"),
        "deployment policy must be chosen explicitly and must not implement Default"
    );

    for required in [
        "pub struct RelayerPollPolicy",
        "pub fn try_new",
        "pub fn max_attempts",
        "pub fn initial_interval",
        "pub fn max_interval",
        "pub enum RelayerPollOutcome",
    ] {
        assert!(
            polling.contains(required),
            "polling surface is missing {required}"
        );
    }
    assert!(
        polling.contains("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub struct RelayerPollPolicy"),
        "poll policy must keep its reviewed value semantics"
    );
    assert!(
        !derive_attributes_for_struct(&polling, "RelayerPollPolicy").contains("Default")
            && !polling.contains("impl Default for RelayerPollPolicy"),
        "poll policy must be constructed through validated try_new"
    );
    let poll_policy = struct_block(&polling, "RelayerPollPolicy");
    for field in ["max_attempts", "initial_interval", "max_interval"] {
        assert!(
            poll_policy.contains(&format!("    {field}:")),
            "RelayerPollPolicy::{field} must exist"
        );
        assert!(
            !poll_policy.contains(&format!("pub {field}:")),
            "RelayerPollPolicy::{field} must remain private"
        );
    }
    assert_eq!(
        polling.matches("pub async fn poll_").count(),
        2,
        "the HTTP client must expose exactly the two reviewed polling methods"
    );
    for method in [
        "poll_wallet_transaction",
        "poll_deposit_wallet_deployment",
    ] {
        let signatures = function_signatures(&polling, &format!("pub async fn {method}"));
        assert_eq!(signatures.len(), 1, "{method} must exist exactly once");
        for required in [
            "owner: Address",
            "transaction_id: &str",
            "policy: RelayerPollPolicy",
            "read_permit: &RelayerReadPermit",
            "cancel: impl Future<Output = ()> + Send",
            "Result<RelayerPollOutcome>",
        ] {
            assert!(
                signatures[0].contains(required),
                "{method} signature is missing {required:?}"
            );
        }
    }
    assert!(
        !polling.contains("pub async fn submit_")
            && !polling.contains("submit_wallet_create(")
            && !polling.contains("submit_signed_wallet_batch(")
            && !polling.contains("get_wallet_nonce("),
        "polling must remain read-only and must not submit or fetch a nonce"
    );

    let execute_signature = function_signatures(&execute, "pub async fn execute_wallet_batch");
    assert_eq!(
        execute_signature.len(),
        1,
        "the HTTP client must expose exactly one reviewed execute_wallet_batch method"
    );
    for required in [
        "ctx: DepositWalletRequestContext",
        "calls: Vec<DepositWalletCall>",
        "deadline: U256",
        "signer: &S",
        "read_permit: &RelayerReadPermit",
        "mutation_permit: &RelayerMutationPermit",
        "Result<RelayerSubmitOutcome>",
        "S: Signer",
    ] {
        assert!(
            execute_signature[0].contains(required),
            "execute_wallet_batch signature is missing {required:?}"
        );
    }

    for method in ["new_with_mutation_enabled", "disable_mutation"] {
        assert!(
            http.contains(&format!("pub fn {method}(")),
            "reviewed mutation gate surface is missing {method}"
        );
    }
    for method in ["submit_wallet_create", "submit_signed_wallet_batch"] {
        assert!(
            submit.contains(&format!("pub async fn {method}(")),
            "reviewed mutation submit surface is missing {method}"
        );
        assert!(
            function_block(&submit, &format!("pub async fn {method}"))
                .contains("permit: &RelayerMutationPermit"),
            "reviewed mutation submit method {method} must require RelayerMutationPermit"
        );
    }
    assert_eq!(
        submit.matches("pub async fn submit_").count(),
        2,
        "only the two reviewed mutation submit methods may be public"
    );
    assert!(
        !http.contains("pub fn enable_mutation"),
        "the one-way mutation latch must not expose a reactivation method"
    );

    let production_submit_signatures =
        function_signatures(&production_http_surface, "pub async fn submit_");
    assert_eq!(
        production_submit_signatures.len(),
        2,
        "the complete production HTTP implementation must expose exactly two public submit methods"
    );
    for signature in production_submit_signatures {
        assert!(
            signature.contains("permit: &RelayerMutationPermit"),
            "every public submit method in the complete production HTTP implementation must require RelayerMutationPermit: {signature}"
        );
    }
    for forbidden in ["pub fn enable_mutation", "pub async fn enable_mutation"] {
        assert_eq!(
            production_http_surface.matches(forbidden).count(),
            0,
            "the complete production HTTP implementation must not expose mutation reactivation: {forbidden}"
        );
    }

    for required in [
        "pub struct RelayerMutationPermit",
        "pub fn try_new",
        "pub fn mode",
        "pub fn operation",
        "pub fn owner",
        "pub fn chain_id",
        "pub fn expires_at_unix",
        "pub struct DepositWalletDryRunEvidence",
        "pub struct DryRunCallSummary",
        "pub struct DepositWalletSubmitReceipt",
        "pub enum RelayerSubmitOutcome",
    ] {
        assert!(
            mutation.contains(required),
            "reviewed mutation capability surface is missing {required}"
        );
    }
    assert_eq!(
        mutation.matches("pub fn evidence_ref").count(),
        1,
        "only dry-run evidence may expose the evidence reference"
    );
    assert_eq!(
        mutation.matches("pub fn operator_approval_ref").count(),
        1,
        "only dry-run evidence may expose the operator approval reference"
    );
    assert!(
        mutation.contains("#[derive(Clone, PartialEq, Eq)]\npub struct RelayerMutationPermit"),
        "permit must not derive Debug, Display, or Serialize"
    );
    let permit_attributes = derive_attributes_for_struct(&mutation, "RelayerMutationPermit");
    assert!(
        !permit_attributes.contains("Debug") && !permit_attributes.contains("Serialize"),
        "permit must use only its redacted manual Debug and must not derive Serialize"
    );
    assert!(
        !mutation.contains("Serialize for RelayerMutationPermit")
            && !mutation.contains("Display for RelayerMutationPermit"),
        "permit must not expose reference contents through Serialize or Display"
    );
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
    for required in [
        "DepositWalletDeploymentPolicy",
        "DepositWalletDeploymentStatus",
        "DepositWalletDryRunEvidence",
        "DepositWalletReadiness",
        "DepositWalletSubmitReceipt",
        "DryRunCallSummary",
        "RelayerMutationMode",
        "RelayerMutationOperation",
        "RelayerMutationPermit",
        "RelayerPollOutcome",
        "RelayerPollPolicy",
        "RelayerReadPermit",
        "RelayerSubmitOutcome",
    ] {
        assert!(
            contains_identifier(&module, required),
            "deposit_wallet surface must explicitly re-export {required}"
        );
    }
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
fn mutation_permit_evidence_and_receipt_fields_stay_private() {
    let mutation = fs::read_to_string("src/deposit_wallet/http/mutation.rs")
        .expect("mutation capability source is readable");

    for (name, fields) in [
        (
            "RelayerMutationPermit",
            &[
                "mode",
                "operation",
                "owner",
                "chain_id",
                "expires_at_unix",
                "evidence_ref",
                "operator_approval_ref",
            ][..],
        ),
        (
            "DryRunCallSummary",
            &["target", "value", "selector", "data_len"][..],
        ),
        (
            "DepositWalletDryRunEvidence",
            &[
                "operation",
                "endpoint_path",
                "chain_id",
                "owner",
                "deposit_wallet",
                "to",
                "payload_keccak256",
                "nonce",
                "deadline",
                "calls",
                "evidence_ref",
                "operator_approval_ref",
                "redaction",
            ][..],
        ),
        (
            "DepositWalletSubmitReceipt",
            &["transaction_id", "state", "payload_keccak256"][..],
        ),
    ] {
        let block = struct_block(&mutation, name);
        for field in fields {
            assert!(
                block.contains(&format!("{field}:")),
                "{name}::{field} must remain in the reviewed shape"
            );
            assert!(
                !block.contains(&format!("pub {field}:"))
                    && !block.contains(&format!("pub(crate) {field}:")),
                "{name}::{field} must stay private"
            );
        }
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
            "docs/DECISIONS.md",
            decisions.as_str(),
            "ADR-0009: PBRSDK-7 Explicit Mutation Permit and Dry-Run Gate",
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

fn function_block<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("function signature {signature:?} should exist"));
    let rest = &source[start..];
    let end = rest
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("function {signature:?} should have a closing brace"));

    &rest[..end]
}

fn production_http_surface(http_module: &str) -> String {
    let mut surface = http_module.to_string();
    append_production_rust_sources(Path::new("src/deposit_wallet/http"), &mut surface);
    surface
}

fn append_production_rust_sources(directory: &Path, surface: &mut String) {
    for entry in fs::read_dir(directory).expect("production HTTP source directory is readable") {
        let path = entry.expect("production HTTP source entry is readable").path();
        if path.is_dir() {
            append_production_rust_sources(&path, surface);
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs")
            || path.file_name().and_then(|name| name.to_str()) == Some("tests.rs")
        {
            continue;
        }

        surface.push('\n');
        surface.push_str(
            &fs::read_to_string(&path).unwrap_or_else(|_| {
                panic!("production HTTP source is readable: {}", path.display())
            }),
        );
    }
}

fn function_signatures<'a>(source: &'a str, needle: &str) -> Vec<&'a str> {
    let mut signatures = Vec::new();
    let mut search_start = 0;
    while let Some(relative_start) = source[search_start..].find(needle) {
        let signature_start = search_start + relative_start;
        let rest = &source[signature_start..];
        let signature_end = rest.find('{').unwrap_or_else(|| {
            panic!("function signature beginning with {needle:?} must have an opening brace")
        });
        signatures.push(&rest[..signature_end]);
        search_start = signature_start + signature_end + 1;
    }
    signatures
}

fn derive_attributes_for_struct<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("pub struct {name}");
    let struct_start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} struct should exist"));
    let attributes_start = source[..struct_start]
        .rfind("#[derive(")
        .unwrap_or_else(|| panic!("{name} should have derive attributes"));

    &source[attributes_start..struct_start]
}

fn derive_attributes_for_enum<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("pub enum {name}");
    let enum_start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} enum should exist"));
    let attributes_start = source[..enum_start]
        .rfind("#[derive(")
        .unwrap_or_else(|| panic!("{name} should have derive attributes"));

    &source[attributes_start..enum_start]
}
