use std::fmt;

use ethers::types::{Address, Bytes, U256};
use ethers::utils::to_checksum;
use serde::{Serialize, Serializer};

pub const WALLET_CREATE_TRANSACTION_TYPE: &str = "WALLET-CREATE";
pub const WALLET_TRANSACTION_TYPE: &str = "WALLET";

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletRequestContext {
    pub owner_address: Address,
    pub deposit_wallet_address: Address,
}

impl fmt::Debug for DepositWalletRequestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let owner = redacted_address(self.owner_address);
        let deposit_wallet = redacted_address(self.deposit_wallet_address);

        f.debug_struct("DepositWalletRequestContext")
            .field("owner_address", &owner)
            .field("deposit_wallet_address", &deposit_wallet)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWalletCall {
    #[serde(serialize_with = "serialize_address")]
    pub target: Address,
    #[serde(serialize_with = "serialize_u256_decimal")]
    pub value: U256,
    #[serde(serialize_with = "serialize_bytes_hex")]
    pub data: Bytes,
}

impl fmt::Debug for DepositWalletCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = redacted_address(self.target);

        f.debug_struct("DepositWalletCall")
            .field("target", &target)
            .field("value", &self.value)
            .field("data", &"<redacted>")
            .field("data_len", &self.data.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DepositWalletCreateRequest {
    #[serde(rename = "type")]
    pub tx_type: String,
    #[serde(rename = "from", serialize_with = "serialize_address")]
    pub from_address: Address,
    #[serde(serialize_with = "serialize_address")]
    pub to: Address,
}

impl fmt::Debug for DepositWalletCreateRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let from = redacted_address(self.from_address);
        let to = redacted_address(self.to);

        f.debug_struct("DepositWalletCreateRequest")
            .field("type", &self.tx_type)
            .field("from", &from)
            .field("to", &to)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWalletParams {
    #[serde(serialize_with = "serialize_address")]
    pub(crate) deposit_wallet: Address,
    #[serde(serialize_with = "serialize_u256_decimal")]
    pub(crate) deadline: U256,
    pub(crate) calls: Vec<DepositWalletCall>,
}

impl fmt::Debug for DepositWalletParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let deposit_wallet = redacted_address(self.deposit_wallet);

        f.debug_struct("DepositWalletParams")
            .field("deposit_wallet", &deposit_wallet)
            .field("deadline", &self.deadline)
            .field("calls_count", &self.calls.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWalletBatchRequest {
    #[serde(rename = "type")]
    pub(crate) tx_type: String,
    #[serde(rename = "from", serialize_with = "serialize_address")]
    pub(crate) from_address: Address,
    #[serde(serialize_with = "serialize_address")]
    pub(crate) to: Address,
    #[serde(serialize_with = "serialize_u256_decimal")]
    pub(crate) nonce: U256,
    pub(crate) signature: String,
    pub(crate) deposit_wallet_params: DepositWalletParams,
}

impl fmt::Debug for DepositWalletBatchRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let from = redacted_address(self.from_address);
        let to = redacted_address(self.to);
        let deposit_wallet = redacted_address(self.deposit_wallet_params.deposit_wallet);

        f.debug_struct("DepositWalletBatchRequest")
            .field("type", &self.tx_type)
            .field("from", &from)
            .field("to", &to)
            .field("nonce", &self.nonce)
            .field("deposit_wallet", &deposit_wallet)
            .field("deadline", &self.deposit_wallet_params.deadline)
            .field("calls_count", &self.deposit_wallet_params.calls.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayerSubmitResponse {
    #[serde(rename = "transactionID", alias = "transactionId")]
    pub transaction_id: String,
    pub state: crate::deposit_wallet::RelayerTransactionState,
    #[serde(default, rename = "transactionHash")]
    pub transaction_hash: Option<String>,
}

pub(crate) fn serialize_address<S>(address: &Address, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&to_checksum(address, None))
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

pub(crate) fn serialize_u256_decimal<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

pub(crate) fn serialize_bytes_hex<S>(bytes: &Bytes, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format!("0x{}", hex::encode(bytes.as_ref())))
}
