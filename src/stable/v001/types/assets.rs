use candid::CandidType;
use ic_canister_kit::types::*;
use serde::{Deserialize, Serialize};

use super::HashDigest;

// ============================== 文件数据 ==============================

// 单个文件数据
#[derive(CandidType, Serialize, Deserialize, Debug, Clone)]
pub struct AssetData {
    data: Vec<u8>, // 实际数据
}

impl AssetData {
    pub fn from(_hash: &HashDigest, data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn slice(&self, _hash: &HashDigest, data_size: u64, offset: usize, size: usize) -> std::borrow::Cow<'_, [u8]> {
        let data_size = match usize::try_from(data_size) {
            Ok(data_size) => data_size,
            Err(_) => ic_cdk::trap("Asset size exceeds the platform address range."),
        };
        assert_eq!(
            self.data.len(),
            data_size,
            "Asset metadata size does not match its data."
        );
        assert!(offset <= data_size, "Asset offset exceeds file size.");
        let offset_end = match offset.checked_add(size) {
            Some(offset_end) => offset_end,
            None => ic_cdk::trap("Asset slice range overflow."),
        };
        assert!(offset_end <= data_size, "Asset slice exceeds file size.");
        std::borrow::Cow::Borrowed(&self.data[offset..offset_end])
    }
}

// 对外的路径数据 指向文件数据
#[derive(CandidType, Serialize, Deserialize, Debug, Clone)]
pub struct AssetFile {
    pub path: String,
    pub created: TimestampNanos,
    pub modified: TimestampNanos,
    pub headers: Vec<(String, String)>,
    pub hash: HashDigest,
    pub size: u64,
}
