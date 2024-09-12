use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade, Message},
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use ethereum_consensus::deneb::Slot;
use std::{collections::HashMap, sync::Arc};

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
    #[error("Invalid slot")]
    InvalidSlot,
    #[error("Invalid validator index")]
    InvalidValidatorIndex,
}

impl ConstraintsApi {
    pub fn new() -> Self {
        Self { ..Default::default() }
    }

    pub async fn submit_constraints(Extension(constraints_api): Extension<Arc<ConstraintsApi>>, 
    req: Request<Body>
    ) -> Result<StatusCode, ConstraintApiError> {
        // let constraints = to_bytes(req.into_body()).await?;
        // let constraints = serde_json::from_slice::<SignedConstraints>(&constraints)?;
        unimplemented!()
    }

    pub async fn delegate() {
        unimplemented!()
    }

    pub async fn revoke() {
        unimplemented!()
    }
}
