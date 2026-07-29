use std::sync::atomic::Ordering;

use serde::Serialize;

use super::mutation::payload_keccak256;
use super::response::validate_transaction_id;
use super::*;
use crate::deposit_wallet::requests::build_wallet_create_request;
use crate::deposit_wallet::types::{
    DepositWalletBatchRequest, WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};

const INVALID_SUBMIT_RESPONSE_MESSAGE: &str = "submit response did not include a valid transactionID; the request may have been accepted and must be reconciled before any resubmission";

impl DepositWalletRelayerClient {
    pub async fn submit_wallet_create(
        &self,
        owner: Address,
        permit: &RelayerMutationPermit,
    ) -> Result<RelayerSubmitOutcome> {
        let (chain_id, _) = self.validate_mutation_permit(
            permit,
            RelayerMutationOperation::WalletCreate,
            owner,
        )?;

        match permit.mode() {
            RelayerMutationMode::DryRun => {
                let request = build_wallet_create_request(owner, self.config);
                let body = serialize_submit_request(&request, WALLET_CREATE_TRANSACTION_TYPE)?;
                let deposit_wallet = derive_deposit_wallet_address(owner, self.config)?;
                let evidence = DepositWalletDryRunEvidence::for_wallet_create(
                    permit,
                    chain_id,
                    owner,
                    deposit_wallet,
                    request.to,
                    &body,
                );
                Ok(RelayerSubmitOutcome::DryRun(Box::new(evidence)))
            }
            RelayerMutationMode::Live => {
                self.ensure_mutation_enabled()?;
                let request = build_wallet_create_request(owner, self.config);
                let body = serialize_submit_request(&request, WALLET_CREATE_TRANSACTION_TYPE)?;
                let payload_keccak256 = payload_keccak256(&body);
                self.submit_serialized_body(body, payload_keccak256).await
            }
        }
    }

    pub async fn submit_signed_wallet_batch(
        &self,
        request: DepositWalletBatchRequest,
        permit: &RelayerMutationPermit,
    ) -> Result<RelayerSubmitOutcome> {
        let (chain_id, now_unix) = self.validate_mutation_permit(
            permit,
            RelayerMutationOperation::WalletBatch,
            request.from_address,
        )?;

        if U256::from(now_unix) >= request.deposit_wallet_params.deadline {
            return Err(RelayerError::mutation_blocked(
                "batch deadline expired before submit",
            ));
        }

        match permit.mode() {
            RelayerMutationMode::DryRun => {
                let body = serialize_submit_request(&request, WALLET_TRANSACTION_TYPE)?;
                let evidence = DepositWalletDryRunEvidence::for_wallet_batch(
                    permit, chain_id, &request, &body,
                );
                Ok(RelayerSubmitOutcome::DryRun(Box::new(evidence)))
            }
            RelayerMutationMode::Live => {
                self.ensure_mutation_enabled()?;
                let body = serialize_submit_request(&request, WALLET_TRANSACTION_TYPE)?;
                let payload_keccak256 = payload_keccak256(&body);
                self.submit_serialized_body(body, payload_keccak256).await
            }
        }
    }

    pub(super) fn validate_mutation_permit(
        &self,
        permit: &RelayerMutationPermit,
        expected_operation: RelayerMutationOperation,
        requested_owner: Address,
    ) -> Result<(u64, u64)> {
        if permit.operation() != expected_operation {
            return Err(RelayerError::mutation_blocked(
                "mutation permit operation did not match requested operation",
            ));
        }
        if permit.owner() != requested_owner {
            return Err(RelayerError::mutation_blocked(
                "mutation permit owner did not match requested owner",
            ));
        }

        let configured_chain_id = deposit_wallet_contract_chain_id(self.config)?;
        if permit.chain_id() != configured_chain_id {
            return Err(RelayerError::mutation_blocked(format!(
                "mutation permit chain {} did not match configured chain {configured_chain_id}",
                permit.chain_id()
            )));
        }

        let now_unix = self.clock.now_unix();
        if now_unix >= permit.expires_at_unix() {
            return Err(RelayerError::mutation_blocked(
                "mutation permit expired before submit",
            ));
        }

        Ok((configured_chain_id, now_unix))
    }

    fn ensure_mutation_enabled(&self) -> Result<()> {
        if !self.mutation_gate.load(Ordering::SeqCst) {
            return Err(RelayerError::mutation_blocked(
                "relayer mutation is disabled for this client",
            ));
        }
        Ok(())
    }

    async fn submit_serialized_body(
        &self,
        body: Vec<u8>,
        payload_keccak256: String,
    ) -> Result<RelayerSubmitOutcome> {
        let body = String::from_utf8(body).map_err(|_| {
            RelayerError::Other("serialized submit request was not valid UTF-8".to_string())
        })?;
        let url = self.base_url.endpoint(SUBMIT_PATH);
        let response = self
            .send(Method::POST, url, Some(body))
            .await
            .map_err(classify_submit_error)?;
        let receipt = parse_submit_response(&response, payload_keccak256)?;
        Ok(RelayerSubmitOutcome::Submitted(receipt))
    }
}

fn serialize_submit_request<T: Serialize>(request: &T, operation: &str) -> Result<Vec<u8>> {
    serde_json::to_vec(request).map_err(|_| {
        RelayerError::Other(format!(
            "could not serialize {operation} submit request"
        ))
    })
}

fn classify_submit_error(error: RelayerError) -> RelayerError {
    match error {
        RelayerError::Http(_) => RelayerError::reconciliation_required(
            "submit transport failed after dispatch; reconcile before any resubmission",
        ),
        RelayerError::Other(message) if message == RESPONSE_BODY_TOO_LARGE_MESSAGE => {
            RelayerError::reconciliation_required(
                "submit response exceeded the maximum size; reconcile before any resubmission",
            )
        }
        other => other,
    }
}

fn parse_submit_response(
    response: &[u8],
    payload_keccak256: String,
) -> Result<DepositWalletSubmitReceipt> {
    let response = serde_json::from_slice::<RelayerSubmitResponse>(response)
        .map_err(|_| RelayerError::reconciliation_required(INVALID_SUBMIT_RESPONSE_MESSAGE))?;
    let transaction_id = validate_transaction_id(&response.transaction_id)
        .map_err(|_| RelayerError::reconciliation_required(INVALID_SUBMIT_RESPONSE_MESSAGE))?;

    Ok(DepositWalletSubmitReceipt::new(
        transaction_id,
        response.state,
        payload_keccak256,
    ))
}
