use std::{sync::Arc, time::Duration};

use axum::{
    extract::{Extension, Query}, response::{sse::{Event, KeepAlive}, IntoResponse, Sse}, BoxError, Json
};
use futures::{FutureExt, Stream, StreamExt};
use helix_datastore::Auctioneer;
use moka::sync::Cache;
use tracing::warn;
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;

use helix_common::{
    api::data_api::{
        BuilderBlocksReceivedParams, DeliveredPayloadsResponse, ProposerPayloadDeliveredParams,
        ReceivedBlocksResponse, ValidatorRegistrationParams,
    }, proofs::ConstraintsMessage, validator_preferences, ValidatorPreferences
};
use helix_database::DatabaseService;

use crate::{constraints::{self, api::ConstraintsHandle}, relay_data::error::DataApiError};

pub(crate) const PATH_DATA_API: &str = "/relay/v1/data";

pub(crate) const PATH_PROPOSER_PAYLOAD_DELIVERED: &str = "/bidtraces/proposer_payload_delivered";
pub(crate) const PATH_BUILDER_BIDS_RECEIVED: &str = "/bidtraces/builder_blocks_received";
pub(crate) const PATH_VALIDATOR_REGISTRATION: &str = "/validator_registration";

pub(crate) type BidsCache = Cache<String, Vec<ReceivedBlocksResponse>>;
pub(crate) type DeliveredPayloadsCache = Cache<String, Vec<DeliveredPayloadsResponse>>;

pub struct DataApi<A:Auctioneer, DB: DatabaseService> {
    validator_preferences: Arc<ValidatorPreferences>,
    auctioneer: Arc<A>,
    db: Arc<DB>,

    constraints_rx: broadcast::Receiver<ConstraintsMessage>,
}

impl<A: Auctioneer + 'static, DB: DatabaseService + 'static> DataApi<A, DB> {
    pub fn new(validator_preferences: Arc<ValidatorPreferences>, auctioneer:Arc<A>, db: Arc<DB>) -> (Self, ConstraintsHandle) {
        let (constraints_tx, constraints_rx ) = broadcast::channel(100); // Should we have buffer?
        (Self { 
            validator_preferences,
            auctioneer,
            db,
            constraints_rx, 
        },
        ConstraintsHandle {constraints_tx} )
    }

    /// Implements this API: <https://flashbots.github.io/relay-specs/#/Data/getDeliveredPayloads>
    pub async fn proposer_payload_delivered(
        Extension(data_api): Extension<Arc<DataApi<A, DB>>>,
        Extension(cache): Extension<Arc<DeliveredPayloadsCache>>,
        Query(params): Query<ProposerPayloadDeliveredParams>,
    ) -> Result<impl IntoResponse, DataApiError> {
        if params.slot.is_some() && params.cursor.is_some() {
            return Err(DataApiError::SlotAndCursor);
        }

        let cache_key = format!("{:?}", params);

        if let Some(cached_result) = cache.get(&cache_key) {
            return Ok(Json(cached_result));
        }

        match data_api
            .db
            .get_delivered_payloads(&params.into(), data_api.validator_preferences.clone())
            .await
        {
            Ok(result) => {
                let response = result
                    .into_iter()
                    .map(|b| b.into())
                    .collect::<Vec<DeliveredPayloadsResponse>>();

                cache.insert(cache_key, response.clone());

                Ok(Json(response))
            }
            Err(err) => {
                warn!(error=%err, "Failed to fetch delivered payloads");
                Err(DataApiError::InternalServerError)
            }
        }
    }

    /// Implements this API: <https://flashbots.github.io/relay-specs/#/Data/getReceivedBids>
    pub async fn builder_bids_received(
        Extension(data_api): Extension<Arc<DataApi<A, DB>>>,
        Extension(cache): Extension<Arc<BidsCache>>,
        Query(params): Query<BuilderBlocksReceivedParams>,
    ) -> Result<impl IntoResponse, DataApiError> {
        if params.slot.is_none()
            && params.block_hash.is_none()
            && params.block_number.is_none()
            && params.builder_pubkey.is_none()
        {
            return Err(DataApiError::MissingFilter);
        }

        if params.limit.is_some() && params.limit.unwrap() > 500 {
            return Err(DataApiError::LimitReached);
        }

        let cache_key = format!("{:?}", params);

        if let Some(cached_result) = cache.get(&cache_key) {
            return Ok(Json(cached_result));
        }

        match data_api.db.get_bids(&params.into()).await {
            Ok(result) => {
                let response =
                    result.into_iter().map(|b| b.into()).collect::<Vec<ReceivedBlocksResponse>>();

                cache.insert(cache_key, response.clone());

                Ok(Json(response))
            }
            Err(err) => {
                warn!(error=%err, "Failed to fetch bids");
                Err(DataApiError::InternalServerError)
            }
        }
    }

    /// Implements this API: <https://flashbots.github.io/relay-specs/#/Data/getValidatorRegistration>
    pub async fn validator_registration(
        Extension(data_api): Extension<Arc<DataApi<A, DB>>>,
        Query(params): Query<ValidatorRegistrationParams>,
    ) -> Result<impl IntoResponse, DataApiError> {
        match data_api.db.get_validator_registration(params.pubkey).await {
            Ok(result) => Ok(Json(result.registration_info.registration)),
            Err(err) => {
                warn!(error=%err, "Failed to get validator registration info");
                Err(DataApiError::InternalServerError)
            }
        }
    }

    /// Implements this API: <https://chainbound.github.io/bolt-docs/api/relay#constraints>
    pub async fn constraints(
        Extension(data_api): Extension<Arc<DataApi<A, DB>>>,
        Query(params): Query<usize>,
    ) -> Result<impl IntoResponse, DataApiError> {
        // TODO: Return err if the slot is outside expiry bounds i.e. before an epoch
        // For this we need a beacon state tracker (housekeep and submit block sub)
        let slot = params as u64;
        
        match data_api.auctioneer.get_constraints(slot).await {
            Ok(Some(constraints_with_proof_data)) => {
                let constraints = constraints_with_proof_data
                    .into_iter()
                    .map(|data| data.message)
                    .collect::<Vec<ConstraintsMessage>>();
        
                Ok(Json(constraints))
            }
            Ok(None) => {
                Ok(Json(vec![])) // Return an empty vector if no constraints are found
            }
            Err(err) => {
                warn!(error=%err, "Failed to get constraints");
                Err(DataApiError::ConstraintsError(slot))
            }
        }
    }

    /// Implements this API: <https://chainbound.github.io/bolt-docs/api/relay#constraints-stream>
    pub async fn constraints_stream(
        Extension(data_api): Extension<Arc<DataApi<A, DB>>>,
    ) -> Sse<impl Stream<Item = Result<Event, DataApiError>>> {
        let constraints_rx = data_api.constraints_rx.resubscribe();
        let stream = BroadcastStream::new(constraints_rx);

        let filtered = stream.map(|result| match result {
            Ok(constraint) => match serde_json::to_string(&constraint) {
                Ok(json) => Ok(Event::default()
                    .data(json)
                    .event("constraint")
                    .retry(Duration::from_millis(50))),
                Err(err) => {
                    warn!(error = %err, "Failed to serialize constraint");
                    Err(DataApiError::InternalServerError)
                }
            },
            Err(err) => {
                warn!(error = %err, "Error receiving constraint message");
                Err(DataApiError::InternalServerError)
            }
        });

        Sse::new(filtered).keep_alive(KeepAlive::default())
    }
}
