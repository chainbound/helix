use ethereum_consensus::{
    bellatrix::presets::minimal::Transaction, deneb::minimal::MAX_TRANSACTIONS_PER_PAYLOAD,
    phase0::Bytes32, primitives::{BlsPublicKey, BlsSignature}, ssz::prelude::*
};
use alloy_primitives::{B256, keccak256, TxHash};
use tree_hash::Hash256;

// Import the new version of the `ssz-rs` crate for multiproof verification.
use ::ssz_rs as ssz;

use crate::api::constraints_api::MAX_CONSTRAINTS_PER_SLOT;
use crate::eth::SignedBuilderBid;

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

#[derive(Debug, Clone, SimpleSerialize, serde::Serialize, serde::Deserialize)]
pub struct InclusionProofs {
    pub transaction_hashes: List<Bytes32, MAX_CONSTRAINTS_PER_SLOT>,
    pub generalized_indexes: List<u64, MAX_CONSTRAINTS_PER_SLOT>,
    pub merkle_hashes: List<Bytes32, MAX_TRANSACTIONS_PER_PAYLOAD>,
}

impl InclusionProofs {
    /// Returns the total number of leaves in the tree.
    pub fn total_leaves(&self) -> usize {
        self.transaction_hashes.len()
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct BidWithProofs {
    pub bid: SignedBuilderBid,
    pub proofs: Option<InclusionProofs>,
}

pub type HashTreeRoot = tree_hash::Hash256;

// NOTE: This type is redefined here to avoid circular dependencies.
#[derive(Debug, Clone, Serializable, serde::Deserialize, serde::Serialize)]
pub struct SignedConstraints {
    pub message: ConstraintsMessage,
    pub signature: BlsSignature,
}

// NOTE: This type is redefined here to avoid circular dependencies.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Serializable, Merkleized)]
pub struct ConstraintsMessage {
    pub pubkey: BlsPublicKey,
    pub slot: u64,
    pub top: bool,
    pub transactions: List<Transaction, MAX_CONSTRAINTS_PER_SLOT>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ConstraintsWithProofData {
    pub message: ConstraintsMessage,
    /// List of transaction hashes and corresponding hash tree roots. Same order
    /// as the transactions in the `message`.
    pub proof_data: Vec<(TxHash, HashTreeRoot)>,
}

impl TryFrom<ConstraintsMessage> for ConstraintsWithProofData {
    type Error = ProofError;

    fn try_from(value: ConstraintsMessage) -> Result<Self, ProofError> {
        let transactions = value
            .transactions
            .iter()
            .map(|tx| {
                let tx_hash = TxHash::from_slice(keccak256(tx.to_vec()).as_slice());
                let tx_root = Transaction::try_from(tx.to_vec().as_ref())
                    .map_err(|_| ProofError::VerificationFailed)?
                    .hash_tree_root()
                    .map_err(|_| ProofError::VerificationFailed)?;
                let tx_root = Hash256::from_slice(&tx_root.to_vec());

                Ok((tx_hash, tx_root))
            })
            .collect::<Result<Vec<_>, ProofError>>()?;

        Ok(Self { message: value, proof_data: transactions })
    }
}

/// Returns the length of the leaves that need to be proven (i.e. all transactions).
fn total_leaves(constraints: &[ConstraintsWithProofData]) -> usize {
    constraints.iter().map(|c| c.proof_data.len()).sum()
}

/// Verifies the provided multiproofs against the constraints & transactions root.
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

    // NOTE: Get the leaves from the constraints cache by matching the saved hashes.
    // We need the leaves in order to verify the multiproof.
    for hash in proofs.transaction_hashes.iter() {
        let mut found = false;
        for constraint in constraints {
            for (saved_hash, leaf) in &constraint.proof_data {
                if saved_hash.as_slice() == hash.as_slice() {
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

    // Conversions to the correct types (and versions of the same type)
    let merkle_proofs = proofs.merkle_hashes.to_vec().iter().map(|h| B256::from_slice(h.as_ref())).collect::<Vec<_>>();
    let indeces = proofs.generalized_indexes.to_vec().iter().map(|h| *h as usize).collect::<Vec<_>>();

    // Verify the Merkle multiproof against the root
    ssz::multiproofs::verify_merkle_multiproof(
        &leaves,
        &merkle_proofs,
        &indeces,
        root
    )
    .map_err(|_| ProofError::VerificationFailed)?;

    Ok(())
}
