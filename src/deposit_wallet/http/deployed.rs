use super::*;

#[derive(Deserialize)]
struct DeployedResponse {
    deployed: bool,
}

impl DepositWalletRelayerClient {
    /// Returns whether the owner's derived deposit wallet has been deployed.
    ///
    /// A `true` result is a read of deployment fact only. It does not establish
    /// submit readiness, which requires the separate `STATE_CONFIRMED` policy.
    pub async fn is_deposit_wallet_deployed(
        &self,
        owner: Address,
        permit: &RelayerReadPermit,
    ) -> Result<bool> {
        self.ensure_read_permit(permit, owner)?;

        let deposit_wallet = derive_deposit_wallet_address(owner, self.config)?;
        let mut url = self.base_url.endpoint(DEPLOYED_PATH);
        url.query_pairs_mut()
            .append_pair("address", &to_checksum(&deposit_wallet, None))
            .append_pair("type", WALLET_TRANSACTION_TYPE);
        let response = self.send(Method::GET, url, None).await?;
        parse_deployed_response(&response)
    }
}

fn parse_deployed_response(response: &[u8]) -> Result<bool> {
    serde_json::from_slice::<DeployedResponse>(response)
        .map(|response| response.deployed)
        .map_err(|_| RelayerError::Other("could not parse deployed response".to_string()))
}
