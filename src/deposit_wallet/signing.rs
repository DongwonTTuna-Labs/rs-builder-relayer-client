use std::{collections::BTreeMap, fmt};

use ethers::types::transaction::eip712::{
    EIP712Domain, Eip712, Eip712DomainType, TypedData, Types,
};
use ethers::types::{Address, H256, Signature, U256};
use ethers::utils::to_checksum;
use serde_json::{json, Value};

use crate::deposit_wallet::{
    deposit_wallet_contract_config, derive_deposit_wallet_address, DepositWalletBatchRequest,
    DepositWalletCall, DepositWalletContractConfig,
};
use crate::deposit_wallet::requests::build_wallet_batch_request_unchecked;
use crate::error::{RelayerError, Result};

const DEPOSIT_WALLET_DOMAIN_NAME: &str = "DepositWallet";
const DEPOSIT_WALLET_DOMAIN_VERSION: &str = "1";
const DEPOSIT_WALLET_PRIMARY_TYPE: &str = "Batch";
const ECDSA_SIGNATURE_HEX_LEN: usize = 132;
const ECDSA_SIGNATURE_PAYLOAD_HEX_LEN: usize = 130;
const MAX_DEPOSIT_WALLET_BATCH_CALLS: usize = 256;
const MAX_DEPOSIT_WALLET_BATCH_CALLDATA_BYTES: usize = 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletBatchToSign {
    pub owner: Address,
    pub nonce_owner: Address,
    pub submit_from: Address,
    pub deposit_wallet: Address,
    pub chain_id: u64,
    pub nonce: U256,
    pub deadline: U256,
    pub calls: Vec<DepositWalletCall>,
}

impl fmt::Debug for DepositWalletBatchToSign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let owner = redacted_address(self.owner);
        let nonce_owner = redacted_address(self.nonce_owner);
        let submit_from = redacted_address(self.submit_from);
        let deposit_wallet = redacted_address(self.deposit_wallet);

        f.debug_struct("DepositWalletBatchToSign")
            .field("owner", &owner)
            .field("nonce_owner", &nonce_owner)
            .field("submit_from", &submit_from)
            .field("deposit_wallet", &deposit_wallet)
            .field("chain_id", &self.chain_id)
            .field("nonce", &self.nonce)
            .field("deadline", &self.deadline)
            .field("calls_count", &self.calls.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedDepositWalletBatch {
    owner: Address,
    nonce_owner: Address,
    submit_from: Address,
    deposit_wallet: Address,
    chain_id: u64,
    nonce: U256,
    deadline: U256,
    calls: Vec<DepositWalletCall>,
    digest: H256,
    signature: String,
    verified_signer: Address,
}

impl fmt::Debug for SignedDepositWalletBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let owner = redacted_address(self.owner);
        let nonce_owner = redacted_address(self.nonce_owner);
        let submit_from = redacted_address(self.submit_from);
        let deposit_wallet = redacted_address(self.deposit_wallet);
        let verified_signer = redacted_address(self.verified_signer);

        f.debug_struct("SignedDepositWalletBatch")
            .field("owner", &owner)
            .field("nonce_owner", &nonce_owner)
            .field("submit_from", &submit_from)
            .field("deposit_wallet", &deposit_wallet)
            .field("chain_id", &self.chain_id)
            .field("nonce", &self.nonce)
            .field("deadline", &self.deadline)
            .field("calls_count", &self.calls.len())
            .field("digest", &self.digest)
            .field("signature", &"<redacted>")
            .field("verified_signer", &verified_signer)
            .finish()
    }
}

impl SignedDepositWalletBatch {
    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn nonce_owner(&self) -> Address {
        self.nonce_owner
    }

    pub fn submit_from(&self) -> Address {
        self.submit_from
    }

    pub fn deposit_wallet(&self) -> Address {
        self.deposit_wallet
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn nonce(&self) -> U256 {
        self.nonce
    }

    pub fn deadline(&self) -> U256 {
        self.deadline
    }

    pub fn calls(&self) -> &[DepositWalletCall] {
        &self.calls
    }

    pub fn digest(&self) -> H256 {
        self.digest
    }

    pub fn verified_signer(&self) -> Address {
        self.verified_signer
    }

    fn validate_submit_preflight(&self) -> Result<()> {
        validate_batch_identity_parts(self.owner, self.nonce_owner, self.submit_from)?;

        if self.verified_signer != self.owner {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch signer must match owner".to_string(),
            ));
        }

        Ok(())
    }
}

