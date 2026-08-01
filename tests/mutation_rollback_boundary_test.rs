use ethers::types::Address;
use polymarket_relayer::{
    deposit_wallet_contract_config, DepositWalletRelayerClient, DepositWalletRelayerUrl,
    RelayerKeyAuth, RelayerMutationMode, RelayerMutationOperation, RelayerMutationPermit,
    RelayerReadPermit,
};

const POLYGON_CHAIN_ID: u64 = 137;
const PERMIT_EXPIRY_UNIX: u64 = 4_102_444_800;

fn address(value: u64) -> Address {
    Address::from_low_u64_be(value)
}

fn mutation_enabled_client(owner: Address) -> DepositWalletRelayerClient {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/").unwrap();
    let auth = RelayerKeyAuth::new("test-api-key", owner).unwrap();
    let config = deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap();

    DepositWalletRelayerClient::new_with_mutation_enabled(url, auth, config).unwrap()
}

#[tokio::test]
async fn rollback_blocks_live_mutation_before_network_dispatch() {
    let owner = address(1);
    let client = mutation_enabled_client(owner);
    let permit = RelayerMutationPermit::try_new(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        PERMIT_EXPIRY_UNIX,
        "evidence-ref",
        "operator-approval-ref",
    )
    .unwrap();

    client.disable_mutation();
    let error = client
        .submit_wallet_create(owner, &permit)
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error
        .to_string()
        .contains("relayer mutation is disabled for this client"));
}

#[tokio::test]
async fn rollback_keeps_read_permit_validation_active_before_network_dispatch() {
    let owner = address(1);
    let client = mutation_enabled_client(owner);
    client.disable_mutation();

    let wrong_owner_permit = RelayerReadPermit::for_owner(address(2), POLYGON_CHAIN_ID);
    let owner_error = client
        .get_wallet_nonce(owner, &wrong_owner_permit)
        .await
        .unwrap_err();
    assert!(owner_error.is_deposit_wallet_read_blocked());
    assert!(owner_error
        .to_string()
        .contains("read permit owner did not match requested owner"));

    let wrong_chain_permit = RelayerReadPermit::for_owner(owner, 80_002);
    let chain_error = client
        .get_wallet_nonce(owner, &wrong_chain_permit)
        .await
        .unwrap_err();
    assert!(chain_error.is_deposit_wallet_read_blocked());
    assert!(chain_error
        .to_string()
        .contains("read permit chain 80002 did not match configured chain 137"));
}
