use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade, Message},
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use ethereum_consensus::{deneb::Slot, ssz};
use helix_common::ConstraintSubmissionTrace;
use tracing::{info, warn};
use uuid::Uuid;
use std::{collections::HashMap, sync::Arc};

use crate::proposer::api::get_nanos_timestamp;

use super::types::SignedConstraints;

#[derive(Debug, Default)]
pub struct ConstraintsApi {
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
}

impl ConstraintsApi {
    pub fn new() -> Self {
        Self { ..Default::default() }
    }

    /// Handles the submission of batch of signed constraints.
    pub async fn submit_constraints(Extension(constraints_api): Extension<Arc<ConstraintsApi>>, 
    req: Request<Body>
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
        // NOTE: Maybe its better to use redis cache
        for signed_constraints in constraints {
            // TODO: Implement check_known_validators_by_index fn in the postgress_db_service
            // there is already check_known_validators by pubkey present
            // 
            // Get the bls pub key from the index
            // then verify the signature of the signed_constraints


            // From GO impl to remember -
            // TODO: uncomment this code once we send messages signed with correct validator pubkey on the sidecar.
            // We can for setup this for the devnet but it's not trivial so we'll skip it for now.
            // if !ok {
            // 	log.Error("Invalid BLS signature over constraint message")
            // 	api.RespondError(w, http.StatusBadRequest, fmt.Sprintf("Invalid BLS signature over constraint message %s", messageSSZ))
            // 	return
            // }

            let message = signed_constraints.message.clone();

            // Finally add the constraints to the cache
            if constraints_api.constraints.contains_key(&message.slot) {
                constraints_api.constraints.get_mut(&message.slot).unwrap().push(signed_constraints);
            } else {
                constraints_api.constraints.insert(message.slot, vec![signed_constraints]);
            }
        }
        // TODO: Not really needed
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