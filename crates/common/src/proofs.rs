use ethereum_consensus::deneb::minimal::MAX_TRANSACTIONS_PER_PAYLOAD;
use ethereum_consensus::ssz::prelude::ssz_rs;
use ethereum_consensus::{
    phase0::Bytes32,
    ssz::prelude::{List, SimpleSerialize},
};

use crate::api::constraints_api::MAX_CONSTRAINTS_PER_SLOT;

#[derive(Debug, Clone, SimpleSerialize, serde::Serialize, serde::Deserialize)]
pub struct InclusionProofs {
    transaction_hashes: List<Bytes32, MAX_CONSTRAINTS_PER_SLOT>,
    generalized_indexes: List<u64, MAX_CONSTRAINTS_PER_SLOT>,
    merkle_hashes: List<List<Bytes32, MAX_TRANSACTIONS_PER_PAYLOAD>, MAX_CONSTRAINTS_PER_SLOT>,
}
