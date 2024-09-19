use ethereum_consensus::{
    bellatrix::presets::minimal::Transaction, primitives::{BlsSignature, BlsPublicKey},
    ssz::prelude::List,
};
use helix_common::api::constraints_api::MAX_CONSTRAINTS_PER_SLOT;

#[derive(Debug, Clone)]
pub struct SignedConstraints {
    pub message: ConstraintsMessage,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone)]
pub struct ConstraintsMessage {
    pub pubkey: BlsPublicKey,
    pub slot: u64,
    pub top: bool,
    pub transactions: List<Transaction, MAX_CONSTRAINTS_PER_SLOT>,
}
