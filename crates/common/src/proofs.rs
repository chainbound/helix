use ethereum_consensus::{
    deneb::minimal::MAX_TRANSACTIONS_PER_PAYLOAD,
    bellatrix::presets::minimal::Transaction,
    primitives::BlsSignature,
    phase0::Bytes32,
    ssz::prelude::*,
};
use reth_primitives::{TxHash, B256};
use alloy_primitives::B256 as AB256;

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

pub struct BidWithProofs {
    pub bid: SignedBuilderBid,
    pub proofs: Option<InclusionProofs>,
}

pub type HashTreeRoot = tree_hash::Hash256;

#[derive(Debug)]
pub struct ConstraintsWithProofData {
    pub message: ConstraintsMessage,
    /// List of transaction hashes and corresponding hash tree roots. Same order
    /// as the transactions in the `message`.
    pub proof_data: Vec<(TxHash, HashTreeRoot)>,
}

// TODO: Requires Alloy. Trying to not add alloy to prevent cargo lock issues.
// impl TryFrom<ConstraintsMessage> for ConstraintsWithProofData {
//     // type Error = Eip2718Error;

//     fn try_from(value: ConstraintsMessage) -> Result<Self, Self::Error> {
//         let transactions = value
//             .transactions
//             .iter()
//             .map(|tx| {
//                 let tx_hash = *TxEnvelope::decode_2718(&mut tx.as_ref())?.tx_hash();

//                 let tx_root =
//                     tree_hash::TreeHash::tree_hash_root(&Transaction::<
//                         <DenebSpec as EthSpec>::MaxBytesPerTransaction,
//                     >::from(tx.to_vec()));

//                 Ok((tx_hash, tx_root))
//             })
//             .collect::<Result<Vec<_>, Eip2718Error>>()?;

//         Ok(Self { message: value, proof_data: transactions })
//     }
// }


// NOTE: This type is redefined here to avoid circular dependencies.
#[derive(Debug, Clone)]
pub struct SignedConstraints {
    pub message: ConstraintsMessage,
    pub signature: BlsSignature,
}

// NOTE: This type is redefined here to avoid circular dependencies.
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
                    leaves.push(AB256::from(leaf.0));
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
    let merkle_proofs = proofs.merkle_hashes.to_vec().iter().map(|h| AB256::from_slice(h.as_ref())).collect::<Vec<_>>();
    let indeces = proofs.generalized_indexes.to_vec().iter().map(|h| *h as usize).collect::<Vec<_>>();

    // Verify the Merkle multiproof against the root
    ssz::multiproofs::verify_merkle_multiproof(
        &leaves,
        &merkle_proofs,
        &indeces,
        AB256::from_slice(root.as_slice()),
    )
    .map_err(|_| ProofError::VerificationFailed)?;

    Ok(())
}
