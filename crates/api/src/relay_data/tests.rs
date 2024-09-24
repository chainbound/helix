#[cfg(test)]
mod data_api_tests {
    // *** IMPORTS ***
    use crate::{
        constraints::api::ConstraintsHandle, relay_data::{
            DataApi, PATH_BUILDER_BIDS_RECEIVED, PATH_DATA_API, PATH_PROPOSER_PAYLOAD_DELIVERED,
            PATH_VALIDATOR_REGISTRATION, PATH_CONSTRAINTS_API,
        }, test_utils::data_api_app
    };
    use ethereum_consensus::{builder::SignedValidatorRegistration, primitives::{BlsPublicKey, BlsSignature}};
    use futures::StreamExt;
    use helix_common::{
        api::data_api::{
            BuilderBlocksReceivedParams, DeliveredPayloadsResponse, ProposerPayloadDeliveredParams,
            ReceivedBlocksResponse, ValidatorRegistrationParams,
        }, bellatrix::List, proofs::{ConstraintsMessage, ConstraintsWithProofData, SignedConstraints}
    };
    use helix_database::MockDatabaseService;
    use helix_datastore::{Auctioneer, MockAuctioneer};
    use helix_utils::request_encoding::Encoding;
    use reqwest::{Client, Response, StatusCode};
    use reqwest_eventsource::{EventSource, Event as ReqwestEvent};
    use serial_test::serial;
    use tracing::info;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::{oneshot, RwLock};
    use async_trait::async_trait;

    // +++ HELPER VARIABLES +++
    const ADDRESS: &str = "0.0.0.0";
    const PORT: u16 = 3000;
    const HEAD_SLOT: u64 = 32;

    // +++ HELPER FUNCTIONS +++
    #[derive(Debug, Clone)]
    struct HttpServiceConfig {
        address: String,
        port: u16,
    }

    impl HttpServiceConfig {
        fn new(address: &str, port: u16) -> Self {
            HttpServiceConfig { address: address.to_string(), port }
        }

        fn base_url(&self) -> String {
            format!("http://{}:{}", self.address, self.port)
        }

        fn bind_address(&self) -> String {
            format!("{}:{}", self.address, self.port)
        }
    }

    async fn send_request(req_url: &str, encoding: Encoding, req_payload: Vec<u8>) -> Response {
        let client = Client::new();
        let request = client.post(req_url).header("accept", "*/*");
        let request = encoding.to_headers(request);

        request.body(req_payload).send().await.unwrap()
    }

    async fn start_api_server() -> (
        oneshot::Sender<()>,
        HttpServiceConfig,
        Arc<DataApi<MockAuctioneer, MockDatabaseService>>,
        Arc<MockAuctioneer>,
        Arc<MockDatabaseService>,
        ConstraintsHandle,
    ) {
        let (tx, rx) = oneshot::channel();
        let http_config = HttpServiceConfig::new(ADDRESS, PORT);
        let bind_address = http_config.bind_address();

        let (router, api, auctioneer, database, constraints_handle) = data_api_app();

        // Run the app in a background task
        tokio::spawn(async move {
            // run it with hyper on localhost:3000
            let listener = tokio::net::TcpListener::bind(bind_address).await.unwrap();
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
                .unwrap();
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        (tx, http_config, api, auctioneer, database, constraints_handle)
    }

    fn get_test_proposer_payload_delivered_params() -> ProposerPayloadDeliveredParams {
        ProposerPayloadDeliveredParams {
            slot: Some(HEAD_SLOT),
            cursor: None,
            limit: None,
            block_hash: None,
            block_number: None,
            proposer_pubkey: None,
            builder_pubkey: None,
            order_by: None,
        }
    }

    fn get_test_builder_blocks_received_params() -> BuilderBlocksReceivedParams {
        BuilderBlocksReceivedParams {
            slot: Some(HEAD_SLOT),
            block_hash: None,
            block_number: None,
            builder_pubkey: None,
            limit: None,
        }
    }

    fn get_test_validator_registration_params() -> ValidatorRegistrationParams {
        ValidatorRegistrationParams { pubkey: BlsPublicKey::default() }
    }

    // *** TESTS ***
    #[tokio::test]
    #[serial]
    async fn test_payload_delivered_slot_and_cursor() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, _handler) = start_api_server().await;

        // Prepare the request
        let req_url = format!(
            "{}{}{}",
            http_config.base_url(),
            PATH_DATA_API,
            PATH_PROPOSER_PAYLOAD_DELIVERED,
        );

        let mut query_params = get_test_proposer_payload_delivered_params();
        query_params.cursor = Some(HEAD_SLOT);

        // Send JSON encoded request
        let resp = reqwest::Client::new()
            .get(req_url.as_str())
            .header("accept", "application/json")
            .query(&query_params)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.text().await.unwrap(), "cannot specify both slot and cursor");

