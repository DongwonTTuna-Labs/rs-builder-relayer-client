use std::fmt;

use ethers::abi::{encode, Token};
use ethers::types::transaction::eip712::{Eip712, EIP712Domain};
use ethers::types::{Address, H256, Signature, U256};
use ethers::utils::{keccak256, to_checksum};
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
const DEPOSIT_WALLET_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const DEPOSIT_WALLET_CALL_TYPE: &str = "Call(address target,uint256 value,bytes data)";
const DEPOSIT_WALLET_BATCH_TYPE: &str =
    "Batch(address wallet,uint256 nonce,uint256 deadline,Call[] calls)Call(address target,uint256 value,bytes data)";
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

impl Eip712 for DepositWalletBatchToSign {
    type Error = RelayerError;

    fn domain(&self) -> Result<EIP712Domain> {
        Ok(EIP712Domain {
            name: Some(DEPOSIT_WALLET_DOMAIN_NAME.to_string()),
            version: Some(DEPOSIT_WALLET_DOMAIN_VERSION.to_string()),
            chain_id: Some(U256::from(self.chain_id)),
            verifying_contract: Some(self.deposit_wallet),
            salt: None,
        })
    }

    fn type_hash() -> Result<[u8; 32]> {
        Ok(keccak256(DEPOSIT_WALLET_BATCH_TYPE.as_bytes()))
    }

    /// Returns the canonical Batch struct hash without validating resource limits.
    ///
    /// Signing callers must run the separate resource preflight before invoking
    /// `Signer::sign_typed_data`.
    fn struct_hash(&self) -> Result<[u8; 32]> {
        Ok(hash_deposit_wallet_batch_struct(
            self.deposit_wallet,
            self.nonce,
            self.deadline,
            &self.calls,
        )
        .0)
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
    digest_deposit_wallet_batch_unchecked(batch)
}

fn digest_deposit_wallet_batch_unchecked(batch: &DepositWalletBatchToSign) -> Result<H256> {
    Ok(digest_deposit_wallet_batch_parts(
        batch.deposit_wallet,
        batch.chain_id,
        batch.nonce,
        batch.deadline,
        &batch.calls,
    ))
}

fn digest_deposit_wallet_batch_parts(
    deposit_wallet: Address,
    chain_id: u64,
    nonce: U256,
    deadline: U256,
    calls: &[DepositWalletCall],
) -> H256 {
    let domain_separator = hash_abi(&[
        type_hash_token(DEPOSIT_WALLET_DOMAIN_TYPE),
        string_hash_token(DEPOSIT_WALLET_DOMAIN_NAME),
        string_hash_token(DEPOSIT_WALLET_DOMAIN_VERSION),
        Token::Uint(U256::from(chain_id)),
        Token::Address(deposit_wallet),
    ]);
    let batch_hash =
        hash_deposit_wallet_batch_struct(deposit_wallet, nonce, deadline, calls);

    let mut digest_input = Vec::with_capacity(66);
    digest_input.extend_from_slice(b"\x19\x01");
    digest_input.extend_from_slice(domain_separator.as_bytes());
    digest_input.extend_from_slice(batch_hash.as_bytes());
    H256::from(keccak256(digest_input))
}

fn hash_deposit_wallet_batch_struct(
    deposit_wallet: Address,
    nonce: U256,
    deadline: U256,
    calls: &[DepositWalletCall],
) -> H256 {
    let calls_hash = hash_call_array(calls);
    hash_abi(&[
        type_hash_token(DEPOSIT_WALLET_BATCH_TYPE),
        Token::Address(deposit_wallet),
        Token::Uint(nonce),
        Token::Uint(deadline),
        fixed_hash_token(calls_hash),
    ])
}

fn hash_call_array(calls: &[DepositWalletCall]) -> H256 {
    let mut encoded_hashes = Vec::with_capacity(calls.len() * 32);
    for call in calls {
        encoded_hashes.extend_from_slice(hash_call(call).as_bytes());
    }
    H256::from(keccak256(encoded_hashes))
}

fn hash_call(call: &DepositWalletCall) -> H256 {
    hash_abi(&[
        type_hash_token(DEPOSIT_WALLET_CALL_TYPE),
        Token::Address(call.target),
        Token::Uint(call.value),
        bytes_hash_token(call.data.as_ref()),
    ])
}

fn hash_abi(tokens: &[Token]) -> H256 {
    H256::from(keccak256(encode(tokens)))
}

fn type_hash_token(type_name: &str) -> Token {
    fixed_hash_token(H256::from(keccak256(type_name.as_bytes())))
}

fn string_hash_token(value: &str) -> Token {
    fixed_hash_token(H256::from(keccak256(value.as_bytes())))
}

fn bytes_hash_token(value: &[u8]) -> Token {
    fixed_hash_token(H256::from(keccak256(value)))
}

fn fixed_hash_token(value: H256) -> Token {
    Token::FixedBytes(value.as_bytes().to_vec())
}

pub fn recover_deposit_wallet_batch_signer(
    batch: &DepositWalletBatchToSign,
    signature: &str,
) -> Result<Address> {
    let signature_payload = validate_signature_shape(signature)?;
    validate_batch_resource_limits(batch)?;
    let digest = digest_deposit_wallet_batch_unchecked(batch)?;
    recover_digest_signer_payload(digest, signature_payload)
}

pub fn validate_deposit_wallet_batch_signature(
    batch: DepositWalletBatchToSign,
    signature: &str,
) -> Result<SignedDepositWalletBatch> {
    let signature_payload = validate_signature_shape(signature)?;
    validate_batch_resource_limits(&batch)?;
    validate_deposit_wallet_batch_signature_parts(batch, signature, signature_payload)
}

pub(crate) fn validate_deposit_wallet_batch_signature_with_validated_resources(
    batch: DepositWalletBatchToSign,
    signature: &str,
) -> Result<SignedDepositWalletBatch> {
    let signature_payload = validate_signature_shape(signature)?;
    validate_deposit_wallet_batch_signature_parts(batch, signature, signature_payload)
}

fn validate_deposit_wallet_batch_signature_parts(
    batch: DepositWalletBatchToSign,
    signature: &str,
    signature_payload: &str,
) -> Result<SignedDepositWalletBatch> {
    validate_batch_identity(&batch)?;
    let digest = digest_deposit_wallet_batch_unchecked(&batch)?;
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
        calls: batch.calls,
        digest,
        signature: signature.to_string(),
        verified_signer,
    })
}

