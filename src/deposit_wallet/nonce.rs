use std::fmt;

use ethers::types::Address;
use ethers::utils::to_checksum;
use serde::{Serialize, Serializer};

use crate::deposit_wallet::WALLET_TRANSACTION_TYPE;

const WALLET_NONCE_PATH: &str = "/nonce";
const GET_METHOD: &str = "GET";

#[derive(Clone, PartialEq, Eq)]
pub struct WalletNonceRequest {
    method: String,
    path: String,
    address: Address,
    nonce_type: String,
}

impl WalletNonceRequest {
    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn nonce_type(&self) -> &str {
        &self.nonce_type
    }

    pub fn path_and_query(&self) -> String {
        wallet_nonce_path_and_query(self.address)
    }
}

impl Serialize for WalletNonceRequest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            method: String,
            path: String,
            #[serde(serialize_with = "crate::deposit_wallet::types::serialize_address")]
            address: Address,
            #[serde(rename = "type")]
            nonce_type: String,
            path_and_query: String,
        }

        Wire {
            method: self.method.clone(),
            path: self.path.clone(),
            address: self.address,
            nonce_type: self.nonce_type.clone(),
            path_and_query: self.path_and_query(),
        }
        .serialize(serializer)
    }
}

impl fmt::Debug for WalletNonceRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let address = redacted_address(self.address);
        let path_and_query =
            format!("{WALLET_NONCE_PATH}?address={address}&type={}", self.nonce_type);

        f.debug_struct("WalletNonceRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("address", &address)
            .field("type", &self.nonce_type)
            .field("path_and_query", &path_and_query)
            .finish()
    }
}

pub fn build_wallet_nonce_request(owner: Address) -> WalletNonceRequest {
    WalletNonceRequest {
        method: GET_METHOD.to_string(),
        path: WALLET_NONCE_PATH.to_string(),
        address: owner,
        nonce_type: WALLET_TRANSACTION_TYPE.to_string(),
    }
}

fn wallet_nonce_path_and_query(owner: Address) -> String {
    let address = to_checksum(&owner, None);
    format!("{WALLET_NONCE_PATH}?address={address}&type={WALLET_TRANSACTION_TYPE}")
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}
