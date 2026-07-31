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
        "AmbiguousCandidate",
        "AmbiguousCandidateReport",
        "InMemoryMutationIntentStore",
        "IntentGatedClient",
        "IntentReconcileOutcome",
        "MutationIntentAuditArtifact",
        "MutationIntentLease",
        "MutationIntentRecord",
        "MutationIntentStatus",
        "MutationIntentStore",
        "OwnerMutationRegistry",
        "ReconciliationDecision",
        "ReconciliationEvidence",
        "ReconciliationSummary",
        "TryBeginOutcome",
        "MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION",
        "try_build_wallet_batch_request_with_signature",
        "CtfPositionAmount",
        "CtfRoute",
        "build_split_position_call",
        "build_merge_positions_call",
        "build_redeem_positions_call",
        "build_neg_risk_redeem_positions_call",
    ] {
        assert!(
            contains_identifier(&lib, required),
            "crate root reviewed surface is missing {required}"
        );
    }
}

#[test]
fn calldata_config_surface_is_explicit_validated_and_synchronous() {
    let lib = fs::read_to_string("src/lib.rs").expect("crate root is readable");
    let deposit_wallet = fs::read_to_string("src/deposit_wallet/mod.rs")
        .expect("deposit_wallet module is readable");
    let calldata_module = fs::read_to_string("src/deposit_wallet/calldata/mod.rs")
        .expect("calldata module is readable");
    let amount = fs::read_to_string("src/deposit_wallet/calldata/amount.rs")
        .expect("calldata amount source is readable");
    let approval = fs::read_to_string("src/deposit_wallet/calldata/approval.rs")
        .expect("calldata approval source is readable");
    let config = fs::read_to_string("src/deposit_wallet/calldata/config.rs")
        .expect("calldata config source is readable");
    let ctf = fs::read_to_string("src/deposit_wallet/calldata/ctf.rs")
        .expect("calldata CTF source is readable");
    let position = fs::read_to_string("src/deposit_wallet/calldata/position.rs")
        .expect("calldata position source is readable");
    let calldata_src =
        format!("{calldata_module}\n{amount}\n{approval}\n{config}\n{ctf}\n{position}");

    assert_no_wildcard_reexports("src/deposit_wallet/calldata/mod.rs", &calldata_module);
    assert!(
        deposit_wallet.contains("pub mod calldata;"),
        "deposit_wallet must expose the reviewed calldata module"
    );
    assert!(
        calldata_module.contains("mod amount;")
            && calldata_module.contains("mod approval;")
            && calldata_module.contains("mod ctf;")
            && calldata_module.contains("mod position;")
            && !calldata_module.contains("pub mod amount;")
            && !calldata_module.contains("pub mod approval;")
            && !calldata_module.contains("pub mod ctf;")
            && !calldata_module.contains("pub mod position;"),
        "calldata implementations must remain private modules"
    );
    assert!(
        calldata_module.contains("pub use amount::PusdAmount;")
            && calldata_module.contains(
                "pub use approval::{build_ctf_approval_for_all_call, build_pusd_approval_call};"
            ),
        "calldata module must explicitly re-export the reviewed PBRSDK-18 surface"
    );
    for required in [
        "pub use position::CtfPositionAmount;",
        "build_merge_positions_call",
        "build_neg_risk_redeem_positions_call",
        "build_redeem_positions_call",
        "build_split_position_call",
        "CtfRoute",
        "POLYGON_NEG_RISK_ADAPTER",
    ] {
        assert!(
            calldata_module.contains(required),
            "calldata module must explicitly re-export PBRSDK-19 symbol {required:?}"
        );
    }
    assert!(
        amount.contains("pub struct PusdAmount {\n    base_units: U256,\n}"),
        "PusdAmount must keep the exact reviewed private base-unit field shape"
    );
    assert!(
        amount.contains(
            "#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]\npub struct PusdAmount"
        ),
        "PusdAmount must retain the reviewed value semantics without derived Debug or Serialize"
    );
    assert!(
        position.contains("pub struct CtfPositionAmount {\n    base_units: U256,\n}"),
        "CtfPositionAmount must keep the exact reviewed private base-unit field shape"
    );
    assert!(
        position.contains(
            "#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]\npub struct CtfPositionAmount"
        ),
        "CtfPositionAmount must retain reviewed value semantics without derived Debug or Serialize"
    );
    assert!(
        ctf.contains("#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]\npub enum CtfRoute"),
        "CtfRoute must enumerate only the reviewed serializable routes"
    );
    let route_block = enum_block(&ctf, "CtfRoute");
    let route_variants = route_block
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_suffix(','))
        .collect::<Vec<_>>();
    assert_eq!(
        route_variants,
        vec![
            "ConditionalTokensSplit",
            "ConditionalTokensMerge",
            "ConditionalTokensRedeem",
            "NegRiskAdapterRedeem",
        ],
        "CtfRoute must contain exactly the four verified routes"
    );

    for signature in [
        "pub fn from_base_units(base_units: U256) -> Result<Self>",
        "pub fn from_whole_pusd(whole: u64) -> Result<Self>",
        "pub fn unlimited() -> Self",
        "pub fn base_units(&self) -> U256",
        "pub fn decimals(&self) -> u8",
        "pub fn is_unlimited(&self) -> bool",
    ] {
        assert!(
            amount.contains(signature),
            "pUSD amount surface is missing signature {signature:?}"
        );
    }

    let pusd_builder = function_signatures(&approval, "pub fn build_pusd_approval_call");
    assert_eq!(pusd_builder.len(), 1);
    for required in [
        "config: &DepositWalletCalldataConfig",
        "spender: Address",
        "amount: PusdAmount",
        "Result<DepositWalletCall>",
    ] {
        assert!(
            pusd_builder[0].contains(required),
            "pUSD approval builder signature is missing {required:?}"
        );
    }
    let ctf_builder =
        function_signatures(&approval, "pub fn build_ctf_approval_for_all_call");
    assert_eq!(ctf_builder.len(), 1);
    for required in [
        "config: &DepositWalletCalldataConfig",
        "operator: Address",
        "approved: bool",
        "Result<DepositWalletCall>",
    ] {
        assert!(
            ctf_builder[0].contains(required),
            "CTF approval builder signature is missing {required:?}"
        );
    }

    for (name, required) in [
        (
            "build_split_position_call",
            &[
                "config: &DepositWalletCalldataConfig",
                "condition_id: H256",
                "partition: &[U256]",
                "amount: PusdAmount",
                "Result<DepositWalletCall>",
            ][..],
        ),
        (
            "build_merge_positions_call",
            &[
                "config: &DepositWalletCalldataConfig",
                "condition_id: H256",
                "partition: &[U256]",
                "amount: PusdAmount",
                "Result<DepositWalletCall>",
            ][..],
        ),
        (
            "build_redeem_positions_call",
            &[
                "config: &DepositWalletCalldataConfig",
                "condition_id: H256",
                "index_sets: &[U256]",
                "Result<DepositWalletCall>",
            ][..],
        ),
        (
            "build_neg_risk_redeem_positions_call",
            &[
                "config: &DepositWalletCalldataConfig",
                "adapter: Address",
                "condition_id: H256",
                "amounts: &[CtfPositionAmount]",
                "Result<DepositWalletCall>",
            ][..],
        ),
    ] {
        let signatures = function_signatures(&ctf, &format!("pub fn {name}"));
        assert_eq!(signatures.len(), 1, "{name} must exist exactly once");
        for expected in required {
            assert!(
                signatures[0].contains(expected),
                "{name} signature is missing {expected:?}"
            );
        }
    }

    for signature in [
        "pub fn from_base_units(base_units: U256) -> Result<Self>",
        "pub fn base_units(&self) -> U256",
        "pub fn decimals(&self) -> u8",
    ] {
        assert!(
            position.contains(signature),
            "CTF position amount surface is missing signature {signature:?}"
        );
    }
    for signature in [
        "pub fn selector(&self) -> [u8; 4]",
        "pub fn target(\n        &self,\n        config: &DepositWalletCalldataConfig,\n        adapter: Option<Address>,\n    ) -> Result<Address>",
    ] {
        assert!(
            ctf.contains(signature),
            "CTF route surface is missing signature {signature:?}"
        );
    }

    for type_name in [
        "CalldataConfigInput",
        "CalldataSourceRef",
        "DepositWalletCalldataConfig",
        "SourcedAddress",
    ] {
        assert!(
            config.contains(&format!("pub struct {type_name}")),
            "calldata config source is missing public type {type_name}"
        );
    }

    for (type_name, fields) in [
        (
            "CalldataSourceRef",
            &["source_name", "version_or_commit", "url"][..],
        ),
        ("SourcedAddress", &["address", "source"][..]),
        (
            "DepositWalletCalldataConfig",
            &[
                "chain_id",
                "pusd",
                "ctf",
                "pusd_decimals",
                "pusd_decimals_source",
                "pusd_spender_allowlist",
                "ctf_operator_allowlist",
                "adapter_allowlist",
            ][..],
        ),
    ] {
        let block = struct_block(&config, type_name);
        for field in fields {
            assert!(
                block.contains(&format!("{field}:")),
                "{type_name}::{field} must remain in the reviewed shape"
            );
            assert!(
                !block.contains(&format!("pub {field}:"))
                    && !block.contains(&format!("pub(crate) {field}:")),
                "{type_name}::{field} must stay private"
            );
        }
    }

    let input = struct_block(&config, "CalldataConfigInput");
    for field in [
        "chain_id",
        "pusd",
        "ctf",
        "pusd_decimals",
        "pusd_decimals_source",
        "pusd_spender_allowlist",
        "ctf_operator_allowlist",
        "adapter_allowlist",
    ] {
        assert!(
            input.contains(&format!("pub {field}:")),
            "CalldataConfigInput::{field} must remain public input data"
        );
    }

    for signature in [
        "pub fn source_name(&self) -> &str",
        "pub fn version_or_commit(&self) -> &str",
        "pub fn url(&self) -> &str",
        "pub fn new(address: Address, source: CalldataSourceRef) -> Self",
        "pub fn address(&self) -> Address",
        "pub fn source(&self) -> &CalldataSourceRef",
        "pub fn chain_id(&self) -> u64",
        "pub fn pusd(&self) -> &SourcedAddress",
        "pub fn ctf(&self) -> &SourcedAddress",
        "pub fn pusd_decimals(&self) -> u8",
        "pub fn pusd_decimals_source(&self) -> &CalldataSourceRef",
        "pub fn pusd_spender_allowlist(&self) -> &[SourcedAddress]",
        "pub fn ctf_operator_allowlist(&self) -> &[SourcedAddress]",
        "pub fn adapter_allowlist(&self) -> &[SourcedAddress]",
        "pub fn is_allowed_pusd_spender(&self, address: Address) -> bool",
        "pub fn is_allowed_ctf_operator(&self, address: Address) -> bool",
        "pub fn is_allowed_adapter(&self, address: Address) -> bool",
        "pub fn polygon_calldata_config() -> Result<DepositWalletCalldataConfig>",
    ] {
        assert!(
            config.contains(signature),
            "calldata config surface is missing signature {signature:?}"
        );
    }

    let try_new_signatures = function_signatures(&config, "pub fn try_new");
    assert_eq!(try_new_signatures.len(), 2);
    assert!(try_new_signatures.iter().any(|signature| {
        signature.contains("source_name: impl Into<String>")
            && signature.contains("version_or_commit: impl Into<String>")
            && signature.contains("url: impl Into<String>")
            && signature.contains("Result<Self>")
    }));
    assert!(try_new_signatures.iter().any(|signature| {
        signature.contains("input: CalldataConfigInput") && signature.contains("Result<Self>")
    }));

    let reexport_start = lib
        .find("pub use deposit_wallet::calldata::{")
        .expect("crate root must explicitly re-export calldata config symbols");
    let reexport_rest = &lib[reexport_start..];
    let reexport_end = reexport_rest
        .find("};")
        .expect("calldata config re-export must have a closing delimiter");
    let reexport = &reexport_rest[..reexport_end];
    for symbol in [
        "CalldataConfigInput",
        "CalldataSourceRef",
        "DepositWalletCalldataConfig",
        "PusdAmount",
        "SourcedAddress",
        "build_ctf_approval_for_all_call",
        "build_pusd_approval_call",
        "polygon_calldata_config",
        "CtfPositionAmount",
        "CtfRoute",
        "build_split_position_call",
        "build_merge_positions_call",
        "build_redeem_positions_call",
        "build_neg_risk_redeem_positions_call",
    ] {
        assert!(
            contains_identifier(reexport, symbol),
            "crate root calldata re-export is missing {symbol}"
        );
    }

    let _f: fn() -> polymarket_relayer::Result<
        polymarket_relayer::DepositWalletCalldataConfig,
    > = polymarket_relayer::polygon_calldata_config;
    let _pusd: fn(
        &polymarket_relayer::DepositWalletCalldataConfig,
        ethers::types::Address,
        polymarket_relayer::PusdAmount,
    ) -> polymarket_relayer::Result<polymarket_relayer::DepositWalletCall> =
        polymarket_relayer::build_pusd_approval_call;
    let _ctf: fn(
        &polymarket_relayer::DepositWalletCalldataConfig,
        ethers::types::Address,
        bool,
    ) -> polymarket_relayer::Result<polymarket_relayer::DepositWalletCall> =
        polymarket_relayer::build_ctf_approval_for_all_call;
    let amount = polymarket_relayer::PusdAmount::unlimited();
    assert_eq!(amount.decimals(), 6);
    let _split: fn(
        &polymarket_relayer::DepositWalletCalldataConfig,
        ethers::types::H256,
        &[ethers::types::U256],
        polymarket_relayer::PusdAmount,
    ) -> polymarket_relayer::Result<polymarket_relayer::DepositWalletCall> =
        polymarket_relayer::build_split_position_call;
    let _merge: fn(
        &polymarket_relayer::DepositWalletCalldataConfig,
        ethers::types::H256,
        &[ethers::types::U256],
        polymarket_relayer::PusdAmount,
    ) -> polymarket_relayer::Result<polymarket_relayer::DepositWalletCall> =
        polymarket_relayer::build_merge_positions_call;
    let _redeem: fn(
        &polymarket_relayer::DepositWalletCalldataConfig,
        ethers::types::H256,
        &[ethers::types::U256],
    ) -> polymarket_relayer::Result<polymarket_relayer::DepositWalletCall> =
        polymarket_relayer::build_redeem_positions_call;
    let _neg_risk: fn(
        &polymarket_relayer::DepositWalletCalldataConfig,
        ethers::types::Address,
        ethers::types::H256,
        &[polymarket_relayer::CtfPositionAmount],
    ) -> polymarket_relayer::Result<polymarket_relayer::DepositWalletCall> =
        polymarket_relayer::build_neg_risk_redeem_positions_call;
    let _selector: fn(&polymarket_relayer::CtfRoute) -> [u8; 4] =
        polymarket_relayer::CtfRoute::selector;
    let _target: fn(
        &polymarket_relayer::CtfRoute,
        &polymarket_relayer::DepositWalletCalldataConfig,
        Option<ethers::types::Address>,
    ) -> polymarket_relayer::Result<ethers::types::Address> =
        polymarket_relayer::CtfRoute::target;
    let _adapter = polymarket_relayer::deposit_wallet::calldata::POLYGON_NEG_RISK_ADAPTER;

    assert!(
        !calldata_src.contains("reqwest"),
        "calldata modules must not depend on HTTP"
    );
    assert!(
        !calldata_src.contains("pub async fn"),
        "calldata modules must remain synchronous"
    );
    assert!(
        !calldata_src.contains("Deserialize"),
        "calldata modules must not create a runtime deserialization path"
    );
    assert!(
        !calldata_src.contains("crate::operations")
            && !calldata_src.contains("crate::contracts"),
        "calldata approval builders must not reuse legacy operations or contract defaults"
    );
}

