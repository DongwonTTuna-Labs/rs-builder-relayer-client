use ethers::signers::Signer;
use ethers::types::U256;

use super::{
    DepositWalletRelayerClient, RelayerMutationOperation, RelayerMutationPermit,
    RelayerReadPermit, RelayerSubmitOutcome,
};
use crate::deposit_wallet::signing::validate_deposit_wallet_batch_resource_limits;
use crate::deposit_wallet::{
    derive_deposit_wallet_address, try_build_wallet_batch_request_with_signature,
    DepositWalletBatchToSign, DepositWalletCall, DepositWalletRequestContext,
};
use crate::error::{RelayerError, Result};

impl DepositWalletRelayerClient {
    /// Fetches a fresh WALLET nonce, signs the batch, validates it, and submits it.
    ///
    /// Dry-run execution still performs the nonce read and local signature so
    /// its evidence records the fresh nonce. Callers must prevent concurrent
    /// executions for the same owner; an owner-scoped lease is deferred to a
    /// subsequent change.
    pub async fn execute_wallet_batch<S>(
        &self,
        ctx: DepositWalletRequestContext,
        calls: Vec<DepositWalletCall>,
        deadline: U256,
        signer: &S,
        read_permit: &RelayerReadPermit,
        mutation_permit: &RelayerMutationPermit,
    ) -> Result<RelayerSubmitOutcome>
    where
        S: Signer,
    {
        let (chain_id, now_unix) = self.validate_mutation_permit(
            mutation_permit,
            RelayerMutationOperation::WalletBatch,
            ctx.owner_address,
        )?;
        self.ensure_read_permit(read_permit, ctx.owner_address)?;

        if U256::from(now_unix) >= deadline {
            return Err(RelayerError::mutation_blocked(
                "batch deadline expired before signing",
            ));
        }

        if signer.address() != ctx.owner_address {
            return Err(RelayerError::Signing(
                "batch signer address did not match deposit wallet owner".to_string(),
            ));
        }

        validate_deposit_wallet_batch_resource_limits(&calls)?;
        let derived_wallet = derive_deposit_wallet_address(ctx.owner_address, self.config)?;
        if derived_wallet != ctx.deposit_wallet_address {
            return Err(RelayerError::Signing(
                "deposit wallet request context wallet does not match owner/config derived wallet"
                    .to_string(),
            ));
        }

        let nonce = self
            .get_wallet_nonce(ctx.owner_address, read_permit)
            .await?;
        let batch = DepositWalletBatchToSign {
            owner: ctx.owner_address,
            nonce_owner: ctx.owner_address,
            submit_from: ctx.owner_address,
            deposit_wallet: ctx.deposit_wallet_address,
            chain_id,
            nonce,
            deadline,
            calls,
        };
        let signature = signer
            .sign_typed_data(&batch)
            .await
            .map_err(|_| RelayerError::Signing("batch signing failed".to_string()))?;
        let signature = format!("0x{signature}");

        let request = try_build_wallet_batch_request_with_signature(
            ctx,
            self.config,
            nonce,
            deadline,
            batch.calls.clone(),
            signature,
        )?;

        self.submit_signed_wallet_batch(request, mutation_permit)
            .await
    }
}
