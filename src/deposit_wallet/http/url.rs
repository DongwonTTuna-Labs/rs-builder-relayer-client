use super::*;

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletRelayerUrl {
    base: Url,
    kind: DepositWalletRelayerUrlKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DepositWalletRelayerUrlKind {
    PolygonProduction,
    AmoyProduction,
    #[cfg(test)]
    MockLoopback,
}

impl DepositWalletRelayerUrl {
    /// Builds a production relayer URL.
    ///
    /// This validates the production host boundary only. PR #20 keeps
    /// production WALLET polling and nonce reads blocked until official or
    /// recorded deposit-wallet relayer response evidence is reviewed; live
    /// submit approval also needs durable owner state and a trusted capability
    /// outside this URL type.
    pub fn parse(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
        let kind = validate_rel_url(&url)?;
        Ok(Self { base: url, kind })
    }

    pub(super) fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(None);
        url
    }

    #[cfg(test)]
    pub(super) fn is_production_host(&self) -> bool {
        matches!(
            self.kind,
            DepositWalletRelayerUrlKind::PolygonProduction
                | DepositWalletRelayerUrlKind::AmoyProduction
        )
    }

    pub(super) fn required_chain_id(&self) -> Option<u64> {
        match self.kind {
            DepositWalletRelayerUrlKind::PolygonProduction => Some(POLYGON_CHAIN_ID),
            DepositWalletRelayerUrlKind::AmoyProduction => Some(AMOY_CHAIN_ID),
            #[cfg(test)]
            DepositWalletRelayerUrlKind::MockLoopback => None,
        }
    }

    #[cfg(test)]
    pub(super) fn loopback(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
        validate_mock_loopback_url(&url)?;
        Ok(Self {
            base: url,
            kind: DepositWalletRelayerUrlKind::MockLoopback,
        })
    }
}

impl fmt::Debug for DepositWalletRelayerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletRelayerUrl")
            .field(&self.base.as_str())
            .finish()
    }
}

pub(super) fn validate_rel_url(url: &Url) -> Result<DepositWalletRelayerUrlKind> {
    if url.scheme() != "https" {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must use https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include userinfo".to_string(),
        ));
    }
    let kind = match url.host_str() {
        Some(POLYGON_RELAYER_HOST) => DepositWalletRelayerUrlKind::PolygonProduction,
        Some(AMOY_RELAYER_HOST) => DepositWalletRelayerUrlKind::AmoyProduction,
        _ => {
            return Err(RelayerError::invalid_relayer_url(
                "relayer URL host is not allowlisted".to_string(),
            ));
        }
    };
    if !matches!(url.port(), None | Some(443)) {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must use the default HTTPS port".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include query or fragment".to_string(),
        ));
    }
    if url.path() != "/" {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include a path".to_string(),
        ));
    }
    Ok(kind)
}

#[cfg(test)]
pub(super) fn validate_mock_loopback_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must use http or https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must not include userinfo".to_string(),
        ));
    }
    if !matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("::1") | Some("[::1]")
    ) {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must be loopback-only".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must not include query or fragment".to_string(),
        ));
    }
    if url.path() != "/" {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must not include a path".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_relayer_contract_config(
    base_url: &DepositWalletRelayerUrl,
    config: DepositWalletContractConfig,
) -> Result<()> {
    let Some(chain_id) = base_url.required_chain_id() else {
        return Ok(());
    };

    if config != deposit_wallet_contract_config(chain_id)? {
        return Err(RelayerError::invalid_relayer_url(format!(
            "production deposit wallet relayer URL requires chain {chain_id} deposit wallet contract config"
        )));
    }
    Ok(())
}