/// Builds a WALLET submit request from an already owner-validated signed batch.
///
/// This is a serialization and consistency guard for fixture-backed request
/// construction. It rechecks the signed owner, chain/config, and derived wallet,
/// but it intentionally does not apply wall-clock deadline freshness. Live
/// submit code must add a clock-injected expiry guard before calling it.
pub fn build_deposit_wallet_batch_request_from_signed(
    signed: SignedDepositWalletBatch,
    config: DepositWalletContractConfig,
) -> Result<DepositWalletBatchRequest> {
    signed.validate_submit_preflight()?;
    validate_submit_config(&signed, config)?;

    Ok(build_deposit_wallet_batch_request_from_prechecked_signed(
        signed, config,
    ))
}

pub(crate) fn build_deposit_wallet_batch_request_from_prechecked_signed(
    signed: SignedDepositWalletBatch,
    config: DepositWalletContractConfig,
) -> DepositWalletBatchRequest {
    build_wallet_batch_request_unchecked(
        crate::deposit_wallet::DepositWalletRequestContext {
            owner_address: signed.submit_from,
            deposit_wallet_address: signed.deposit_wallet,
        },
        config,
        signed.nonce,
        signed.deadline,
        signed.calls,
        signed.signature,
    )
}

fn validate_batch_resource_limits(batch: &DepositWalletBatchToSign) -> Result<()> {
    validate_deposit_wallet_batch_resource_limits(&batch.calls)
}

pub(crate) fn validate_deposit_wallet_batch_resource_limits(
    calls: &[DepositWalletCall],
) -> Result<()> {
    // A batch with no calls still signs, still submits, and still consumes the
    // owner's nonce. The relayer may well accept it, leaving a confirmed
    // transaction that did nothing and a nonce that later work has to account
    // for. There is no caller for whom that is the intended outcome.
    if calls.is_empty() {
        return Err(RelayerError::Signing(
            "deposit wallet batch must contain at least one call".to_string(),
        ));
    }

    if calls.len() > MAX_DEPOSIT_WALLET_BATCH_CALLS {
        return Err(RelayerError::Signing(format!(
            "deposit wallet batch call count exceeds maximum of {MAX_DEPOSIT_WALLET_BATCH_CALLS}"
        )));
    }

    let total_calldata_bytes =
        calls
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
        validate_deposit_wallet_batch_signature(batch, data["ownerSignature"].as_str().unwrap())
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
