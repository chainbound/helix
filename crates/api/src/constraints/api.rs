use axum::{
    body::{to_bytes, Body}, extract::ws::{Message, WebSocket, WebSocketUpgrade}, http::{request, Request, StatusCode}, response::{IntoResponse, Response}, Extension
};
use ethereum_consensus::{primitives::{BlsPublicKey, BlsSignature}, deneb::{verify_signed_data, Slot}, ssz};
use helix_common::{api::constraints_api::{SignedDelegation, SignedRevocation, MAX_CONSTRAINTS_PER_SLOT}, bellatrix::List, chain_info::ChainInfo, proofs::{ConstraintsMessage, ConstraintsWithProofData, ProofError, SignedConstraints, SignedConstraintsWithProofData}, ConstraintSubmissionTrace};
use helix_database::DatabaseService;
use helix_datastore::{error::AuctioneerError, Auctioneer};
use helix_utils::signing::verify_signed_builder_message as verify_signature;
use tracing::{info, warn, error};
use uuid::Uuid;
use std::{sync::Arc, time::{SystemTime, UNIX_EPOCH}};
use tokio::{sync::broadcast, time::Instant};

use crate::constraints::error::ConstraintsApiError;

use super::error::Conflict;

// This is the maximum length (randomly chosen) of a request body in bytes.
pub(crate) const MAX_REQUEST_LENGTH: usize = 1024 * 1024 * 5;

#[derive(Clone)]
pub struct ConstraintsApi <A, DB>
where 
    A: Auctioneer + 'static,
    DB: DatabaseService + 'static,
{
    auctioneer: Arc<A>,
    db: Arc<DB>,
    chain_info: Arc<ChainInfo>,

    constraints_handle: ConstraintsHandle,
}

#[derive(Clone)]
pub struct ConstraintsHandle {
    pub(crate) constraints_tx: broadcast::Sender<ConstraintsMessage>, 
}

impl ConstraintsHandle {
    pub fn send_constraints(&self, constraints: ConstraintsMessage) {
        if let Err(err) = self.constraints_tx.send(constraints) {
            error!(?err, "Failed to send constraints to the constraints channel");
        }
    }
}