#[test]
fn calldata_amount_keeps_base_units_private() {
    let amount = fs::read_to_string("src/deposit_wallet/calldata/amount.rs")
        .expect("calldata amount source is readable");

    assert!(
        amount.contains("pub struct PusdAmount {\n    base_units: U256,\n}"),
        "PusdAmount must keep the exact reviewed private base-unit field shape"
    );
}

#[test]
fn calldata_position_keeps_base_units_private() {
    let position = fs::read_to_string("src/deposit_wallet/calldata/position.rs")
        .expect("calldata position source is readable");

    assert!(
        position.contains("pub struct CtfPositionAmount {\n    base_units: U256,\n}"),
        "CtfPositionAmount must keep the exact reviewed private base-unit field shape"
    );
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
    let intent = fs::read_to_string("src/deposit_wallet/http/intent.rs")
        .expect("mutation intent source is readable");
    let lifecycle = fs::read_to_string("src/deposit_wallet/http/lifecycle.rs")
        .expect("deployment lifecycle source is readable");
    let mutation = fs::read_to_string("src/deposit_wallet/http/mutation.rs")
        .expect("mutation capability source is readable");
    let polling = fs::read_to_string("src/deposit_wallet/http/polling.rs")
        .expect("polling source is readable");
    let recent = fs::read_to_string("src/deposit_wallet/http/recent.rs")
        .expect("recent transaction source is readable");
    let submit = fs::read_to_string("src/deposit_wallet/http/submit.rs")
        .expect("submit source is readable");
    let read_surface = format!("{read}\n{deployed}");
    let production_http_surface = production_http_surface(&http);

    assert_no_wildcard_reexports("src/deposit_wallet/http.rs", &http);
    assert!(
        !production_http_surface.contains("crate::operations")
            && !production_http_surface.contains("crate::contracts"),
        "deposit-wallet HTTP production modules must not reuse legacy operations or contract defaults"
    );
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
        http.contains("mod intent;") && http.contains("pub use intent::{"),
        "HTTP module must include and explicitly re-export the reviewed mutation intent path"
    );
    assert!(
        http.contains("mod polling;"),
        "HTTP module must include the reviewed bounded polling path"
    );
    assert!(
        http.contains("pub use polling::{RelayerPollOutcome, RelayerPollPolicy};"),
        "HTTP module must explicitly re-export the reviewed polling types"
    );
    assert!(
        http.contains("mod recent;") && http.contains("pub use recent::{"),
        "HTTP module must include and explicitly re-export the reviewed recent report path"
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
        "AmbiguousCandidate",
        "AmbiguousCandidateReport",
        "InMemoryMutationIntentStore",
        "IntentGatedClient",
        "IntentReconcileOutcome",
        "MutationIntentAuditArtifact",
        "MutationIntentLease",
        "MutationIntentRecord",
        "MutationIntentStatus",
        "MutationIntentStore",
        "OwnerMutationRegistry",
        "ReconciliationDecision",
        "ReconciliationEvidence",
        "ReconciliationSummary",
        "TryBeginOutcome",
        "MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION",
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

    for required in [
        "pub trait MutationIntentStore",
        "pub enum TryBeginOutcome",
        "pub struct InMemoryMutationIntentStore",
        "pub struct MutationIntentRecord",
        "pub enum MutationIntentStatus",
        "pub struct OwnerMutationRegistry",
        "pub struct MutationIntentLease",
        "pub struct IntentGatedClient",
        "pub enum ReconciliationDecision",
        "pub struct ReconciliationEvidence",
        "pub struct ReconciliationSummary",
        "pub struct MutationIntentAuditArtifact",
        "pub enum IntentReconcileOutcome",
    ] {
        assert!(
            intent.contains(required),
            "mutation intent surface is missing {required}"
        );
    }
    for required in [
        "fn load(&self, owner: Address, chain_id: u64)",
        "fn try_begin(&self, template: MutationIntentRecord)",
        "expected_epoch: u64",
        "expected_revision: u64",
        "record: MutationIntentRecord",
    ] {
        assert!(
            intent.contains(required),
            "mutation intent store contract is missing {required:?}"
        );
    }
    assert!(
        !intent.contains("fn save("),
        "mutation intent storage must expose only atomic begin and versioned update writes"
    );
    assert!(
        intent.contains("#[cfg(test)]\n    pub(super) fn with_clock("),
        "registry clock injection must retain the reviewed test-only parent visibility"
    );
    assert!(
        intent.contains("pub const MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION: u32 = 1;"),
        "mutation audit artifact schema version must remain pinned at v1"
    );
    let gate_signature = function_signatures(&intent, "pub fn gate");
    assert_eq!(gate_signature.len(), 1);
    assert!(
        gate_signature[0].contains("client: &'a DepositWalletRelayerClient")
            && gate_signature[0].contains("IntentGatedClient<'a>"),
        "registry gate must return the client- and registry-borrowing wrapper"
    );
    let export_signature = function_signatures(&intent, "pub fn export_audit_artifact");
    assert_eq!(export_signature.len(), 1);
    for required in [
        "&self",
        "owner: Address",
        "chain_id: u64",
        "Result<MutationIntentAuditArtifact>",
    ] {
        assert!(
            export_signature[0].contains(required),
            "export_audit_artifact signature is missing {required:?}"
        );
    }
    let poll_record_signature = function_signatures(&intent, "pub fn record_poll_outcome");
    assert_eq!(poll_record_signature.len(), 1);
    assert!(
        poll_record_signature[0].contains("owner: Address")
            && poll_record_signature[0].contains("chain_id: u64")
            && poll_record_signature[0].contains("polled_transaction_id: &str")
            && poll_record_signature[0].contains("outcome: &RelayerPollOutcome"),
        "poll outcomes must be explicitly bound to owner, chain, and transaction ID"
    );
    let terminal_failure_signature =
        function_signatures(&intent, "pub fn record_terminal_failure");
    assert_eq!(terminal_failure_signature.len(), 1);
    assert!(
        terminal_failure_signature[0].contains("polled_transaction_id: &str")
            && terminal_failure_signature[0].contains("error: &RelayerError"),
        "terminal failure recording must stay transaction-bound and lease-free"
    );
    let begin_signature = function_signatures(&intent, "pub fn begin_intent");
    assert_eq!(begin_signature.len(), 1);
    for required in [
        "owner: Address",
        "chain_id: u64",
        "operation: RelayerMutationOperation",
        "Result<MutationIntentLease<'_>>",
    ] {
        assert!(
            begin_signature[0].contains(required),
            "begin_intent signature is missing {required:?}"
        );
    }
    let manual_signature = function_signatures(&intent, "pub fn reconcile_manually");
    assert_eq!(manual_signature.len(), 1);
    for required in [
        "owner: Address",
        "chain_id: u64",
        "expected_epoch: u64",
        "evidence: ReconciliationEvidence",
        "Result<()>",
    ] {
        assert!(
            manual_signature[0].contains(required),
            "reconcile_manually signature is missing {required:?}"
        );
    }
    let adopt_signature = function_signatures(&intent, "pub fn adopt_transaction");
    assert_eq!(adopt_signature.len(), 1);
    for required in [
        "owner: Address",
        "chain_id: u64",
        "expected_epoch: u64",
        "transaction_id: &str",
        "evidence: ReconciliationEvidence",
        "Result<()>",
    ] {
        assert!(
            adopt_signature[0].contains(required),
            "adopt_transaction signature is missing {required:?}"
        );
    }
    let reconcile_poll_signature =
        function_signatures(&intent, "pub async fn reconcile_by_polling");
    assert_eq!(reconcile_poll_signature.len(), 1);
    for required in [
        "owner: Address",
        "policy: RelayerPollPolicy",
        "read_permit: &RelayerReadPermit",
        "cancel: impl Future<Output = ()> + Send",
        "Result<IntentReconcileOutcome>",
    ] {
        assert!(
            reconcile_poll_signature[0].contains(required),
            "reconcile_by_polling signature is missing {required:?}"
        );
    }
    let report_signature =
        function_signatures(&intent, "pub async fn report_ambiguous_candidates");
    assert_eq!(report_signature.len(), 1);
    for required in [
        "owner: Address",
        "read_permit: &RelayerReadPermit",
        "Result<AmbiguousCandidateReport>",
    ] {
        assert!(
            report_signature[0].contains(required),
            "report_ambiguous_candidates signature is missing {required:?}"
        );
    }
    let record_block = struct_block(&intent, "MutationIntentRecord");
    for field in [
        "owner",
        "chain_id",
        "epoch",
        "revision",
        "operation",
        "status",
        "nonce",
        "payload_keccak256",
        "deadline_unix",
        "transaction_id",
        "last_observed_state",
        "poll_attempts",
        "reconciliation",
        "created_at_unix",
        "updated_at_unix",
    ] {
        assert!(
            record_block.contains(&format!("    {field}:")),
            "MutationIntentRecord::{field} must exist"
        );
        assert!(
            !record_block.contains(&format!("pub {field}:"))
                && !record_block.contains(&format!("pub(crate) {field}:")),
            "MutationIntentRecord::{field} must remain private"
        );
        assert!(
            intent.contains(&format!("pub fn {field}(&self)"))
                || intent.contains(&format!("pub fn {field}(\n")),
            "MutationIntentRecord::{field} must have the reviewed getter"
        );
    }
    let record_attributes = derive_attributes_for_struct(&intent, "MutationIntentRecord");
    assert!(
        record_attributes.contains("Serialize")
            && record_attributes.contains("Deserialize")
            && !record_attributes.contains("Debug"),
        "mutation intent records must round-trip through stores and use manual redacted Debug"
    );
    let status_attributes = derive_attributes_for_enum(&intent, "MutationIntentStatus");
    assert!(
        status_attributes.contains("Serialize") && status_attributes.contains("Deserialize"),
        "mutation intent status must round-trip through durable stores"
    );
    let operation_attributes =
        derive_attributes_for_enum(&mutation, "RelayerMutationOperation");
    assert!(
        operation_attributes.contains("Serialize")
            && operation_attributes.contains("Deserialize"),
        "mutation operation must be serializable as part of durable intent state"
    );
    let evidence_block = struct_block(&intent, "ReconciliationEvidence");
    for field in ["operator_ref", "decision", "summary", "recorded_at_unix"] {
        assert!(
            evidence_block.contains(&format!("    {field}:")),
            "ReconciliationEvidence::{field} must exist"
        );
        assert!(
            !evidence_block.contains(&format!("pub {field}:"))
                && !evidence_block.contains(&format!("pub(crate) {field}:")),
            "ReconciliationEvidence::{field} must remain private"
        );
        assert!(
            intent.contains(&format!("pub fn {field}(&self)")),
            "ReconciliationEvidence::{field} must have the reviewed getter"
        );
    }
    let evidence_attributes = derive_attributes_for_struct(&intent, "ReconciliationEvidence");
    assert!(
        evidence_attributes.contains("Serialize")
            && evidence_attributes.contains("Deserialize")
            && !evidence_attributes.contains("Debug"),
        "reconciliation evidence must serialize for stores and use manual redacted Debug"
    );
    let decision_attributes = derive_attributes_for_enum(&intent, "ReconciliationDecision");
    assert!(
        decision_attributes.contains("Serialize") && decision_attributes.contains("Deserialize"),
        "reconciliation decisions must round-trip with evidence"
    );
    for (name, fields) in [
        (
            "MutationIntentAuditArtifact",
            &[
                "schema_version",
                "owner",
                "chain_id",
                "operation",
                "epoch",
                "revision",
                "status",
                "nonce",
                "payload_keccak256",
                "deadline_unix",
                "transaction_id",
                "last_observed_state",
                "poll_attempts",
                "reconciliation",
                "created_at_unix",
                "updated_at_unix",
                "redaction",
            ][..],
        ),
        (
            "ReconciliationSummary",
            &[
                "decision",
                "operator_ref_len",
                "summary_len",
                "recorded_at_unix",
            ][..],
        ),
    ] {
        let block = struct_block(&intent, name);
        for field in fields {
            assert!(
                block.contains(&format!("    {field}:")),
                "{name}::{field} must exist"
            );
            assert!(
                !block.contains(&format!("pub {field}:"))
                    && !block.contains(&format!("pub(crate) {field}:")),
                "{name}::{field} must remain private"
            );
            assert!(
                intent.contains(&format!("pub fn {field}(&self)")),
                "{name}::{field} must have the reviewed getter"
            );
        }
    }
    let artifact_attributes =
        derive_attributes_for_struct(&intent, "MutationIntentAuditArtifact");
    assert!(
        artifact_attributes.contains("Clone")
            && artifact_attributes.contains("PartialEq")
            && artifact_attributes.contains("Eq")
            && artifact_attributes.contains("Serialize")
            && !artifact_attributes.contains("Debug")
            && !artifact_attributes.contains("Deserialize")
            && intent.contains("impl fmt::Debug for MutationIntentAuditArtifact"),
        "mutation audit artifacts must serialize with reviewed value semantics and use manual redacted Debug"
    );
    let summary_attributes = derive_attributes_for_struct(&intent, "ReconciliationSummary");
    assert!(
        summary_attributes.contains("Clone")
            && summary_attributes.contains("Debug")
            && summary_attributes.contains("PartialEq")
            && summary_attributes.contains("Eq")
            && summary_attributes.contains("Serialize")
            && !summary_attributes.contains("Deserialize"),
        "reconciliation summaries must retain the reviewed redacted value semantics"
    );
    assert!(
        derive_attributes_for_enum(&intent, "IntentReconcileOutcome").contains("Debug"),
        "intent reconciliation outcomes must retain reviewed debug value semantics"
    );

    for (name, fields) in [
        (
            "AmbiguousCandidate",
            &["transaction_id", "state_label", "tx_type", "created_at"][..],
        ),
        (
            "AmbiguousCandidateReport",
            &[
                "owner",
                "intent_status",
                "intent_payload_keccak256",
                "intent_epoch",
                "intent_created_at_unix",
                "candidates",
                "skipped_items",
                "redaction",
            ][..],
        ),
    ] {
        let block = struct_block(&recent, name);
        for field in fields {
            assert!(
                block.contains(&format!("    {field}:")),
                "{name}::{field} must exist"
            );
            assert!(
                !block.contains(&format!("pub {field}:"))
                    && !block.contains(&format!("pub(crate) {field}:")),
                "{name}::{field} must remain private"
            );
            assert!(
                recent.contains(&format!("pub fn {field}(&self)")),
                "{name}::{field} must have the reviewed getter"
            );
        }
        assert!(
            derive_attributes_for_struct(&recent, name).contains("Serialize"),
            "{name} must serialize as review evidence"
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
    for required in [
        "pub struct AmbiguousCandidate",
        "pub struct AmbiguousCandidateReport",
        "pub(super) async fn fetch_recent_wallet_transactions",
    ] {
        assert!(
            recent.contains(required),
            "recent report surface is missing {required}"
        );
    }
    assert!(
        !recent.contains("submit_wallet_create(")
            && !recent.contains("submit_signed_wallet_batch(")
            && !recent.contains("get_wallet_nonce("),
        "recent report lookup must remain read-only"
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
        3,
        "the complete production HTTP implementation must expose two permit-bound primitives plus one intent-gated wrapper"
    );
    for signature in production_submit_signatures {
        assert!(
            signature.contains("permit: &RelayerMutationPermit"),
            "every public submit method in the complete production HTTP implementation must require RelayerMutationPermit: {signature}"
        );
    }
    let production_execute_signatures =
        function_signatures(&production_http_surface, "pub async fn execute_wallet_batch");
    assert_eq!(
        production_execute_signatures.len(),
        2,
        "the complete production HTTP implementation must expose the primitive and intent-gated execute methods"
    );
    for signature in production_execute_signatures {
        assert!(
            signature.contains("read_permit: &RelayerReadPermit")
                && signature.contains("mutation_permit: &RelayerMutationPermit"),
            "every execute wrapper layer must remain read- and mutation-permit-bound: {signature}"
        );
    }
    let production_ensure_signatures = function_signatures(
        &production_http_surface,
        "pub async fn ensure_deposit_wallet_deployment",
    );
    assert_eq!(
        production_ensure_signatures.len(),
        2,
        "the complete production HTTP implementation must expose the primitive and intent-gated deployment methods"
    );
    for signature in production_ensure_signatures {
        assert!(
            signature.contains("read_permit: &RelayerReadPermit")
                && signature.contains("mutation_permit: Option<&RelayerMutationPermit>"),
            "every deployment wrapper layer must retain read authority and optional scoped mutation authority: {signature}"
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
        "AmbiguousCandidate",
        "AmbiguousCandidateReport",
        "InMemoryMutationIntentStore",
        "IntentGatedClient",
        "IntentReconcileOutcome",
        "MutationIntentAuditArtifact",
        "MutationIntentLease",
        "MutationIntentRecord",
        "MutationIntentStatus",
        "MutationIntentStore",
        "OwnerMutationRegistry",
        "ReconciliationDecision",
        "ReconciliationEvidence",
        "ReconciliationSummary",
        "TryBeginOutcome",
        "MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION",
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
            "docs/DECISIONS.md",
            decisions.as_str(),
            "ADR-0013: PBRSDK-12 Owner-Scoped Mutation Intent Registry",
        ),
        (
            "docs/DECISIONS.md",
            decisions.as_str(),
            "ADR-0014: PBRSDK-13 Evidence-Bound Ambiguous Reconciliation",
        ),
        (
            "docs/DECISIONS.md",
            decisions.as_str(),
            "ADR-0015: PBRSDK-15 Redacted Mutation Audit and Tracing Contract",
        ),
        (
            "docs/CONSUMER_INTEGRATION.md",
            consumer.as_str(),
            "Sample schema-v1 redacted artifact",
        ),
        (
            "docs/REVIEW_CHECKLIST.md",
            checklist.as_str(),
            "Public API Boundary",
        ),
        (
            "docs/REVIEW_CHECKLIST.md",
            checklist.as_str(),
            "Mutation Audit And Observability",
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

fn enum_block<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("pub enum {name}");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} enum should exist"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} enum should have a closing brace"));

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
