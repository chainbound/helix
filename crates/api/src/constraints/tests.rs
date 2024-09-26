use rand::thread_rng;
#[cfg(test)]
mod tests {
    // +++ IMPORTS +++

    use std::{sync::Arc, time::Duration};

    use axum::body::Body;
    use ethereum_consensus::{builder::ValidatorRegistration, primitives::{BlsPublicKey, BlsSignature}, ssz};
    use helix_common::{api::{builder_api::{BuilderGetValidatorsResponse, BuilderGetValidatorsResponseEntry}, constraints_api::{SignedDelegation, SignedRevocation}, proposer_api::ValidatorRegistrationInfo}, bellatrix::{ByteVector, List}, deneb::SignedValidatorRegistration, proofs::SignedConstraints, Route, ValidatorPreferences};
    use helix_common::api::constraints_api::MAX_CONSTRAINTS_PER_SLOT;
    use helix_database::MockDatabaseService;
    use helix_datastore::MockAuctioneer;
    use helix_housekeeper::{ChainUpdate, SlotUpdate};
    use helix_utils::request_encoding::Encoding;
    use hyper::Request;
    use rand::Rng;
    use reqwest::{Client, Response};
    use serial_test::serial;
    use tokio::sync::{mpsc::{Receiver, Sender}, oneshot};
    use tracing::info;
    use reth_primitives::hex;

    use crate::{builder::{api::BuilderApi, mock_simulator::MockSimulator}, constraints::api::ConstraintsApi, gossiper::mock_gossiper::MockGossiper, test_utils::constraints_api_app};
    
    // +++ HELPER VARIABLES +++
    const ADDRESS: &str = "0.0.0.0";
    const PORT: u16 = 3000;
    const HEAD_SLOT: u64 = 32; //ethereum_consensus::configs::mainnet::CAPELLA_FORK_EPOCH;
    const SUBMISSION_SLOT: u64 = HEAD_SLOT + 1;
    const SUBMISSION_TIMESTAMP: u64 = 1606824419;
    const VALIDATOR_INDEX: usize = 1;
    const PUB_KEY: &str = "0xa695ad325dfc7e1191fbc9f186f58eff42a634029731b18380ff89bf42c464a42cb8ca55b200f051f57f1e1893c68759";

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

    fn get_test_pub_key_bytes(random: bool) -> [u8; 48] {
        if random {
            let mut pubkey_array = [0u8; 48];
            rand::thread_rng().fill(&mut pubkey_array[..]);
            pubkey_array
        } else {
            let pubkey_bytes = hex::decode(&PUB_KEY[2..]).unwrap();
            let mut pubkey_array = [0u8; 48];
            pubkey_array.copy_from_slice(&pubkey_bytes);
            pubkey_array
        }
    }

    fn get_byte_vector_20_for_hex(hex: &str) -> ByteVector<20> {
        let bytes = hex::decode(&hex[2..]).unwrap();
        ByteVector::try_from(bytes.as_ref()).unwrap()
    }

    fn get_byte_vector_32_for_hex(hex: &str) -> ByteVector<32> {
        let bytes = hex::decode(&hex[2..]).unwrap();
        ByteVector::try_from(bytes.as_ref()).unwrap()
    }

    fn get_valid_payload_register_validator(
        submission_slot: Option<u64>,
        validator_index: Option<usize>,
    ) -> BuilderGetValidatorsResponseEntry {
        BuilderGetValidatorsResponseEntry {
            slot: submission_slot.unwrap_or(SUBMISSION_SLOT),
            validator_index: validator_index.unwrap_or(VALIDATOR_INDEX),
            entry: ValidatorRegistrationInfo {
                registration: SignedValidatorRegistration {
                    message: ValidatorRegistration {
                        fee_recipient: get_byte_vector_20_for_hex("0x5cc0dde14e7256340cc820415a6022a7d1c93a35"),
                        gas_limit: 30000000,
                        timestamp: SUBMISSION_TIMESTAMP,
                        public_key: BlsPublicKey::try_from(&get_test_pub_key_bytes(false)[..]).unwrap(),
                    },
                    signature: BlsSignature::try_from(hex::decode(&"0xaf12df007a0c78abb5575067e5f8b089cfcc6227e4a91db7dd8cf517fe86fb944ead859f0781277d9b78c672e4a18c5d06368b603374673cf2007966cece9540f3a1b3f6f9e1bf421d779c4e8010368e6aac134649c7a009210780d401a778a5"[2..]).unwrap().as_slice()).unwrap(),
                },
                preferences: ValidatorPreferences::default(),
            }
        }
    }
    
