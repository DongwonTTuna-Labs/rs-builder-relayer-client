use ethers::types::transaction::eip712::{Eip712, TypedData};
use ethers::types::{Address, H256, Signature, U256};
use ethers::utils::to_checksum;
use serde_json::{json, Value};

use crate::deposit_wallet::{DepositWalletBatchRequest, DepositWalletCall};
use crate::error::{RelayerError, Result};

const DEPOSIT_WALLET_DOMAIN_NAME: &str = "DepositWallet";
const DEPOSIT_WALLET_DOMAIN_VERSION: &str = "1";
const DEPOSIT_WALLET_PRIMARY_TYPE: &str = "Batch";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSignerScope {
    DepositWalletBatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSignerSource {
    TrustedConfig(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedSessionSigner {
    pub signer: Address,
    pub owner: Address,
    pub scope: SessionSignerScope,
    pub expires_at: U256,
    pub source: SessionSignerSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerAuthorization {
    Owner,
    TrustedSessionSigner(ApprovedSessionSigner),
}

impl SignerAuthorization {
    pub fn is_owner(&self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn trusted_session_signer(&self) -> Option<&ApprovedSessionSigner> {
        match self {
            Self::Owner => None,
            Self::TrustedSessionSigner(signer) => Some(signer),
        }
    }

    fn authorizes(&self, owner: Address, deadline: U256, signer: Address) -> bool {
        match self {
            Self::Owner => signer == owner,
            Self::TrustedSessionSigner(approved) => approved.authorizes(owner, deadline, signer),
        }
    }
}

impl ApprovedSessionSigner {
    fn authorizes(&self, owner: Address, deadline: U256, signer: Address) -> bool {
        self.signer == signer
            && self.owner == owner
            && self.scope == SessionSignerScope::DepositWalletBatch
            && self.expires_at >= deadline
            && matches!(self.source, SessionSignerSource::TrustedConfig(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedDepositWalletBatch {
    pub owner: Address,
    pub nonce_owner: Address,
    pub submit_from: Address,
    pub deposit_wallet: Address,
    pub chain_id: u64,
    pub nonce: U256,
    pub deadline: U256,
    pub calls: Vec<DepositWalletCall>,
    pub typed_data: Value,
    pub digest: H256,
    pub signature: String,
    pub verified_signer: Address,
    pub signer_authorization: SignerAuthorization,
}

impl SignedDepositWalletBatch {
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

        if !self
            .signer_authorization
            .authorizes(self.owner, self.deadline, self.verified_signer)
        {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch signer authorization is not valid".to_string(),
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
    approved_session_signers: &[ApprovedSessionSigner],
) -> Result<SignedDepositWalletBatch> {
    validate_batch_identity(batch)?;
    let digest = digest_deposit_wallet_batch(batch)?;
    let verified_signer = recover_digest_signer(digest, signature)?;
    let signer_authorization =
        signer_authorization(batch, verified_signer, approved_session_signers)?;

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
        signer_authorization,
    })
}

pub fn build_deposit_wallet_batch_request_from_signed(
    signed: &SignedDepositWalletBatch,
    config: crate::deposit_wallet::DepositWalletContractConfig,
) -> Result<DepositWalletBatchRequest> {
    signed.validate_submit_preflight()?;

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

fn signer_authorization(
    batch: &DepositWalletBatchToSign,
    verified_signer: Address,
    approved_session_signers: &[ApprovedSessionSigner],
) -> Result<SignerAuthorization> {
    if verified_signer == batch.owner {
        return Ok(SignerAuthorization::Owner);
    }

    approved_session_signers
        .iter()
        .find(|approved| approved.authorizes(batch.owner, batch.deadline, verified_signer))
        .cloned()
        .map(SignerAuthorization::TrustedSessionSigner)
        .ok_or_else(|| {
            RelayerError::Signing(
                "deposit wallet batch signer is neither owner nor approved session signer"
                    .to_string(),
            )
        })
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

fn recover_digest_signer(digest: H256, signature: &str) -> Result<Address> {
    let signature: Signature = signature
        .parse()
        .map_err(|e| RelayerError::Signing(format!("invalid deposit wallet signature: {e}")))?;
    signature
        .recover(digest)
        .map_err(|e| RelayerError::Signing(format!("could not recover deposit wallet signer: {e}")))
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
