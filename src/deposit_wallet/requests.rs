use ethers::types::U256;

use crate::deposit_wallet::config::deposit_wallet_contract_chain_id;
use crate::deposit_wallet::{
    derive_deposit_wallet_address, validate_deposit_wallet_batch_signature,
    DepositWalletBatchRequest, DepositWalletBatchToSign, DepositWalletCall,
    DepositWalletContractConfig, DepositWalletCreateRequest, DepositWalletParams,
    DepositWalletRequestContext, WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};
use crate::error::{RelayerError, Result};

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

/// Builds a WALLET batch request from an owner-signed payload.
///
/// New callers should prefer this fallible compatibility entry point or
/// `build_deposit_wallet_batch_request_from_signed` so signer/config validation
/// failures are returned as `RelayerError` instead of producing an unchecked
/// request body.
pub fn try_build_wallet_batch_request_with_signature(
    ctx: DepositWalletRequestContext,
    config: DepositWalletContractConfig,
    nonce: U256,
    deadline: U256,
    calls: Vec<DepositWalletCall>,
    signature: String,
) -> Result<DepositWalletBatchRequest> {
    let chain_id = deposit_wallet_contract_chain_id(config)?;
    let derived_wallet = derive_deposit_wallet_address(ctx.owner_address, config)?;
    if ctx.deposit_wallet_address != derived_wallet {
        return Err(RelayerError::Signing(
            "deposit wallet request context wallet does not match owner/config derived wallet"
                .to_string(),
        ));
    }

    let batch = DepositWalletBatchToSign {
        owner: ctx.owner_address,
        nonce_owner: ctx.owner_address,
        submit_from: ctx.owner_address,
        deposit_wallet: ctx.deposit_wallet_address,
        chain_id,
        nonce,
        deadline,
        calls: calls.clone(),
    };
    validate_deposit_wallet_batch_signature(&batch, &signature)?;

    Ok(build_wallet_batch_request_unchecked(
        ctx, config, nonce, deadline, calls, signature,
    ))
}

/// Compatibility wrapper for the original public WALLET batch builder.
///
/// This preserves the existing function signature for consumers that have not
/// migrated yet, but it now performs the same owner signature, derived wallet,
/// signature shape, and batch resource preflight as the fallible builder before
/// constructing a request body.
#[deprecated(
    since = "0.1.3",
    note = "use try_build_wallet_batch_request_with_signature or build_deposit_wallet_batch_request_from_signed so signature/config preflight errors are returned instead of panicking"
)]
pub fn build_wallet_batch_request_with_signature(
    ctx: DepositWalletRequestContext,
    config: DepositWalletContractConfig,
    nonce: U256,
    deadline: U256,
    calls: Vec<DepositWalletCall>,
    signature: String,
) -> DepositWalletBatchRequest {
    try_build_wallet_batch_request_with_signature(ctx, config, nonce, deadline, calls, signature)
        .expect("deposit wallet WALLET batch compatibility builder preflight failed")
}

pub(crate) fn build_wallet_batch_request_unchecked(
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

    use super::build_wallet_batch_request_unchecked;
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

        let request = build_wallet_batch_request_unchecked(
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
