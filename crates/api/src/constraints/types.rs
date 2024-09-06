use ethereum_consensus::{
    bellatrix::presets::minimal::Transaction, crypto::PublicKey, primitives::BlsSignature,
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
    pub validator_index: u64,
    pub slot: u64,
    pub top: bool,
    pub transactions: List<Transaction, MAX_CONSTRAINTS_PER_SLOT>,
}

#[derive(Debug, Clone)]
pub struct SignedDelegation {
    pub message: Delegation,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone)]
pub struct Delegation {
    pub validator_index: u64,
    pub pubkey: PublicKey,
}

#[derive(Debug, Clone)]
pub struct SignedRevocation {
    pub message: Revocation,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone)]
pub struct Revocation {
    pub validator_index: u64,
    pub pubkey: PublicKey,
}
