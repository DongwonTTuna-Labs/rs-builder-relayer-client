use ethers::types::Address;
use polymarket_relayer::{
    deposit_wallet_contract_config, DepositWalletIdlessSubmitReconciliationEvidence,
    DepositWalletMutationAction, DepositWalletMutationGate, DepositWalletMutationPermit,
    DepositWalletNonceLease, DepositWalletOwnerSerializationEvidence, DepositWalletPollPolicy,
    DepositWalletRelayerClient, DepositWalletRelayerUrl, DepositWalletSubmitReconciliationEvidence,
    DepositWalletSubmitReconciliationObservation, DepositWalletTransactionReceipt, RelayerKeyAuth,
    RelayerTransactionState,
};

#[test]
fn deposit_wallet_http_types_are_reexported_at_crate_root() {
    fn assert_debug<T: std::fmt::Debug>() {}

    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let auth = RelayerKeyAuth::new("compile-test-api-key", Address::zero()).unwrap();
    let config = deposit_wallet_contract_config(137).unwrap();
    let client = DepositWalletRelayerClient::new(url, auth, config).unwrap();

    let gate = DepositWalletMutationGate::Deny;
    let evidence = DepositWalletOwnerSerializationEvidence::new(
        Address::zero(),
        client.mutation_scope(DepositWalletMutationAction::WalletCreate).unwrap(),
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
    let receipt = DepositWalletTransactionReceipt {
        transaction_id: "tx-public-api".to_string(),
        state: RelayerTransactionState::New,
        transaction_hash: None,
        owner: None,
    };

    let rendered = format!("{client:?}");
    assert!(rendered.contains("DepositWalletRelayerClient"));
    assert!(!rendered.contains("compile-test-api-key"));
    assert_debug::<DepositWalletNonceLease>();
    assert_debug::<DepositWalletSubmitReconciliationEvidence>();
    assert_debug::<DepositWalletSubmitReconciliationObservation>();
    assert_debug::<DepositWalletIdlessSubmitReconciliationEvidence>();

    let _ = (gate, policy, receipt);
}
