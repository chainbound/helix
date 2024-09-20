use ethereum_consensus::{primitives::{BlsPublicKey, BlsSignature}, ssz::prelude::*};

pub const MAX_CONSTRAINTS_PER_SLOT: usize = 256;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SignedDelegation {
    pub message: Delegation,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone, SimpleSerialize, serde::Deserialize, serde::Serialize)]
pub struct Delegation {
    pub validator_pubkey: BlsPublicKey,
    pub delegatee_pubkey: BlsPublicKey,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SignedRevocation {
    pub message: Revocation,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone, SimpleSerialize, serde::Deserialize, serde::Serialize)]
pub struct Revocation {
    pub validator_pubkey: BlsPublicKey,
    pub delegatee_pubkey: BlsPublicKey,
}
