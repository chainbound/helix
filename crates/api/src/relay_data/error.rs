use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;
use serde_json;
use hyper;
use helix_datastore::error::AuctioneerError;

#[derive(Debug, Error)]
pub enum DataApiError {
    #[error("hyper error: {0}")]
    HyperError(#[from] hyper::Error),

    #[error("axum error: {0}")]
    AxumError(#[from] axum::Error),

    #[error("serde decode error: {0}")]
    SerializeError(#[from] serde_json::Error),

    #[error("cannot specify both slot and cursor")]
    SlotAndCursor,

    #[error("need to query for specific slot or block_hash or block_number or builder_pubkey")]
    MissingFilter,

    #[error("maximum limit is 500")]
    LimitReached,

    #[error("internal server error")]
    InternalServerError,

    #[error("failed to get constraints for slot {0}")]
    ConstraintsError(u64),

    #[error("incorrect slot for constraints request {0}")]
    IncorrectSlot(u64),

    #[error("datastore error: {0}")]
    AuctioneerError(#[from] AuctioneerError),
}

impl IntoResponse for DataApiError {
    fn into_response(self) -> Response {
        match self {
            DataApiError::HyperError(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Hyper error: {err}")).into_response()
            }
            DataApiError::AxumError(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Axum error: {err}")).into_response()
            }
            DataApiError::SerializeError(err) => {
                (StatusCode::BAD_REQUEST, format!("Serde error: {err}")).into_response()
            }
            DataApiError::SlotAndCursor => {
                (StatusCode::BAD_REQUEST, "cannot specify both slot and cursor").into_response()
            }
            DataApiError::MissingFilter => {
                (StatusCode::BAD_REQUEST, "need to query for specific slot or block_hash or block_number or builder_pubkey").into_response()
            }
            DataApiError::LimitReached => {
                (StatusCode::BAD_REQUEST, "maximum limit is 500").into_response()
            }
            DataApiError::InternalServerError => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
            DataApiError::ConstraintsError(slot) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to get constraints for slot {slot}")).into_response()
            }
            DataApiError::IncorrectSlot(slot) => {
                (StatusCode::BAD_REQUEST, format!("incorrect slot for constraints request {slot}")).into_response()
            }
            DataApiError::AuctioneerError(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("datastore error: {err}")).into_response()
            }
        }
    }
}