/// Builds the DepositWallet Batch EIP-712 typed data after resource preflight.
pub fn try_build_deposit_wallet_batch_typed_data(batch: &DepositWalletBatchToSign) -> Result<Value> {
    validate_batch_resource_limits(batch)?;
    Ok(build_deposit_wallet_batch_typed_data_parts(
        batch.deposit_wallet,
        batch.chain_id,
        batch.nonce,
        batch.deadline,
        &batch.calls,
    ))
}

/// Compatibility wrapper for the original typed-data builder.
///
/// New callers should prefer `try_build_deposit_wallet_batch_typed_data` so
/// oversized batch errors are returned instead of panicking.
#[deprecated(
    since = "0.1.3",
    note = "use try_build_deposit_wallet_batch_typed_data so batch resource limit errors are returned instead of panicking"
)]
pub fn build_deposit_wallet_batch_typed_data(batch: &DepositWalletBatchToSign) -> Value {
    try_build_deposit_wallet_batch_typed_data(batch)
        .expect("deposit wallet typed-data compatibility builder preflight failed")
}

fn build_deposit_wallet_batch_typed_data_parts(
    deposit_wallet: Address,
    chain_id: u64,
    nonce: U256,
    deadline: U256,
    calls: &[DepositWalletCall],
) -> Value {
    json!({
        "primaryType": DEPOSIT_WALLET_PRIMARY_TYPE,
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" },
                { "name": "verifyingContract", "type": "address" }
            ],
            "Call": [
                { "name": "target", "type": "address" },
                { "name": "value", "type": "uint256" },
                { "name": "data", "type": "bytes" }
            ],
            "Batch": [
                { "name": "wallet", "type": "address" },
                { "name": "nonce", "type": "uint256" },
                { "name": "deadline", "type": "uint256" },
                { "name": "calls", "type": "Call[]" }
            ]
        },
        "domain": {
            "name": DEPOSIT_WALLET_DOMAIN_NAME,
            "version": DEPOSIT_WALLET_DOMAIN_VERSION,
            "chainId": chain_id,
            "verifyingContract": checksum(deposit_wallet)
        },
        "message": {
            "wallet": checksum(deposit_wallet),
            "nonce": nonce.to_string(),
            "deadline": deadline.to_string(),
            "calls": calls.iter().map(call_to_typed_data).collect::<Vec<_>>()
        }
    })
}

pub fn digest_deposit_wallet_batch(batch: &DepositWalletBatchToSign) -> Result<H256> {
    validate_batch_resource_limits(batch)?;
    digest_deposit_wallet_typed_data(build_deposit_wallet_batch_typed_data_model(
        batch.deposit_wallet,
        batch.chain_id,
        batch.nonce,
        batch.deadline,
        &batch.calls,
    ))
}

fn build_deposit_wallet_batch_typed_data_model(
    deposit_wallet: Address,
    chain_id: u64,
    nonce: U256,
    deadline: U256,
    calls: &[DepositWalletCall],
) -> TypedData {
    TypedData {
        domain: EIP712Domain {
            name: Some(DEPOSIT_WALLET_DOMAIN_NAME.to_string()),
            version: Some(DEPOSIT_WALLET_DOMAIN_VERSION.to_string()),
            chain_id: Some(U256::from(chain_id)),
            verifying_contract: Some(deposit_wallet),
            salt: None,
        },
        types: deposit_wallet_types(),
        primary_type: DEPOSIT_WALLET_PRIMARY_TYPE.to_string(),
        message: batch_message(deposit_wallet, nonce, deadline, calls),
    }
}

fn deposit_wallet_types() -> Types {
    let mut types = BTreeMap::new();
    types.insert(
        "EIP712Domain".to_string(),
        vec![
            eip712_field("name", "string"),
            eip712_field("version", "string"),
            eip712_field("chainId", "uint256"),
            eip712_field("verifyingContract", "address"),
        ],
    );
    types.insert(
        "Call".to_string(),
        vec![
            eip712_field("target", "address"),
            eip712_field("value", "uint256"),
            eip712_field("data", "bytes"),
        ],
    );
    types.insert(
        "Batch".to_string(),
        vec![
            eip712_field("wallet", "address"),
            eip712_field("nonce", "uint256"),
            eip712_field("deadline", "uint256"),
            eip712_field("calls", "Call[]"),
        ],
    );
    types
}

fn eip712_field(name: &str, r#type: &str) -> Eip712DomainType {
    Eip712DomainType {
        name: name.to_string(),
        r#type: r#type.to_string(),
    }
}

fn batch_message(
    deposit_wallet: Address,
    nonce: U256,
    deadline: U256,
    calls: &[DepositWalletCall],
) -> BTreeMap<String, Value> {
    let mut message = BTreeMap::new();
    message.insert("wallet".to_string(), Value::String(checksum(deposit_wallet)));
    message.insert("nonce".to_string(), Value::String(nonce.to_string()));
    message.insert("deadline".to_string(), Value::String(deadline.to_string()));
    message.insert(
        "calls".to_string(),
        Value::Array(calls.iter().map(call_to_typed_data).collect()),
    );
    message
}

