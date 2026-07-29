use super::redaction::redacted_address;
use super::*;

/// Owner- and chain-scoped capability required by relayer HTTP read methods.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayerReadPermit {
    owner: Address,
    chain_id: u64,
}

impl RelayerReadPermit {
    /// Creates a read capability for one owner on one chain.
    pub fn for_owner(owner: Address, chain_id: u64) -> Self {
        Self { owner, chain_id }
    }

    /// Returns the owner bound to this capability.
    pub fn owner(&self) -> Address {
        self.owner
    }

    /// Returns the chain id bound to this capability.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

impl fmt::Debug for RelayerReadPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayerReadPermit")
            .field("owner", &redacted_address(self.owner))
            .field("chain_id", &self.chain_id)
            .finish()
    }
}

impl DepositWalletRelayerClient {
    pub(super) fn ensure_read_permit(
        &self,
        permit: &RelayerReadPermit,
        requested_owner: Address,
    ) -> Result<()> {
        if permit.owner != requested_owner {
            return Err(RelayerError::read_blocked(
                "read permit owner did not match requested owner".to_string(),
            ));
        }

        let configured_chain_id = deposit_wallet_contract_chain_id(self.config)?;
        if permit.chain_id != configured_chain_id {
            return Err(RelayerError::read_blocked(format!(
                "read permit chain {} did not match configured chain {configured_chain_id}",
                permit.chain_id
            )));
        }

        Ok(())
    }
}
