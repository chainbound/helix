use ethereum_consensus::deneb::minimal::MAX_TRANSACTIONS_PER_PAYLOAD;
use ethereum_consensus::ssz::prelude::ssz_rs;
use ethereum_consensus::{
    phase0::Bytes32,
    ssz::prelude::{List, SimpleSerialize},
};

pub const MAX_CONSTRAINTS_PER_SLOT: usize = 256;

#[derive(Debug, Clone, SimpleSerialize, serde::Serialize, serde::Deserialize)]
pub struct InclusionProofs {
    pub transaction_hashes: List<Bytes32, MAX_CONSTRAINTS_PER_SLOT>,
    pub generalized_indexes: List<u64, MAX_CONSTRAINTS_PER_SLOT>,
    pub merkle_hashes: List<List<Bytes32, MAX_TRANSACTIONS_PER_PAYLOAD>, MAX_CONSTRAINTS_PER_SLOT>,
}

impl InclusionProofs {
    /// Returns the total number of leaves in the tree.
    pub fn total_leaves(&self) -> usize {
        self.transaction_hashes.len()
    }
}