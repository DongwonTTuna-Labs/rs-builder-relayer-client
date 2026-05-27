use ethers::types::Address;
use polymarket_relayer::{
    deposit_wallet_contract_config, DepositWalletIdlessSubmitReconciliationEvidence,
    DepositWalletMutationAction, DepositWalletMutationGate, DepositWalletMutationPermit,
    DepositWalletOwnerSerializationEvidence, DepositWalletPollPolicy, DepositWalletRelayerClient,
    DepositWalletRelayerUrl, DepositWalletSubmitReconciliationEvidence,
    DepositWalletSubmitReconciliationObservation, DepositWalletTransactionReceipt,
    DepositWalletWalletNonceEvidence, RelayerKeyAuth, RelayerTransactionState,
};

#[test]
fn deposit_wallet_http_types_are_reexported_at_crate_root() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let auth = RelayerKeyAuth::new("compile-test-api-key", Address::zero());
    let config = deposit_wallet_contract_config(137).unwrap();
    let client = DepositWalletRelayerClient::new(url, auth, config).unwrap();

    let gate = DepositWalletMutationGate::Deny;
    let evidence = DepositWalletOwnerSerializationEvidence::new(
        Address::zero(),
        client.mutation_scope(DepositWalletMutationAction::WalletCreate),
        "compile-test owner lock",
        "compile-test owner serialization evidence",
        1,
        2,
    )
    .unwrap();
    assert!(DepositWalletMutationPermit::from_owner_serialization_evidence(
        "compile-test explicit mutation approval",
        evidence,
    )
    .is_err());
    let policy = DepositWalletPollPolicy::default();
    let reconciliation = DepositWalletSubmitReconciliationEvidence::new(
        Address::zero(),
        client.mutation_scope(DepositWalletMutationAction::ManualReconciliation),
        "compile-test owner lock",
        "0x1111111111111111111111111111111111111111111111111111111111111111",
        DepositWalletSubmitReconciliationObservation::new(
            "tx-public-api",
            RelayerTransactionState::Failed,
            None::<&str>,
            "compile-test manual reconciliation",
            1,
        )
        .unwrap(),
    )
    .unwrap();
    let idless_reconciliation = DepositWalletIdlessSubmitReconciliationEvidence::new(
        Address::zero(),
        client.mutation_scope(DepositWalletMutationAction::ManualReconciliation),
        "compile-test owner lock",
        "0x1111111111111111111111111111111111111111111111111111111111111111",
        "compile-test id-less reconciliation",
        1,
    )
    .unwrap();
    let receipt = DepositWalletTransactionReceipt {
        transaction_id: "tx-public-api".to_string(),
        state: RelayerTransactionState::New,
        transaction_hash: None,
        owner: None,
    };
    let nonce_evidence: Option<DepositWalletWalletNonceEvidence> = None;

    let rendered = format!("{client:?}");
    assert!(rendered.contains("DepositWalletRelayerClient"));
    assert!(!rendered.contains("compile-test-api-key"));

    let _ = (
        gate,
        policy,
        reconciliation,
        idless_reconciliation,
        receipt,
        nonce_evidence,
    );
}
