use ethers::types::{Address, Bytes, U256};
use ethers::utils::to_checksum;
use serde::{Serialize, Serializer};

pub const WALLET_CREATE_TRANSACTION_TYPE: &str = "WALLET-CREATE";
pub const WALLET_TRANSACTION_TYPE: &str = "WALLET";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositWalletRequestContext {
    pub owner_address: Address,
    pub deposit_wallet_address: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWalletCall {
    #[serde(serialize_with = "serialize_address")]
    pub target: Address,
    #[serde(serialize_with = "serialize_u256_decimal")]
    pub value: U256,
    #[serde(serialize_with = "serialize_bytes_hex")]
    pub data: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepositWalletCreateRequest {
    #[serde(rename = "type")]
    pub tx_type: String,
    #[serde(rename = "from", serialize_with = "serialize_address")]
    pub from_address: Address,
    #[serde(serialize_with = "serialize_address")]
    pub to: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWalletParams {
    #[serde(serialize_with = "serialize_address")]
    pub deposit_wallet: Address,
    #[serde(serialize_with = "serialize_u256_decimal")]
    pub deadline: U256,
    pub calls: Vec<DepositWalletCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWalletBatchRequest {
    #[serde(rename = "type")]
    pub tx_type: String,
    #[serde(rename = "from", serialize_with = "serialize_address")]
    pub from_address: Address,
    #[serde(serialize_with = "serialize_address")]
    pub to: Address,
    #[serde(serialize_with = "serialize_u256_decimal")]
    pub nonce: U256,
    pub signature: String,
    pub deposit_wallet_params: DepositWalletParams,
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
