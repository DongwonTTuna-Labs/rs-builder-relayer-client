use super::*;

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletRelayerUrl {
    base: Url,
    kind: DepositWalletRelayerUrlKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DepositWalletRelayerUrlKind {
    Production,
    #[cfg(test)]
    MockLoopback,
}

impl DepositWalletRelayerUrl {
    /// Builds a production relayer URL.
    ///
    /// This PR keeps live submit mutations disabled for production URLs. The
    /// loopback submit transport is intentionally limited to crate-local tests
    /// until a later live-gate PR adds approved external integration hooks.
    pub fn parse(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
        validate_rel_url(&url)?;
        Ok(Self {
            base: url,
            kind: DepositWalletRelayerUrlKind::Production,
        })
    }

    pub(super) fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(None);
        url
    }

    pub(super) fn is_production_host(&self) -> bool {
        self.kind == DepositWalletRelayerUrlKind::Production
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

pub(super) fn validate_rel_url(url: &Url) -> Result<()> {
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
    if url.host_str() != Some(RELAYER_HOST) {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL host is not allowlisted".to_string(),
        ));
    }
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
    Ok(())
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
    if !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1" | "[::1]")) {
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
    if !base_url.is_production_host() {
        return Ok(());
    }
    if config != deposit_wallet_contract_config(POLYGON_CHAIN_ID)? {
        return Err(RelayerError::invalid_relayer_url(
            "production deposit wallet relayer URL requires Polygon deposit wallet contract config"
                .to_string(),
        ));
    }
    Ok(())
}
