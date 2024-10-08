use ethereum_consensus::{
    bellatrix::presets::minimal::Transaction,
    deneb::minimal::MAX_TRANSACTIONS_PER_PAYLOAD,
    phase0::Bytes32,
    primitives::{BlsPublicKey, BlsSignature},
    ssz::prelude::*,
};
use reth_primitives::{keccak256, PooledTransactionsElement, TxHash, B256};
use sha2::{Digest, Sha256};
use tree_hash::Hash256;

// Import the new version of the `ssz-rs` crate for multiproof verification.
use ::ssz_rs as ssz;

use crate::api::constraints_api::{SignableBLS, MAX_CONSTRAINTS_PER_SLOT};

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

pub type HashTreeRoot = tree_hash::Hash256;

#[derive(Debug, Clone, Serializable, serde::Deserialize, serde::Serialize)]
pub struct SignedConstraints {
    pub message: ConstraintsMessage,
    pub signature: BlsSignature,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Serializable, Merkleized)]
pub struct ConstraintsMessage {
    pub pubkey: BlsPublicKey,
    pub slot: u64,
    pub top: bool,
    pub transactions: List<Transaction, MAX_CONSTRAINTS_PER_SLOT>,
}

impl SignableBLS for ConstraintsMessage {
    fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.pubkey.to_vec());
        hasher.update(&self.slot.to_le_bytes());
        hasher.update((self.top as u8).to_le_bytes());
        for tx in self.transactions.iter() {
            // Convert the opaque bytes to a EIP-2718 envelope and obtain the tx hash.
            // this is needed to handle type 3 transactions.
            let tx = PooledTransactionsElement::decode_enveloped(tx.to_vec().into()).unwrap();
            hasher.update(&keccak256(tx.hash()).as_slice());
        }

        hasher.finalize().into()
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ConstraintsWithProofData {
    pub message: ConstraintsMessage,
    /// List of transaction hashes and corresponding hash tree roots. Same order
    /// as the transactions in the `message`.
    pub proof_data: Vec<(TxHash, HashTreeRoot)>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SignedConstraintsWithProofData {
    pub signed_constraints: SignedConstraints,
    pub proof_data: Vec<(TxHash, HashTreeRoot)>,
}

impl TryFrom<SignedConstraints> for SignedConstraintsWithProofData {
    type Error = ProofError;

    fn try_from(value: SignedConstraints) -> Result<Self, ProofError> {
        let transactions = value
            .message
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

        Ok(Self { signed_constraints: value, proof_data: transactions })
    }
}

/// Returns the length of the leaves that need to be proven (i.e.  transactions).
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
    let leaves = leaves.into_iter().map(|h| h.as_slice().try_into().unwrap()).collect::<Vec<_>>();
    let merkle_proofs = proofs
        .merkle_hashes
        .to_vec()
        .iter()
        .map(|h| h.as_slice().try_into().unwrap())
        .collect::<Vec<_>>();
    let indexes =
        proofs.generalized_indexes.to_vec().iter().map(|h| *h as usize).collect::<Vec<_>>();
    let root = root.as_slice().try_into().expect("Invalid root length");

    // Verify the Merkle multiproof against the root
    ssz::multiproofs::verify_merkle_multiproof(
        leaves.as_slice(),
        merkle_proofs.as_ref(),
        indexes.as_slice(),
        root,
    )
    .map_err(|_| ProofError::VerificationFailed)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use ethereum_consensus::crypto::verify_signature;
    use helix_utils::signing::{COMMIT_BOOST_DOMAIN, GENESIS_VALIDATORS_ROOT};
    use reth_primitives::{bytes, hex::FromHex};
    use tree_hash::TreeHash;
    use tree_hash_derive::TreeHash;

    use super::*;

    #[test]
    fn test_verify_constraint_signature() {
        let constraint = get_signed_constraint();

        // the expected digest is calculated manually with a separate implementation
        let expected_digest =
            B256::from_hex("0x803a56986382dbf935e68ceefe18af43b110cc598d662ca007ae5bfbd6c83be2")
                .unwrap();
        assert_eq!(constraint.message.digest(), expected_digest);

        let signing_root = calculate_devnet_signing_root(constraint.message.digest());

        // the expected root is calculated manually with a separate implementation
        let expected_root =
            B256::from_hex("0x7fc9904f64eb1e1b9a3f6ae61107d622a05a9c391dbc379ce6d76c0c34d9bd39")
                .unwrap();
        assert_eq!(signing_root, expected_root);

        // verify the signature
        // THIS IS THE WRONG PUBKEY: THE SIDECAR USES ITS CONSTRAINTS PRIVATE KEY TO SIGN THE
        // MESSAGE CONTAINING THE VALIDATOR PUBKEY OF THE VALIDATOR THAT NEEDS TO INCLUDE
        // THESE CONSTRAINTS BUT THAT IS NOT THE SAME AS THE SIGNER OF THE CONSTRAINTS
        assert!(verify_signature(&constraint.message.pubkey, &signing_root, &constraint.signature)
            .is_ok());
    }

    /// this constraint is taken manually from the devnet and can be used as a source of truth in
    /// tests
    fn get_signed_constraint() -> SignedConstraints {
        let pubkey = BlsPublicKey::try_from(bytes!("af89ab00a0eab1131645292a9cfba583a69a1e3ac58b210e262494853e67385aeb50d4af428bdd577b9399daa96d8b20").to_vec().as_slice()).unwrap();
        let signature = BlsSignature::try_from(bytes!("860285c0b16eed39e07f062deb7d44a50ebce3f43ae28129c854b4087e7f63c59456b50427a1e97ae02faed0f5174bea15c1d889a01258434d442f60da077f36dc8cce705b3d8b9ed89a693b1f2e962fea890bb3218ff5c789fcf90f355d6a8e").to_vec().as_slice()).unwrap();
        let tx = Transaction::try_from(bytes!("f8678085019dc6838082520894deaddeaddeaddeaddeaddeaddeaddeaddeaddead38808360306ca06664c078fa60bd3ece050903dd295949908dd9686ec8871fa558f868e031cd39a00ed4f0b122b32b73f19230fabe6a726e2d07f84eda5beaa42a1ae1271bdee39f").to_vec().as_slice()).unwrap();

        SignedConstraints {
            message: ConstraintsMessage {
                pubkey,
                slot: 165,
                top: false,
                transactions: List::try_from(vec![tx]).unwrap(),
            },
            signature,
        }
    }

    /// this helper is used to generate a message signing root with:
    /// - the commit boost domain
    /// - the kurtosis devnet fork version
    /// - the nil genesis validators root
    fn calculate_devnet_signing_root(root: [u8; 32]) -> [u8; 32] {
        #[derive(Debug, TreeHash)]
        struct ForkData {
            fork_version: [u8; 4],
            genesis_validators_root: [u8; 32],
        }

        let mut domain = [0u8; 32];

        // commit boost domain.
        domain[..4].copy_from_slice(&COMMIT_BOOST_DOMAIN);

        // kurtosis fork version.
        let fork_version = [0x10, 0x00, 0x00, 0x38];

        // nil genesis validators root
        let fd = ForkData { fork_version, genesis_validators_root: GENESIS_VALIDATORS_ROOT };
        let fork_data_root = fd.tree_hash_root().0;

        domain[4..].copy_from_slice(&fork_data_root[..28]);

        #[derive(Default, Debug, TreeHash)]
        struct SigningData {
            object_root: [u8; 32],
            signing_domain: [u8; 32],
        }

        let signing_data = SigningData { object_root: root, signing_domain: domain };
        signing_data.tree_hash_root().0
    }
}