    fn get_dummy_slot_update(
        head_slot: Option<u64>,
        submission_slot: Option<u64>,
        validator_index: Option<usize>,
    ) -> SlotUpdate {
        SlotUpdate {
            slot: head_slot.unwrap_or(HEAD_SLOT),
            next_duty: Some(get_valid_payload_register_validator(submission_slot, validator_index)),
            new_duties: Some(vec![get_valid_payload_register_validator(
                submission_slot,
                validator_index,
            )]),
        }
    }

    async fn send_dummy_slot_update(
        slot_update_sender: Sender<ChainUpdate>,
        head_slot: Option<u64>,
        submission_slot: Option<u64>,
        validator_index: Option<usize>,
    ) {
        let chain_update = ChainUpdate::SlotUpdate(get_dummy_slot_update(
            head_slot,
            submission_slot,
            validator_index,
        ));
        slot_update_sender.send(chain_update).await.unwrap();

        // sleep for a bit to allow the api to process the slot update
        tokio::time::sleep(Duration::from_millis(100)).await;
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
        Arc<ConstraintsApi<MockAuctioneer, MockDatabaseService>>,
        Arc<BuilderApi<MockAuctioneer, MockDatabaseService, MockSimulator, MockGossiper>>,
        Receiver<Sender<ChainUpdate>>
    ) {
        let (tx, rx) = oneshot::channel();
        let http_config = HttpServiceConfig::new(ADDRESS, PORT);
        let bind_address = http_config.bind_address();

        let (router, constraints_api, builder_api, slot_update_receiver ) = constraints_api_app();

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

        (tx, http_config, constraints_api, builder_api, slot_update_receiver)
    }

