use std::fmt;

use ethers::types::Address;
use ethers::utils::to_checksum;
use serde::Serialize;

use crate::deposit_wallet::WALLET_TRANSACTION_TYPE;

const WALLET_NONCE_PATH: &str = "/nonce";
const GET_METHOD: &str = "GET";

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletNonceRequest {
    pub method: String,
    pub path: String,
    #[serde(serialize_with = "crate::deposit_wallet::types::serialize_address")]
    pub address: Address,
    #[serde(rename = "type")]
    pub nonce_type: String,
    pub path_and_query: String,
}

impl fmt::Debug for WalletNonceRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let address = redacted_address(self.address);
        let path_and_query = format!(
            "{WALLET_NONCE_PATH}?address={address}&type={}",
            self.nonce_type
        );

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
    let address = to_checksum(&owner, None);
    WalletNonceRequest {
        method: GET_METHOD.to_string(),
        path: WALLET_NONCE_PATH.to_string(),
        address: owner,
        nonce_type: WALLET_TRANSACTION_TYPE.to_string(),
        path_and_query: format!(
            "{WALLET_NONCE_PATH}?address={address}&type={WALLET_TRANSACTION_TYPE}"
        ),
    }
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}
