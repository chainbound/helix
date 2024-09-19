use ethereum_consensus::primitives::{BlsSignature, BlsPublicKey};
use serde::{Deserialize, Serialize};

pub const MAX_CONSTRAINTS_PER_SLOT: usize = 256;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignedDelegation {
    pub message: Delegation,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Delegation {
    pub validator_pubkey: BlsPublicKey,
    pub delegatee_pubkey: BlsPublicKey,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignedRevocation {
    pub message: Revocation,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Revocation {
    pub validator_pubkey: BlsPublicKey,
    pub delegatee_pubkey: BlsPublicKey,
}
