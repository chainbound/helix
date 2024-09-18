#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConstraintSubmissionTrace {
    pub receive: u64,
    pub decode: u64,
    pub cache: u64,
    pub auctioneer: u64,
    pub request_finish: u64,
}