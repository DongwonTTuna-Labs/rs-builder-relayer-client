use std::fmt;

use ethers::types::transaction::eip712::{Eip712, TypedData};
use ethers::types::{Address, H256, Signature, U256};
use ethers::utils::to_checksum;
use serde_json::{json, Value};

use crate::deposit_wallet::{
    deposit_wallet_contract_config, derive_deposit_wallet_address, DepositWalletBatchRequest,
    DepositWalletCall, DepositWalletContractConfig,
};
use crate::error::{RelayerError, Result};

const DEPOSIT_WALLET_DOMAIN_NAME: &str = "DepositWallet";
const DEPOSIT_WALLET_DOMAIN_VERSION: &str = "1";
const DEPOSIT_WALLET_PRIMARY_TYPE: &str = "Batch";
const ECDSA_SIGNATURE_HEX_LEN: usize = 132;
const ECDSA_SIGNATURE_PAYLOAD_HEX_LEN: usize = 130;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    typed_data: Value,
    digest: H256,
    signature: String,
    verified_signer: Address,
}

impl fmt::Debug for SignedDepositWalletBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedDepositWalletBatch")
            .field("owner", &self.owner)
            .field("nonce_owner", &self.nonce_owner)
            .field("submit_from", &self.submit_from)
            .field("deposit_wallet", &self.deposit_wallet)
            .field("chain_id", &self.chain_id)
            .field("nonce", &self.nonce)
            .field("deadline", &self.deadline)
            .field("calls_count", &self.calls.len())
            .field("typed_data", &"<redacted>")
            .field("digest", &self.digest)
            .field("signature", &"<redacted>")
            .field("verified_signer", &self.verified_signer)
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

    pub fn validate_submit_preflight(&self) -> Result<()> {
        let batch = self.batch_to_sign();
        validate_batch_identity(&batch)?;

        let expected_typed_data = build_deposit_wallet_batch_typed_data(&batch);
        if self.typed_data != expected_typed_data {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch typed data metadata was mutated".to_string(),
            ));
        }

        let expected_digest = digest_deposit_wallet_batch(&batch)?;
        if self.digest != expected_digest {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch digest metadata was mutated".to_string(),
            ));
        }

        let recovered = recover_deposit_wallet_batch_signer(&batch, &self.signature)?;
        if recovered != self.verified_signer {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch signer metadata was mutated".to_string(),
            ));
        }

        if self.verified_signer != self.owner {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch signer must match owner".to_string(),
            ));
        }

        Ok(())
    }

    fn batch_to_sign(&self) -> DepositWalletBatchToSign {
        DepositWalletBatchToSign {
            owner: self.owner,
            nonce_owner: self.nonce_owner,
            submit_from: self.submit_from,
            deposit_wallet: self.deposit_wallet,
            chain_id: self.chain_id,
            nonce: self.nonce,
            deadline: self.deadline,
            calls: self.calls.clone(),
        }
    }
}

pub fn build_deposit_wallet_batch_typed_data(batch: &DepositWalletBatchToSign) -> Value {
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
            "chainId": batch.chain_id,
            "verifyingContract": checksum(batch.deposit_wallet)
        },
        "message": {
            "wallet": checksum(batch.deposit_wallet),
            "nonce": batch.nonce.to_string(),
            "deadline": batch.deadline.to_string(),
            "calls": batch.calls.iter().map(call_to_typed_data).collect::<Vec<_>>()
        }
    })
}

pub fn digest_deposit_wallet_batch(batch: &DepositWalletBatchToSign) -> Result<H256> {
    let typed_data: TypedData = serde_json::from_value(build_deposit_wallet_batch_typed_data(batch))
        .map_err(|e| RelayerError::Signing(format!("invalid deposit wallet typed data: {e}")))?;
    let digest = typed_data
        .encode_eip712()
        .map_err(|e| RelayerError::Signing(format!("could not encode EIP-712 digest: {e}")))?;

    Ok(H256::from(digest))
}

pub fn recover_deposit_wallet_batch_signer(
    batch: &DepositWalletBatchToSign,
    signature: &str,
) -> Result<Address> {
    let digest = digest_deposit_wallet_batch(batch)?;
    recover_digest_signer(digest, signature)
}

pub fn validate_deposit_wallet_batch_signature(
    batch: &DepositWalletBatchToSign,
    signature: &str,
) -> Result<SignedDepositWalletBatch> {
    validate_batch_identity(batch)?;
    let digest = digest_deposit_wallet_batch(batch)?;
    let verified_signer = recover_digest_signer(digest, signature)?;
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
        typed_data: build_deposit_wallet_batch_typed_data(batch),
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

    Ok(crate::deposit_wallet::build_wallet_batch_request_with_signature(
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

fn validate_batch_identity(batch: &DepositWalletBatchToSign) -> Result<()> {
    if batch.owner != batch.nonce_owner {
        return Err(RelayerError::Signing(
            "deposit wallet nonce owner must match owner signer".to_string(),
        ));
    }

    if batch.owner != batch.submit_from {
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

fn recover_digest_signer(digest: H256, signature: &str) -> Result<Address> {
    validate_signature_shape(signature)?;
    let signature: Signature = signature
        .parse()
        .map_err(|e| RelayerError::Signing(format!("invalid deposit wallet signature: {e}")))?;
    signature
        .recover(digest)
        .map_err(|e| RelayerError::Signing(format!("could not recover deposit wallet signer: {e}")))
}

fn validate_signature_shape(signature: &str) -> Result<()> {
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

    Ok(())
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

    #[test]
    fn submit_preflight_rejects_private_metadata_tampering() {
        let signed = signed_fixture();
        let dead_address: Address = "0x000000000000000000000000000000000000dEaD"
            .parse()
            .unwrap();

        let mut tampered_owner = signed.clone();
        tampered_owner.owner = dead_address;
        assert!(tampered_owner.validate_submit_preflight().is_err());

        let mut tampered_domain = signed.clone();
        tampered_domain.typed_data["domain"]["verifyingContract"] =
            Value::String("0x000000000000000000000000000000000000dEaD".to_string());
        assert!(tampered_domain.validate_submit_preflight().is_err());

        let mut tampered_signer = signed.clone();
        tampered_signer.verified_signer = dead_address;
        assert!(tampered_signer.validate_submit_preflight().is_err());

        let mut tampered_digest = signed;
        tampered_digest.digest = H256::zero();
        assert!(tampered_digest.validate_submit_preflight().is_err());
    }
}
