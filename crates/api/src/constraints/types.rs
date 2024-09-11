use ethereum_consensus::{
    bellatrix::presets::minimal::Transaction, crypto::PublicKey, primitives::BlsSignature,
    ssz::prelude::List,
};
use helix_common::api::constraints_api::MAX_CONSTRAINTS_PER_SLOT;
use reth_primitives::Bytes;

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
    pub transactions: Vec<Bytes>,
}

#[derive(Debug, Clone)]
pub struct ConstraintsWithProofData {
    pub message: ConstraintsMessage,
    /// List of transaction hashes and corresponding hash tree roots. Same order
    /// as the transactions in the `message`.
    pub proof_data: Vec<(TxHash, HashTreeRoot)>,
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
