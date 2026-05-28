use thiserror::Error;

#[derive(Error, Debug)]
pub enum RelayerError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Relayer API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Signing error: {0}")]
    Signing(String),

    #[error("ABI encoding error: {0}")]
    Abi(String),

    #[error("Transaction failed: {0}")]
    TransactionFailed(String),

    #[error("Transaction invalid: {0}")]
    TransactionInvalid(String),

    #[error("Timeout waiting for transaction confirmation")]
    Timeout,

    #[error("Wallet not deployed: {0}")]
    WalletNotDeployed(String),

    #[error("Wallet already deployed: {0}")]
    WalletAlreadyDeployed(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),

    #[error("Auth error: {0}")]
    AuthError(String),

    #[error("Relayer quota exhausted (429)")]
    QuotaExhausted,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, RelayerError>;

impl RelayerError {
    pub(crate) fn invalid_relayer_url(message: impl Into<String>) -> Self {
        Self::Other(format!("Invalid relayer URL: {}", message.into()))
    }

    pub(crate) fn mutation_blocked(message: impl Into<String>) -> Self {
        Self::Other(format!("Deposit-wallet mutation blocked: {}", message.into()))
    }

    pub(crate) fn reconciliation_required(message: impl Into<String>) -> Self {
        Self::Other(format!(
            "Deposit-wallet reconciliation required: {}",
            message.into()
        ))
    }

    pub(crate) fn transaction_absent(message: impl Into<String>) -> Self {
        Self::Other(format!(
            "Deposit-wallet transaction temporarily absent: {}",
            message.into()
        ))
    }

    pub fn is_deposit_wallet_mutation_blocked(&self) -> bool {
        matches!(self, Self::Other(message) if message.starts_with("Deposit-wallet mutation blocked:"))
    }

    pub fn is_deposit_wallet_reconciliation_required(&self) -> bool {
        matches!(self, Self::Other(message) if message.starts_with("Deposit-wallet reconciliation required:"))
    }

    pub fn is_deposit_wallet_transaction_absent(&self) -> bool {
        matches!(self, Self::Other(message) if message.starts_with("Deposit-wallet transaction temporarily absent:"))
    }
}

#[cfg(test)]
mod tests {
    use super::RelayerError;

    #[test]
    fn deposit_wallet_error_classifiers_do_not_require_consumer_prefix_parsing() {
        assert!(RelayerError::mutation_blocked("locked").is_deposit_wallet_mutation_blocked());
        assert!(RelayerError::reconciliation_required("manual check").is_deposit_wallet_reconciliation_required());
        assert!(RelayerError::transaction_absent("not indexed").is_deposit_wallet_transaction_absent());
        assert!(!RelayerError::Other("other".to_string()).is_deposit_wallet_mutation_blocked());
        assert!(!RelayerError::Other("other".to_string()).is_deposit_wallet_transaction_absent());
    }
}