        // Shut down the server
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_payload_delivered_ok() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, _handler) = start_api_server().await;

        // Prepare the request
        let req_url = format!(
            "{}{}{}",
            http_config.base_url(),
            PATH_DATA_API,
            PATH_PROPOSER_PAYLOAD_DELIVERED,
        );

        let query_params = get_test_proposer_payload_delivered_params();

        // Send JSON encoded request
        let resp = reqwest::Client::new()
            .get(req_url.as_str())
            .header("accept", "application/json")
            .query(&query_params)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        // Deserialize the response into a Vec<DeliveredPayloadsResponse>
        let text = resp.text().await.unwrap();
        let _response: Vec<DeliveredPayloadsResponse> = serde_json::from_str(&text).unwrap();

        // Shut down the server
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_builder_bids_missing_filter() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, _handler) = start_api_server().await;

        // Prepare the request
        let req_url =
            format!("{}{}{}", http_config.base_url(), PATH_DATA_API, PATH_BUILDER_BIDS_RECEIVED,);

        let mut query_params = get_test_builder_blocks_received_params();
        query_params.slot = None;

        // Send JSON encoded request
        let resp = reqwest::Client::new()
            .get(req_url.as_str())
            .header("accept", "application/json")
            .query(&query_params)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            resp.text().await.unwrap(),
            "need to query for specific slot or block_hash or block_number or builder_pubkey"
        );

        // Shut down the server
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_builder_bids_limit_reached() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, _handler) = start_api_server().await;

        // Prepare the request
        let req_url =
            format!("{}{}{}", http_config.base_url(), PATH_DATA_API, PATH_BUILDER_BIDS_RECEIVED,);

        let mut query_params = get_test_builder_blocks_received_params();
        query_params.limit = Some(501);

        // Send JSON encoded request
        let resp = reqwest::Client::new()
            .get(req_url.as_str())
            .header("accept", "application/json")
            .query(&query_params)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.text().await.unwrap(), "maximum limit is 500");

        // Shut down the server
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_builder_bids_ok() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, _handler) = start_api_server().await;

        // Prepare the request
        let req_url =
            format!("{}{}{}", http_config.base_url(), PATH_DATA_API, PATH_BUILDER_BIDS_RECEIVED,);

        let query_params = get_test_builder_blocks_received_params();

        // Send JSON encoded request
        let resp = reqwest::Client::new()
            .get(req_url.as_str())
            .header("accept", "application/json")
            .query(&query_params)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        // Deserialize the response into a Vec<ReceivedBlocksResponse>
        let text = resp.text().await.unwrap();
        let _response: Vec<ReceivedBlocksResponse> = serde_json::from_str(&text).unwrap();

        // Shut down the server
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_validator_registration() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, _handler) = start_api_server().await;

        // Prepare the request
        let req_url =
            format!("{}{}{}", http_config.base_url(), PATH_DATA_API, PATH_VALIDATOR_REGISTRATION,);

        let query_params = get_test_validator_registration_params();

        // Send JSON encoded request
        let resp = reqwest::Client::new()
            .get(req_url.as_str())
            .header("accept", "application/json")
            .query(&query_params)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        // Deserialize the response into a SignedValidatorRegistration
        let text = resp.text().await.unwrap();
        let _response: SignedValidatorRegistration = serde_json::from_str(&text).unwrap();

        // Shut down the server
        let _ = tx.send(());
    }

    // New tests for constraints and constraints_stream

    // #[tokio::test]
    // #[serial]
    // async fn test_constraints_ok() {
    //     // Prepare test constraints
    //     let test_constraint = ConstraintsMessage {
    //         // Fill in fields as required for the test
    //         // Example:
    //         slot: HEAD_SLOT,
    //         data: vec![1, 2, 3],
    //     };

    //     let constraints_with_proof_data = ConstraintsWithProofData {
    //         message: test_constraint.clone(),
    //         proof: vec![],
    //     };

    //     let mock_auctioneer = Arc::new(MockAuctioneer {
    //         constraints: Arc::new(RwLock::new(Some(vec![constraints_with_proof_data]))),
    //     });

    //     // Start the server with the mock auctioneer
    //     let (tx, http_config, api, _database) =
    //         start_api_server_with_auctioneer(mock_auctioneer.clone()).await;

    //     // Set the head_slot
    //     *api.head_slot.write().await = HEAD_SLOT;

    //     // Prepare the request
    //     let req_url = format!(
    //         "{}{}{}",
    //         http_config.base_url(),
    //         PATH_DATA_API,
    //         "/constraints",
    //     );

    //     // Send JSON encoded request
    //     let resp = reqwest::Client::new()
    //         .get(req_url.as_str())
    //         .header("accept", "application/json")
    //         .send()
    //         .await
    //         .unwrap();

    //     assert_eq!(resp.status(), StatusCode::OK);
    //     // Deserialize the response into expected data type
    //     let text = resp.text().await.unwrap();
    //     let response: Vec<ConstraintsMessage> = serde_json::from_str(&text).unwrap();

    //     // Assert that the response matches the expected test data
    //     assert_eq!(response.len(), 1);
    //     assert_eq!(response[0], test_constraint);

    //     // Shut down the server
    //     let _ = tx.send(());
    // }

    // #[tokio::test]
    // #[serial]
    // async fn test_constraints_invalid_slot() {
    //     // Start the server
    //     let (tx, http_config, api, _auctioneer, _database, _handler) = start_api_server().await;

    //     // Set head_slot to 100
    //     *api.head_slot.write().await = 100;

    //     // Prepare the request with a slot beyond the head_slot
    //     let invalid_slot = 200;
    //     let req_url = format!(
    //         "{}{}{}?slot={}",
    //         http_config.base_url(),
    //         PATH_DATA_API,
    //         "/constraints",
    //         invalid_slot,
    //     );

    //     // Send JSON encoded request
    //     let resp = reqwest::Client::new()
    //         .get(req_url.as_str())
    //         .header("accept", "application/json")
    //         .send()
    //         .await
    //         .unwrap();

    //     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    //     // Check the error message
    //     let error_message = resp.text().await.unwrap();
    //     assert_eq!(error_message, format!("incorrect slot requested: {}", invalid_slot));

    //     // Shut down the server
    //     let _ = tx.send(());
    // }

    #[tokio::test]
    #[serial]
    async fn test_constraints_stream_ok() {
        // Start the server
        let (tx, http_config, _api, _auctioneer, _database, handler) = start_api_server().await;
    
        // Prepare the request URL
        let req_url = format!(
            "{}{}{}",
            http_config.base_url(),
            PATH_CONSTRAINTS_API,
            "/relay/v1/builder/constraints_stream",
        );
    
        // Start the SSE client
        let client = reqwest::Client::new();
        let req = client
            .get(&req_url)
            .header("accept", "text/event-stream");
    
        let mut event_source = EventSource::new(req).unwrap();
    
        // Prepare multiple signed constraints
        let test_constraints = vec![
            SignedConstraints {
                message: ConstraintsMessage {
                    pubkey: BlsPublicKey::default(),
                    slot: 0,
                    top: false,
                    transactions: List::default(),
                },
                signature: BlsSignature::default(),
            },
            SignedConstraints {
                message: ConstraintsMessage {
                    pubkey: BlsPublicKey::default(),
                    slot: 1,
                    top: true,
                    transactions: List::default(),
                },
                signature: BlsSignature::default(),
            },
            // Add more constraints as needed
        ];
    
        // Send the signed constraints
        for constraint in &test_constraints {
            handler.send_constraints(constraint.clone());
        }
    
        // Collect received constraints
        let mut received_constraints = Vec::new();
    
        // Read events from the SSE stream
        for _ in 0..test_constraints.len() {
            match event_source.next().await {
                Some(Ok(ReqwestEvent::Message(message))) => {
                    if message.event == "signed_constraint" {
                        let data = &message.data;
                        let received_constraint: SignedConstraints = serde_json::from_str(data).unwrap();
                        received_constraints.push(received_constraint);
                    }
                }
                Some(Ok(ReqwestEvent::Open)) => {
                    // Connection established
                }
                Some(Err(err)) => {
                    panic!("Error receiving SSE event: {:?}", err);
                }
                None => {
                    panic!("SSE stream closed unexpectedly");
                }
            }
        }
    
        // Assert that the received constraints match the sent constraints
        // assert_eq!(received_constraints[0], test_constraints[0]);
        info!("Received constraints: {:?}", received_constraints);
        info!("Sent constraints: {:?}", test_constraints);
    
        // Close the SSE client
        event_source.close();
    
        // Shut down the server
        let _ = tx.send(());
    }
}