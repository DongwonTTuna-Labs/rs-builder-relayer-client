use ethers::types::Address;
use polymarket_relayer::{
    deposit_wallet_contract_config, DepositWalletMutationGate, DepositWalletMutationPermit,
    DepositWalletPollPolicy, DepositWalletRelayerClient, DepositWalletRelayerUrl,
    DepositWalletTransactionReceipt, RelayerKeyAuth, RelayerTransactionState,
};

#[test]
fn deposit_wallet_http_types_are_reexported_at_crate_root() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let auth = RelayerKeyAuth::new("compile-test-api-key", Address::zero());
    let config = deposit_wallet_contract_config(137).unwrap();
    let client = DepositWalletRelayerClient::new(url, auth, config).unwrap();

    let gate = DepositWalletMutationGate::Deny;
    let permit = DepositWalletMutationPermit::new("compile-level public API check");
    let policy = DepositWalletPollPolicy::default();
    let receipt = DepositWalletTransactionReceipt {
        transaction_id: "tx-public-api".to_string(),
        state: RelayerTransactionState::New,
        transaction_hash: None,
    };

    let _ = (client, gate, permit, policy, receipt);
}
