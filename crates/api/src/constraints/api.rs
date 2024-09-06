use axum::Extension;
use ethereum_consensus::deneb::Slot;
use std::{collections::HashMap, sync::Arc};

use super::types::SignedConstraints;

#[derive(Debug, Default)]
pub struct ConstraintsApi {
    constraints: HashMap<Slot, Vec<SignedConstraints>>,
}

impl ConstraintsApi {
    pub fn new() -> Self {
        Self { ..Default::default() }
    }

    pub async fn submit_constraints(Extension(constraints_api): Extension<Arc<ConstraintsApi>>) {
        unimplemented!()
    }

    pub async fn delegate() {
        unimplemented!()
    }

    pub async fn revoke() {
        unimplemented!()
    }
}
