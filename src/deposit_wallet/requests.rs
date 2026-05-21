use ethers::types::U256;

use crate::deposit_wallet::{
    DepositWalletBatchRequest, DepositWalletCall, DepositWalletContractConfig,
    DepositWalletCreateRequest, DepositWalletParams, DepositWalletRequestContext,
    WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};

pub fn build_wallet_create_request(
    owner_address: ethers::types::Address,
    config: DepositWalletContractConfig,
) -> DepositWalletCreateRequest {
    DepositWalletCreateRequest {
        tx_type: WALLET_CREATE_TRANSACTION_TYPE.to_string(),
        from_address: owner_address,
        to: config.factory,
    }
}

pub(crate) fn build_wallet_batch_request_with_signature(
    ctx: DepositWalletRequestContext,
    config: DepositWalletContractConfig,
    nonce: U256,
    deadline: U256,
    calls: Vec<DepositWalletCall>,
    signature: String,
) -> DepositWalletBatchRequest {
    DepositWalletBatchRequest {
        tx_type: WALLET_TRANSACTION_TYPE.to_string(),
        from_address: ctx.owner_address,
        to: config.factory,
        nonce,
        signature,
        deposit_wallet_params: DepositWalletParams {
            deposit_wallet: ctx.deposit_wallet_address,
            deadline,
            calls,
        },
    }
}

#[cfg(test)]
mod tests {
    use ethers::types::{Address, Bytes, U256};
    use serde_json::Value;

    use super::build_wallet_batch_request_with_signature;
    use crate::deposit_wallet::{
        deposit_wallet_contract_config, DepositWalletCall, DepositWalletRequestContext,
    };

    fn fixture(path: &str) -> Value {
        let full_path = format!("tests/fixtures/{path}");
        let text = std::fs::read_to_string(&full_path).expect("fixture should be readable");
        serde_json::from_str(&text).expect("fixture should be valid JSON")
    }

    #[test]
    fn wallet_batch_submit_body_matches_fixture() {
        let owner: Address = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5"
            .parse()
            .unwrap();
        let deposit_wallet: Address = "0x069F89dAEfbaDdF5B6639Dc34D73E59cCCBC63De"
            .parse()
            .unwrap();
        let target: Address = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB"
            .parse()
            .unwrap();
        let config = deposit_wallet_contract_config(137).unwrap();
        let ctx = DepositWalletRequestContext {
            owner_address: owner,
            deposit_wallet_address: deposit_wallet,
        };
        let call = DepositWalletCall {
            target,
            value: U256::zero(),
            data: Bytes::from(hex::decode("095ea7b30000000000000000000000004d97dcd97ec945f40cf65f87097ace5ea0476045ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").unwrap()),
        };
        let signature = "0x111111111111111111111111111111111111111111111111111111111111111122222222222222222222222222222222222222222222222222222222222222221b";

        let request = build_wallet_batch_request_with_signature(
            ctx,
            config,
            U256::from(31u64),
            U256::from(1_760_000_000u64),
            vec![call],
            signature.to_string(),
        );

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            fixture("deposit_wallet/wallet_submit_body.json")
        );
    }
}