fn digest_deposit_wallet_typed_data(typed_data: TypedData) -> Result<H256> {
    let digest = typed_data
        .encode_eip712()
        .map_err(|e| RelayerError::Signing(format!("could not encode EIP-712 digest: {e}")))?;

    Ok(H256::from(digest))
}

pub fn recover_deposit_wallet_batch_signer(
    batch: &DepositWalletBatchToSign,
    signature: &str,
) -> Result<Address> {
    let signature_payload = validate_signature_shape(signature)?;
    validate_batch_resource_limits(batch)?;
    let digest = digest_deposit_wallet_batch(batch)?;
    recover_digest_signer_payload(digest, signature_payload)
}

pub fn validate_deposit_wallet_batch_signature(
    batch: &DepositWalletBatchToSign,
    signature: &str,
) -> Result<SignedDepositWalletBatch> {
    let signature_payload = validate_signature_shape(signature)?;
    validate_batch_resource_limits(batch)?;
    validate_batch_identity(batch)?;
    let digest = digest_deposit_wallet_batch(batch)?;
    let verified_signer = recover_digest_signer_payload(digest, signature_payload)?;
    if verified_signer != batch.owner {
        return Err(RelayerError::Signing(
            "deposit wallet batch signer must match owner".to_string(),
        ));
    }

    Ok(SignedDepositWalletBatch {
        owner: batch.owner,
        nonce_owner: batch.nonce_owner,
        submit_from: batch.submit_from,
        deposit_wallet: batch.deposit_wallet,
        chain_id: batch.chain_id,
        nonce: batch.nonce,
        deadline: batch.deadline,
        calls: batch.calls.clone(),
        digest,
        signature: signature.to_string(),
        verified_signer,
    })
}

pub fn build_deposit_wallet_batch_request_from_signed(
    signed: &SignedDepositWalletBatch,
    config: DepositWalletContractConfig,
) -> Result<DepositWalletBatchRequest> {
    signed.validate_submit_preflight()?;
    validate_submit_config(signed, config)?;

    Ok(build_wallet_batch_request_unchecked(
        crate::deposit_wallet::DepositWalletRequestContext {
            owner_address: signed.submit_from,
            deposit_wallet_address: signed.deposit_wallet,
        },
        config,
        signed.nonce,
        signed.deadline,
        signed.calls.clone(),
        signed.signature.clone(),
    ))
}

fn validate_batch_resource_limits(batch: &DepositWalletBatchToSign) -> Result<()> {
    if batch.calls.len() > MAX_DEPOSIT_WALLET_BATCH_CALLS {
        return Err(RelayerError::Signing(format!(
            "deposit wallet batch call count exceeds maximum of {MAX_DEPOSIT_WALLET_BATCH_CALLS}"
        )));
    }

    let total_calldata_bytes =
        batch
            .calls
            .iter()
            .try_fold(0usize, |total, call| match total.checked_add(call.data.len()) {
                Some(next) => Ok(next),
                None => Err(RelayerError::Signing(
                    "deposit wallet batch calldata byte count overflowed".to_string(),
                )),
            })?;

    if total_calldata_bytes > MAX_DEPOSIT_WALLET_BATCH_CALLDATA_BYTES {
        return Err(RelayerError::Signing(format!(
            "deposit wallet batch calldata bytes exceed maximum of {MAX_DEPOSIT_WALLET_BATCH_CALLDATA_BYTES}"
        )));
    }

    Ok(())
}

fn validate_batch_identity(batch: &DepositWalletBatchToSign) -> Result<()> {
    validate_batch_identity_parts(batch.owner, batch.nonce_owner, batch.submit_from)
}

fn validate_batch_identity_parts(
    owner: Address,
    nonce_owner: Address,
    submit_from: Address,
) -> Result<()> {
    if owner != nonce_owner {
        return Err(RelayerError::Signing(
            "deposit wallet nonce owner must match owner signer".to_string(),
        ));
    }

    if owner != submit_from {
        return Err(RelayerError::Signing(
            "deposit wallet submit from must match owner signer".to_string(),
        ));
    }

    Ok(())
}

fn validate_submit_config(
    signed: &SignedDepositWalletBatch,
    config: DepositWalletContractConfig,
) -> Result<()> {
    let expected_config = deposit_wallet_contract_config(signed.chain_id)?;
    if config != expected_config {
        return Err(RelayerError::Signing(
            "signed deposit wallet batch submit config does not match signed chain id".to_string(),
        ));
    }

    let derived_wallet = derive_deposit_wallet_address(signed.owner, config)?;
    if signed.deposit_wallet != derived_wallet {
        return Err(RelayerError::Signing(
            "signed deposit wallet batch wallet does not match owner/config derived wallet"
                .to_string(),
        ));
    }

    Ok(())
}

