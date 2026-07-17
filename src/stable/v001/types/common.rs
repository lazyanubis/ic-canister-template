use std::collections::HashSet;

use candid::CandidType;
use ic_canister_kit::types::*;
use serde::{Deserialize, Serialize};

#[derive(CandidType, Serialize, Deserialize, Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct HashDigest(pub(super) [u8; 32]);

impl HashDigest {
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
}

#[derive(CandidType, Serialize, Deserialize, Debug, Clone, Default)]
pub struct HashedPath(pub(super) HashSet<String>);

// =========== 查询的对象 ===========

#[derive(CandidType, Serialize, Deserialize, Debug, Clone)]
pub struct QueryFile {
    pub path: String,
    pub size: u64,
    pub headers: Vec<(String, String)>,
    pub created: TimestampNanos,
    pub modified: TimestampNanos,
    pub hash: String,
}