impl<A, DB> ConstraintsApi <A, DB>
where 
    A: Auctioneer + 'static,
    DB: DatabaseService + 'static,
{
    pub fn new(
        auctioneer: Arc<A>,
        db: Arc<DB>,
        chain_info: Arc<ChainInfo>,
        constraints_handle: ConstraintsHandle ,
    ) -> Self {
        Self { auctioneer, db, chain_info, constraints_handle }
    }

    /// Handles the submission of batch of signed constraints.
    /// 
    /// Implements this API: <https://chainbound.github.io/bolt-docs/api/builder#constraints>
    pub async fn submit_constraints(
        Extension(api): Extension<Arc<ConstraintsApi<A, DB>>>, 
        req: Request<Body>,
    ) -> Result<StatusCode, ConstraintsApiError> {
        let request_id = Uuid::new_v4();
        let mut trace = 
            ConstraintSubmissionTrace { receive: get_nanos_timestamp()?, ..Default::default() };
        
        info!(
            request_id = %request_id,
            event = "submit_constraints",
            timestamp_request_start = trace.receive,
        );

        // Decode the incoming request body into a payload.
        let constraints = decode_constraints_submission(req, &mut trace, &request_id).await?;

        if constraints.is_empty() {
            return Err(ConstraintsApiError::NilConstraints);
        }

        // Add all the constraints to the cache
        for mut signed_constraints in constraints {
            let pubkey = signed_constraints.message.pubkey.clone();
            let message = &mut signed_constraints.message;
            
            // Check for conflicts in the constraints
            let saved_constraints = api.auctioneer.get_constraints(message.slot).await?;
            if let Some(conflict) = conflicts_with(saved_constraints, message) {
                return Err(ConstraintsApiError::Conflict(conflict));
            }

            // Verify the signature.
            if let Err(_) = verify_signature(
                message,
                &signed_constraints.signature,
                &pubkey,
                &api.chain_info.context
            ) {
                return Err(ConstraintsApiError::InvalidSignature);
            };

            // Once we support sending messages signed with correct validator pubkey on the sidecar, 
            // return error if invalid

            let message = signed_constraints.message.clone();
            let slot = message.slot;

            // Send to the constraints channel
            api.constraints_handle.send_constraints(message);

            // Finally add the constraints to the redis cache
            if let Err(err) = api.save_constraints_to_auctioneer(
                &mut trace,
                slot,
                signed_constraints,
                &request_id
            ).await {
                error!(request_id = %request_id, error = %err, "Failed to save constraints to auctioneer");
            };
        }

        // Log some final info
        trace.request_finish = get_nanos_timestamp()?;
        info!(
            request_id = %request_id,
            trace = ?trace,
            request_duration_ns = trace.request_finish.saturating_sub(trace.receive),
            "submit_constraints request finished",
        );

        Ok(StatusCode::OK)
    }

    /// Handles delegating constraint submission rights to another BLS key.
    /// 
    /// Implements this API: <https://chainbound.github.io/bolt-docs/api/builder#delegate>
    pub async fn delegate(
        Extension(api): Extension<Arc<ConstraintsApi<A, DB>>>,
        req: Request<Body>,
    ) -> Result<StatusCode, ConstraintsApiError> {
        let request_id = Uuid::new_v4();
        let mut trace = ConstraintSubmissionTrace {
            receive: get_nanos_timestamp()?,
            ..Default::default()
        };

        info!(
            request_id = %request_id,
            event = "delegate",
            timestamp_request_start = trace.receive,
        );

        // Read the body
        let body = req.into_body();
        let body_bytes = to_bytes(body, MAX_REQUEST_LENGTH).await?;
        
        // Decode the incoming request body into a `SignedDelegation`.
        let mut signed_delegation: SignedDelegation = match serde_json::from_slice(&body_bytes) {
            Ok(delegation) => delegation,
            Err(_) => return Err(ConstraintsApiError::InvalidDelegation),
        };
        trace.decode = get_nanos_timestamp()?;

        let pubkey = signed_delegation.message.validator_pubkey.clone();
        let message = &mut signed_delegation.message;
        
        // Verify the delegation signature
        if let Err(e) = verify_signature(
            message,
            &signed_delegation.signature,
            &pubkey,
            &api.chain_info.context
        ) {
            return Err(ConstraintsApiError::InvalidSignature);
        };
        trace.verify_signature = get_nanos_timestamp()?;

        // Store the delegation in the database
        tokio::spawn( async move {
            if let Err(err) = api.db.save_validator_delegation(signed_delegation).await {
                error!(
                    error = %err,
                    "Failed to save delegation",
                )
            }
        });

        // Log some final info
        trace.request_finish = get_nanos_timestamp()?;
        info!(
            request_id = %request_id,
            trace = ?trace,
            request_duration_ns = trace.request_finish.saturating_sub(trace.receive),
            "delegate request finished",
        );
        
        Ok(StatusCode::OK)
    }


    /// Handles revoking constraint submission rights from a BLS key.
    /// 
    /// Implements this API: <https://chainbound.github.io/bolt-docs/api/builder#revoke>
    pub async fn revoke(
        Extension(api): Extension<Arc<ConstraintsApi<A, DB>>>,
        req: Request<Body>,
    ) -> Result<StatusCode, ConstraintsApiError> {
        let request_id = Uuid::new_v4();
        let mut trace = ConstraintSubmissionTrace {
            receive: get_nanos_timestamp()?,
            ..Default::default()
        };

        info!(
            request_id = %request_id,
            event = "revoke",
            timestamp_request_start = trace.receive,
        );

        // Read the body
        let body = req.into_body();
        let body_bytes = to_bytes(body, MAX_REQUEST_LENGTH).await?;
        
        // Decode the incoming request body into a `SignedDelegation`.
        let mut signed_revocation: SignedRevocation = match serde_json::from_slice(&body_bytes) {
            Ok(revocation ) => revocation,
            Err(_) => return Err(ConstraintsApiError::InvalidRevocation),
        };
        trace.decode = get_nanos_timestamp()?;

        let pubkey = signed_revocation.message.validator_pubkey.clone();
        let message = &mut signed_revocation.message;
        // Verify the revocation signature
        if let Err(e) = verify_signature(
            message,
            &signed_revocation.signature,
            &pubkey,
            &api.chain_info.context
        ) {
            return Err(ConstraintsApiError::InvalidSignature);
        };
        trace.verify_signature = get_nanos_timestamp()?;

        // Store the delegation in the database
        tokio::spawn( async move {
            if let Err(err) = api.db.revoke_validator_delegation(signed_revocation).await {
                error!(
                    error = %err,
                    "Failed to do revocation",
                )
            }
        });

        // Log some final info
        trace.request_finish = get_nanos_timestamp()?;
        info!(
            request_id = %request_id,
            trace = ?trace,
            request_duration_ns = trace.request_finish.saturating_sub(trace.receive),
            "revoke request finished",
        );
        
        Ok(StatusCode::OK)
    }
}

