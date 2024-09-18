use axum::{
    body::{to_bytes, Body}, extract::ws::{Message, WebSocket, WebSocketUpgrade}, http::{request, Request, StatusCode}, response::{IntoResponse, Response}, Extension
};
use ethereum_consensus::{primitives::{BlsPublicKey, BlsSignature}, deneb::{verify_signed_data, Slot}, ssz};
use helix_common::ConstraintSubmissionTrace;
use helix_datastore::{error::AuctioneerError, Auctioneer};
use helix_utils::signing::verify_constraints_signature;
use tracing::{info, warn, error};
use uuid::Uuid;
use std::{collections::HashMap, sync::Arc};

use crate::proposer::api::get_nanos_timestamp;

use super::types::SignedConstraints;

#[derive(Debug, Default)]
pub struct ConstraintsApi <A>
where A: Auctioneer + 'static, 
{
    auctioneer: Arc<A>,
    constraints: HashMap<Slot, Vec<SignedConstraints>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConstraintApiError {
    #[error("Invalid constraints")]
    InvalidConstraints,
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("No constraints submitted")]
    NilConstraints,
    #[error("datastore error: {0}")]
    AuctioneerError(#[from] AuctioneerError),
}

impl<A> ConstraintsApi<A>
where A: Auctioneer + 'static,
{
    pub fn new() -> Self {
        Self { ..Default::default() }
    }

    /// Handles the submission of batch of signed constraints.
    pub async fn submit_constraints(
        Extension(api): Extension<Arc<ConstraintsApi<A>>>, 
        req: Request<Body>,
    ) -> Result<StatusCode, ConstraintApiError> {
        let request_id = Uuid::new_v4();
        let mut trace = 
            ConstraintSubmissionTrace {receive: get_nanos_timestamp()?, ..Default::default() };
        
        info!(
            request_id = %request_id,
            event = "submit_constraints",
            timestamp_request_start = trace.receive,
        );

        // Decode the incoming request body into a payload.
        let constraints = decode_constraints_submission(req, &mut trace, &request_id).await?;

        if constraints.is_empty() {
            return Err(ConstraintApiError::NilConstraints);
        }

        // Add all the constraints to the cache
        for signed_constraints in constraints {
            // Verify the signature.
            verify_constraints_signature(
                &mut signed_constraints.message,
                &signed_constraints.signature,
                &signed_constraints.message.pubkey,
                None);

            // Once we support sending messages signed with correct validator pubkey on the sidecar, 
            // return error if invalid

            let message = signed_constraints.message.clone();

            // Finally add the constraints to the redis cache
            api.save_constraints_to_auctioneer(&mut trace, message.slot, signed_constraints, &request_id);
        }
        // NOTE: Not really needed
        trace.cache = get_nanos_timestamp()?;

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

    pub async fn delegate() {
        // TODO: implement postgress db service function for storing delegated keys
        unimplemented!()
    }

    pub async fn revoke() {
        unimplemented!()
    }
}

// Helpers
impl<A> ConstraintsApi<A>
where A: Auctioneer + 'static,
{
    async fn save_constraints_to_auctioneer(
        &self,
        trace: &mut ConstraintSubmissionTrace,
        slot: Slot,
        signed_constraints: &Vec<SignedConstraints>,
        request_id: &Uuid,
    ) -> Result<(), ConstraintApiError> {
        // Save the constraints to the auctioneer
        match self.auctioneer
            .save_constraints(slot, signed_constraints)
            .await
        {
            Ok(()) => {
                trace.auctioneer = get_nanos_timestamp()?;
                info!(
                    request_id = %request_id,
                    timestamp_after_auctioneer = Instant::now().elapsed().as_nanos(),
                    auctioneer_latency_ns = trace.auctioneer.saturating_sub(trace.cache),
                    num_constraints = signed_constraints.len(),
                    "Constraints saved to auctioneer",
                );
                Ok(())
            }
            Err(err) => {
                error!(request_id = %request_id, error = %err, "Failed to save constraints to auctioneer");
                Err(ConstraintApiError::AuctioneerError(err))
            }
        }
    }
}

pub async fn decode_constraints_submission(
    req: Request<Body>,
    trace: &mut ConstraintSubmissionTrace,
    request_id: &Uuid,
) -> Result<Vec<SignedConstraints>, ConstraintApiError> {
    // Check if the request is SSZ encoded
    let is_ssz = req
        .headers()
        .get("Content-Type")
        .and_then(|val| val.to_str().ok())
        .map_or(false, |v| v == "application/octet-stream");

    // Read the body
    let body = req.into_body();
    let body_bytes = to_bytes(body, None).await?;

    
    // Decode the body
    let constraints: Vec<SignedConstraints> = if is_ssz {
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

    Ok(constraints)
}