fn recover_digest_signer_payload(digest: H256, signature_payload: &str) -> Result<Address> {
    let signature: Signature = signature_payload
        .parse()
        .map_err(|e| RelayerError::Signing(format!("invalid deposit wallet signature: {e}")))?;
    signature
        .recover(digest)
        .map_err(|e| RelayerError::Signing(format!("could not recover deposit wallet signer: {e}")))
}

fn validate_signature_shape(signature: &str) -> Result<&str> {
    let Some(hex_payload) = signature.strip_prefix("0x") else {
        return Err(RelayerError::Signing(
            "deposit wallet signature must be 0x-prefixed 65-byte hex".to_string(),
        ));
    };

    if signature.len() != ECDSA_SIGNATURE_HEX_LEN
        || hex_payload.len() != ECDSA_SIGNATURE_PAYLOAD_HEX_LEN
        || !hex_payload.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RelayerError::Signing(
            "deposit wallet signature must be 0x-prefixed 65-byte hex".to_string(),
        ));
    }

    Ok(hex_payload)
}

fn call_to_typed_data(call: &DepositWalletCall) -> Value {
    json!({
        "target": checksum(call.target),
        "value": call.value.to_string(),
        "data": bytes_hex(&call.data)
    })
}

fn checksum(address: Address) -> String {
    to_checksum(&address, None)
}

fn redacted_address(address: Address) -> String {
    let checksum = checksum(address);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

fn bytes_hex(bytes: &ethers::types::Bytes) -> String {
    format!("0x{}", hex::encode(bytes.as_ref()))
}

#[cfg(test)]
mod tests {
    use ethers::types::Bytes;

    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/deposit_wallet/wallet_batch_eip712.json"
        ))
        .expect("fixture should be valid JSON")
    }

    fn parse_address(value: &Value, key: &str) -> Address {
        value[key].as_str().unwrap().parse().unwrap()
    }

    fn parse_u256(value: &Value, key: &str) -> U256 {
        U256::from_dec_str(value[key].as_str().unwrap()).unwrap()
    }

    fn call_from_value(value: &Value) -> DepositWalletCall {
        DepositWalletCall {
            target: value["target"].as_str().unwrap().parse().unwrap(),
            value: U256::from_dec_str(value["value"].as_str().unwrap()).unwrap(),
            data: Bytes::from(hex::decode(&value["data"].as_str().unwrap()[2..]).unwrap()),
        }
    }

    fn batch_from_fixture(data: &Value) -> DepositWalletBatchToSign {
        DepositWalletBatchToSign {
            owner: parse_address(data, "owner"),
            nonce_owner: parse_address(data, "nonceOwner"),
            submit_from: parse_address(data, "submitFrom"),
            deposit_wallet: parse_address(data, "depositWallet"),
            chain_id: data["chainId"].as_u64().unwrap(),
            nonce: parse_u256(data, "nonce"),
            deadline: parse_u256(data, "deadline"),
            calls: data["calls"]
                .as_array()
                .unwrap()
                .iter()
                .map(call_from_value)
                .collect(),
        }
    }

    fn signed_fixture() -> SignedDepositWalletBatch {
        let data = fixture();
        let batch = batch_from_fixture(&data);
        validate_deposit_wallet_batch_signature(
            &batch,
            data["ownerSignature"].as_str().unwrap(),
        )
        .unwrap()
    }

    fn assert_signing_error_contains(result: Result<()>, expected: &str) {
        match result {
            Err(RelayerError::Signing(message)) => assert!(
                message.contains(expected),
                "expected signing error containing {expected:?}, got {message:?}"
            ),
            Err(error) => panic!("expected signing error containing {expected:?}, got {error:?}"),
            Ok(_) => panic!("expected signing error containing {expected:?}, got success"),
        }
    }

    #[test]
    fn submit_preflight_rejects_private_metadata_tampering() {
        let signed = signed_fixture();
        let dead_address: Address = "0x000000000000000000000000000000000000dEaD"
            .parse()
            .unwrap();

        let mut tampered_owner = signed.clone();
        tampered_owner.owner = dead_address;
        assert_signing_error_contains(
            tampered_owner.validate_submit_preflight(),
            "nonce owner must match owner signer",
        );

        let mut tampered_signer = signed.clone();
        tampered_signer.verified_signer = dead_address;
        assert_signing_error_contains(
            tampered_signer.validate_submit_preflight(),
            "signer must match owner",
        );
    }
}