    fn _get_signed_constraints_json() -> &'static str {
        r#"[
            {
            "message": {
                "pubkey": "0xa695ad325dfc7e1191fbc9f186f58eff42a634029731b18380ff89bf42c464a42cb8ca55b200f051f57f1e1893c68759",
                "slot": 32,
                "top": false,
                "transactions": [
                "0x02f86c870c72dd9d5e883e4d0183408f2382520894d2e2adf7177b7a8afddbc12d1634cf23ea1a71020180c001a08556dcfea479b34675db3fe08e29486fe719c2b22f6b0c1741ecbbdce4575cc6a01cd48009ccafd6b9f1290bbe2ceea268f94101d1d322c787018423ebcbc87ab4",
                "0x02f86c870c72dd9d5e883e4d0183408f2382520894d2e2adf7177b7a8afddbc12d1634cf23ea1a71020180c001a08556dcfea479b34675db3fe08e29486fe719c2b22f6b0c1741ecbbdce4575cc6a01cd48009ccafd6b9f1290bbe2ceea268f94101d1d322c787018423ebcbc87ab4"
                ]
            },
            "signature": "0xae5aa93391a256eebef79fe452951ae196b3b3ac9046e45cd63713a57ad0548ddb56430477cb3d70287710984fc4bc4e091da1d5594de25c2caeca4872b35c12587ef168b0c878cde4025d66d4195cd875df7e2c4d7ba2b9fe2010b0cf5caccc"
            },
            {
            "message": {
                "pubkey": "0xa695ad325dfc7e1191fbc9f186f58eff42a634029731b18380ff89bf42c464a42cb8ca55b200f051f57f1e1893c68759",
                "slot": 33,
                "top": true,
                "transactions": [
                "0x02f86c870c72dd9d5e883e4d0183408f2382520894d2e2adf7177b7a8afddbc12d1634cf23ea1a71020180c001a08556dcfea479b34675db3fe08e29486fe719c2b22f6b0c1741ecbbdce4575cc6a01cd48009ccafd6b9f1290bbe2ceea268f94101d1d322c787018423ebcbc87ab4",
                "0x02f86c870c72dd9d5e883e4d0183408f2382520894d2e2adf7177b7a8afddbc12d1634cf23ea1a71020180c001a08556dcfea479b34675db3fe08e29486fe719c2b22f6b0c1741ecbbdce4575cc6a01cd48009ccafd6b9f1290bbe2ceea268f94101d1d322c787018423ebcbc87ab4"
                ]
            },
            "signature": "0x9241909209aa8f5d7128452d478922f7d2f54040eeaa0e998cd395f102c577c171523073aabe3290caeaed2389e412ae03021bf3ef29a836ad3e8dde7cc799fa95695ae980246b5714dd75f6f06427a8c5e911db4295c6f8975bbe716704d19b"
            }
        ]"#
    }

    fn _get_signed_constraint_conflict_1_json() -> &'static str {
        r#"[
            {
            "message": {
                "pubkey": "0xa695ad325dfc7e1191fbc9f186f58eff42a634029731b18380ff89bf42c464a42cb8ca55b200f051f57f1e1893c68759",
                "slot": 32,
                "top": true,
                "transactions": [
                "0x02f86c870c72dd9d5e883e4d0183408f2382520894d2e2adf7177b7a8afddbc12d1634cf23ea1a71020180c001a08556dcfea479b34675db3fe08e29486fe719c2b22f6b0c1741ecbbdce4575cc6a01cd48009ccafd6b9f1290bbe2ceea268f94101d1d322c787018423ebcbc87ab4",
                "0x02f9017b8501a2140cff8303dec685012a05f2008512a05f2000830249f094843669e5220036eddbaca89d8c8b5b82268a0fc580b901040cc7326300000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000022006292538e66f0000000000000000000000005ba38f2c245618e39f6fa067bf1dec304e73ff3c00000000000000000000000092f0ee29e6e1bf0f7c668317ada78f5774a6cb7f000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000003fac6482aee49bf58515be2d3fb58378a8497cc9000000000000000000000000c6cc140787b02ae479a10e41169607000c0d44f6c080a00cf74c45dbe9ee1fb923118ec5ce9db8f88cd651196ed3f9d4f8f2a65827e611a04a6bc1d49a7e18b7c92e8f3614cae116b1832ceb311c81d54b2c87de1545f68f"
                ]
            },
            "signature": "0x97d249cbd4b2a33fab4e8d0b698a1905815375c6eb905356768bc23c90b5f2972e03580116849e3f250e7c650486549d16f090135a109c9dd21360e1ca52cb941fada8bdf0280fe184ec4adba7d9246a6d9fc516bfbe508ad1c21cfc2e169b52"
            }
        ]"#
    }

    fn _get_signed_constraint_conflict_2_json() -> &'static str {
        r#"[
            {
            "message": {
                "pubkey": "0xa695ad325dfc7e1191fbc9f186f58eff42a634029731b18380ff89bf42c464a42cb8ca55b200f051f57f1e1893c68759",
                "slot": 32,
                "top": true,
                "transactions": [
                "0x02f86c870c72dd9d5e883e4d0183408f2382520894d2e2adf7177b7a8afddbc12d1634cf23ea1a71020180c001a08556dcfea479b34675db3fe08e29486fe719c2b22f6b0c1741ecbbdce4575cc6a01cd48009ccafd6b9f1290bbe2ceea268f94101d1d322c787018423ebcbc87ab4"
                ]
            },
            "signature": "0xb648321b682a445d377c3a11180213f1c21e5a645b2f80fb6ec31a85550d3bb0a0fb1eef806d6403a76ede552f87c8f219e0703d254dc78dca0b4a904794f81c995884dd90648190f4e4c5badc7463314ce57ceb5448767211a3c4af5b860650"
            }
        ]"#
    }

    fn _get_signed_delegation() -> &'static str {
        r#"
        {
        "message": {
            "validator_pubkey": "0xa695ad325dfc7e1191fbc9f186f58eff42a634029731b18380ff89bf42c464a42cb8ca55b200f051f57f1e1893c68759",
            "delegatee_pubkey": "0x8db2e6dd9fe48cb14b2d0d5427b639c6aa0c7bf25cf132f27ad5ae5a2dd2523626d26171d5189869cf83228b29ef3919"
        },
        "signature": "0xb62c235dd275859c1bda06b657f9a6058dc4c7d27322e16c743abbe942caa54cf00cc5d206db1924a30e1cc91e44db6b0091a405926e470063313b1022f8982e32476934de79ace0deb5b9332d5347aa9f4a8b9d0ed2222af0144eefb6aed145"
        }
        "#
    }

    // +++ TESTS +++
    #[tokio::test]
    #[serial]
    async fn test_submit_constraints_conflict() {
        tracing_subscriber::fmt::init();

        // Start the server
        let (tx, http_config, _constraints_api, _builder_api, mut slot_update_receiver) = start_api_server().await;

        let slot_update_sender = slot_update_receiver.recv().await.unwrap();
        send_dummy_slot_update(slot_update_sender.clone(), None, None, None).await;

        let test_constraint: List<SignedConstraints, MAX_CONSTRAINTS_PER_SLOT> = serde_json::from_str(_get_signed_constraint_conflict_1_json()).unwrap();

        // Submit constraints
        let req_url = format!("{}{}", http_config.base_url(), Route::SubmitBuilderConstraints.path());

        // Send JSON encoded request
        let resp = send_request(
            &req_url,
            Encoding::Json,
            serde_json::to_vec(&test_constraint).unwrap(),
        )
        .await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let test_constraint: List<SignedConstraints, MAX_CONSTRAINTS_PER_SLOT> = serde_json::from_str(_get_signed_constraint_conflict_2_json()).unwrap();

        // Send JSON encoded request
        let resp = send_request(
            &req_url,
            Encoding::Json,
            serde_json::to_vec(&test_constraint).unwrap(),
        )
        .await;

        // This will result in a conflict as 2 constraints are submitted with top = true
        assert_eq!(resp.status(), reqwest::StatusCode::CONFLICT);
        info!("Response: {:?}", resp);

        // Send shutdown signal
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_submit_constraints_and_get_constraints_ok() {
        tracing_subscriber::fmt::init();

        // Start the server
        let (tx, http_config, _constraints_api, _builder_api, mut slot_update_receiver) = start_api_server().await;

        let slot_update_sender = slot_update_receiver.recv().await.unwrap();
        send_dummy_slot_update(slot_update_sender.clone(), None, None, None).await;

        let test_constraints: List<SignedConstraints, MAX_CONSTRAINTS_PER_SLOT> = serde_json::from_str(_get_signed_constraints_json()).unwrap();

        // Submit constraints
        let req_url = format!("{}{}", http_config.base_url(), Route::SubmitBuilderConstraints.path());

        // Send SSZ encoded request
        let resp = send_request(
            &req_url,
            Encoding::Ssz,
            ssz::prelude::serialize(&test_constraints).unwrap(),
        )
        .await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // Correct and complete the below
        let slot = 32;

        // Get constraints
        let req_url = format!(
            "{}{}",
            http_config.base_url(),
            Route::GetBuilderConstraints.path()
        );

        let resp = reqwest::Client::new()
            .get(req_url)
            .query(&[("slot", slot)])
            .header("accept", "application/json")
            .send()
            .await
            .unwrap();

        // Ensure the response is OK
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        
        // Print the response body
        let body: Vec<SignedConstraints> = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
        info!("Response body: {:?}", body);
        // TODO: clean this
        let constraint = body.first().unwrap().clone();
        let send = test_constraints.first().unwrap().clone();
        assert_eq!(constraint.signature, send.signature);

        // Send shutdown signal
        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_delegate_submission_rights_ok() {
        tracing_subscriber::fmt::init();

        let (tx, http_config, _api, _, _) = start_api_server().await;

        let test_delegation: SignedDelegation = serde_json::from_str(_get_signed_delegation()).unwrap();

        let req_url = format!("{}{}", http_config.base_url(), Route::DelegateSubmissionRights.path());
        let req_payload = serde_json::to_vec(&test_delegation).unwrap();

        // Send JSON encoded request
        let resp = send_request(&req_url, Encoding::Json, req_payload).await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_get_delegations() {
        tracing_subscriber::fmt::init();

        // Start the server
        let (tx, http_config, _constraints_api, _builder_api, mut slot_update_receiver) = start_api_server().await;

        let slot_update_sender = slot_update_receiver.recv().await.unwrap();
        send_dummy_slot_update(slot_update_sender.clone(), None, None, None).await;

        let test_delegation: SignedDelegation = serde_json::from_str(_get_signed_delegation()).unwrap();

        let req_url = format!("{}{}", http_config.base_url(), Route::DelegateSubmissionRights.path());
        let req_payload = serde_json::to_vec(&test_delegation).unwrap();

        // Send JSON encoded request
        let resp = send_request(&req_url, Encoding::Json, req_payload).await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // Get delegations
        let slot = 33;

        let req_url = format!(
            "{}{}",
            http_config.base_url(),
            Route::GetBuilderDelegations.path()
        );

        let resp = reqwest::Client::new()
            .get(req_url)
            .query(&[("slot", slot)])
            .header("accept", "application/json")
            .send()
            .await
            .unwrap();

        // Ensure the response is OK
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let body: Vec<BlsPublicKey> = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
        assert_eq!(test_delegation.message.delegatee_pubkey, body.first().unwrap().clone());

        let _ = tx.send(());
    }

    #[tokio::test]
    #[serial]
    async fn test_revoke_submission_rights_ok() {
        let (tx, http_config, api, _, _) = start_api_server().await;

        let test_revocation: SignedRevocation = serde_json::from_str(_get_signed_delegation()).unwrap();

        let req_url = format!("{}{}", http_config.base_url(), Route::RevokeSubmissionRights.path());
        let req_payload = serde_json::to_vec(&test_revocation).unwrap();

        // Send JSON encoded request
        let resp = send_request(&req_url, Encoding::Json, req_payload).await;
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let _ = tx.send(());
    }
}