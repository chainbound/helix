// use ethereum_consensus::deneb::minimal::MAX_TRANSACTIONS_PER_PAYLOAD;
// use ethereum_consensus::ssz::prelude::ssz_rs;
// use ethereum_consensus::{
//     phase0::Bytes32,
//     ssz::prelude::{List, SimpleSerialize},
// };

// use crate::api::constraints_api::MAX_CONSTRAINTS_PER_SLOT;

use reth_primitives::{TxHash, B256};
use ethereum_consensus::{
    bellatrix::presets::minimal::Transaction, crypto::PublicKey, primitives::BlsSignature,
    ssz::prelude::List,
};
// #[derive(Debug, Clone, SimpleSerialize, serde::Serialize, serde::Deserialize)]
// pub struct InclusionProofs {
//     transaction_hashes: List<Bytes32, MAX_CONSTRAINTS_PER_SLOT>,
//     generalized_indexes: List<u64, MAX_CONSTRAINTS_PER_SLOT>,
//     merkle_hashes: List<List<Bytes32, MAX_TRANSACTIONS_PER_PAYLOAD>, MAX_CONSTRAINTS_PER_SLOT>,
// }
use crate::api::constraints_api::{InclusionProofs, MAX_CONSTRAINTS_PER_SLOT};
use crate::eth::SignedBuilderBid;

pub struct BidWithProofs {
    pub bid: SignedBuilderBid,
    pub proofs: Option<InclusionProofs>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("Leaves and indices length mismatch")]
    LengthMismatch,
    #[error("Mismatch in provided leaves and leaves to prove")]
    LeavesMismatch,
    #[error("Hash not found in constraints cache: {0:?}")]
    MissingHash(TxHash),
    #[error("Proof verification failed")]
    VerificationFailed,
}

pub type HashTreeRoot = tree_hash::Hash256;

#[derive(Debug)]
pub struct ConstraintsWithProofData {
    pub message: ConstraintsMessage,
    /// List of transaction hashes and corresponding hash tree roots. Same order
    /// as the transactions in the `message`.
    pub proof_data: Vec<(TxHash, HashTreeRoot)>,
}

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

/// Returns the length of the leaves that need to be proven (i.e. all transactions).
fn total_leaves(constraints: &[ConstraintsWithProofData]) -> usize {
    constraints.iter().map(|c| c.proof_data.len()).sum()
}

/// Verifies the provided multiproofs against the constraints & transactions root.
/// TODO: support bundle proof verification a.k.a. relative ordering!
pub fn verify_multiproofs(
    constraints: &[ConstraintsWithProofData],
    proofs: &InclusionProofs,
    root: B256,
) -> Result<(), ProofError> {
    // Check if the length of the leaves and indices match
    if proofs.transaction_hashes.len() != proofs.generalized_indexes.len() {
        return Err(ProofError::LengthMismatch);
    }

    let total_leaves = total_leaves(constraints);

    // Check if the total leaves matches the proofs provided
    if total_leaves != proofs.total_leaves() {
        return Err(ProofError::LeavesMismatch);
    }

    // Get all the leaves from the saved constraints
    let mut leaves = Vec::with_capacity(proofs.total_leaves());

    // NOTE: Get the leaves from the constraints cache by matching the saved hashes. We need the leaves
    // in order to verify the multiproof.
    for hash in proofs.transaction_hashes.iter() {
        let mut found = false;
        for constraint in constraints {
            for (saved_hash, leaf) in &constraint.proof_data {
                if **saved_hash == ****hash {
                    found = true;
                    leaves.push(B256::from(leaf.0));
                    break;
                }
            }
            if found {
                break;
            }
        }

        // If the hash is not found in the constraints cache, return an error
        if !found {
            return Err(ProofError::MissingHash(TxHash::from_slice(hash.as_slice())));
        }
    }

    // Verify the Merkle multiproof against the root
    ssz_rs::multiproofs::verify_merkle_multiproof(
        &leaves,
        &proofs.merkle_hashes,
        &proofs.generalized_indexes,
        root,
    )
    .map_err(|_| ProofError::VerificationFailed)?;

    Ok(())
}