// Helpers
impl<A, DB> ConstraintsApi<A, DB>
where 
    A: Auctioneer + 'static,
    DB: DatabaseService + 'static,
{
    async fn save_constraints_to_auctioneer(
        &self,
        trace: &mut ConstraintSubmissionTrace,
        slot: Slot,
        signed_constraints: SignedConstraints,
        request_id: &Uuid,
    ) -> Result<(), ConstraintsApiError> {
        let message_with_data = SignedConstraintsWithProofData::try_from(signed_constraints)?;
        match self.auctioneer
            .save_constraints(slot, message_with_data)
            .await
        {
            Ok(()) => {
                trace.auctioneer_update = get_nanos_timestamp()?;
                info!(
                    request_id = %request_id,
                    timestamp_after_auctioneer = Instant::now().elapsed().as_nanos(),
                    auctioneer_latency_ns = trace.auctioneer_update.saturating_sub(trace.decode),
                    "Constraints saved to auctioneer",
                );
                Ok(())
            }
            Err(err) => {
                error!(request_id = %request_id, error = %err, "Failed to save constraints to auctioneer");
                Err(ConstraintsApiError::AuctioneerError(err))
            }
        }
    }
}

/// Checks if the constraints for the given slot conflict with the existing constraints.
/// Returns a [Conflict] in case of a conflict, None otherwise.
///
/// # Possible conflicts
/// - Multiple ToB constraints per slot
/// - Duplicates of the same transaction per slot
pub fn conflicts_with(
    saved_constraints: Option<Vec<SignedConstraintsWithProofData>>, 
    constraints: &ConstraintsMessage
) -> Option<Conflict> {
    // Check if there are saved constraints to compare against
    if let Some(saved_constraints) = saved_constraints {
        for saved_constraint in saved_constraints {
            // Only 1 ToB (Top of Block) constraint per slot
            if constraints.top && saved_constraint.signed_constraints.message.top {
                return Some(Conflict::TopOfBlock);
            }

            // Check if any of the transactions are the same
            for tx in constraints.transactions.iter() {
                if saved_constraint.signed_constraints.message.transactions.iter().any(|existing| tx == existing) {
                    return Some(Conflict::DuplicateTransaction);
                }
            }
        }
    }

    None
}

pub async fn decode_constraints_submission(
    req: Request<Body>,
    trace: &mut ConstraintSubmissionTrace,
    request_id: &Uuid,
) -> Result<Vec<SignedConstraints>, ConstraintsApiError> {
    // Check if the request is SSZ encoded
    let is_ssz = req
        .headers()
        .get("Content-Type")
        .and_then(|val| val.to_str().ok())
        .map_or(false, |v| v == "application/octet-stream");

    // Read the body
    let body = req.into_body();
    let body_bytes = to_bytes(body, MAX_REQUEST_LENGTH).await?;
    
    // Decode the body
    let constraints: List<SignedConstraints, MAX_CONSTRAINTS_PER_SLOT> = if is_ssz {
        match ssz::prelude::deserialize(&body_bytes){
            Ok(constraints) => constraints,
            Err(err) => {
                // Fallback for JSON
                warn!(request_id = %request_id, error = %err, "Failed to decode SSZ constraints, falling back to JSON");
                serde_json::from_slice(&body_bytes)?
            } 
        }
    } else {
        serde_json::from_slice(&body_bytes)?
    };

    trace.decode = get_nanos_timestamp()?;
    info!(
        request_id = %request_id,
        timestamp_after_decoding = Instant::now().elapsed().as_nanos(),
        decode_latency_ns = trace.decode.saturating_sub(trace.receive),
        num_constraints = constraints.len(),
    );

    Ok(constraints.to_vec())
}

fn get_nanos_timestamp() -> Result<u64, ConstraintsApiError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .map_err(|_| ConstraintsApiError::InternalError)
}