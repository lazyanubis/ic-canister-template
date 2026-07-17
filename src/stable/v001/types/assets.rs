use candid::CandidType;
use ic_canister_kit::types::*;
use serde::{Deserialize, Serialize};

use crate::stable::v001::types::init_assets_data;

use super::{HashDigest, stable::SliceOfHashDigest};

// ============================== 文件数据 ==============================

// 单个文件数据
#[derive(CandidType, Serialize, Deserialize, Debug, Clone, Default)]
pub struct AssetData {
    // 堆内存无数据，存放在稳定内存了
}

const MAX_BUCKET_SIZE: u64 = 1024 * 1024 * 2;

#[inline]
fn get_key(hash: &HashDigest, chunk: u32) -> SliceOfHashDigest {
    let mut key = [0; 36];
    key[..4].copy_from_slice(&chunk.to_be_bytes());
    key[4..].copy_from_slice(&hash.0);
    key
}

impl AssetData {
    pub fn from(hash: &HashDigest, data: Vec<u8>) -> Self {
        // 切片
        let size = data.len() as u64;
        let chunks = size / MAX_BUCKET_SIZE;
        let mut index = (0..chunks)
            .map(|i| {
                let key = get_key(hash, i as u32);
                (key, MAX_BUCKET_SIZE * i, MAX_BUCKET_SIZE)
            })
            .collect::<Vec<_>>();
        let remain = size - chunks * MAX_BUCKET_SIZE;
        if 0 < remain {
            let key = get_key(hash, chunks as u32);
            index.push((key, MAX_BUCKET_SIZE * chunks, remain))
        }

        // 插入数据
        let mut assets = init_assets_data();
        for (key, offset, size) in index {
            let offset = offset as usize;
            let size = size as usize;
            let data = data[offset..offset + size].to_vec();
            assets.insert(key, data);
        }

        // 返回空对象
        AssetData {}
    }

    pub fn remove(hash: &HashDigest, data_size: u64) {
        let chunks = data_size.div_ceil(MAX_BUCKET_SIZE);
        let mut assets = init_assets_data();
        for chunk in 0..chunks {
            let chunk = match u32::try_from(chunk) {
                Ok(chunk) => chunk,
                Err(_) => ic_cdk::trap("Asset chunk index exceeds the supported range."),
            };
            assets.remove(&get_key(hash, chunk));
        }
    }

    pub fn slice(&self, hash: &HashDigest, data_size: u64, offset: usize, size: usize) -> std::borrow::Cow<'_, [u8]> {
        let data_size = match usize::try_from(data_size) {
            Ok(data_size) => data_size,
            Err(_) => ic_cdk::trap("Asset size exceeds the platform address range."),
        };
        assert!(offset <= data_size, "Asset offset exceeds file size.");
        let offset_end = match offset.checked_add(size) {
            Some(offset_end) => offset_end,
            None => ic_cdk::trap("Asset slice range overflow."),
        };
        assert!(offset_end <= data_size, "Asset slice exceeds file size.");

        let mut result = vec![0; size];
        let mut cursor = 0;

        let assets = init_assets_data();

        let mut last_chunk = offset as u64 / MAX_BUCKET_SIZE;
        let mut offset = (offset as u64 - last_chunk * MAX_BUCKET_SIZE) as usize;
        let mut size = size;
        while 0 < size {
            let remain = MAX_BUCKET_SIZE as usize - offset; // 本次最多可以取这么多
            let fetch = std::cmp::min(size, remain); // 本次应该取的数据

            let key = get_key(hash, last_chunk as u32);

            let data = assets.get(&key);
            let data = ic_canister_kit::common::trap(data.ok_or("can not be"));
            let data_end = match offset.checked_add(fetch) {
                Some(data_end) => data_end,
                None => ic_cdk::trap("Asset chunk slice range overflow."),
            };
            assert!(data_end <= data.len(), "Asset chunk is shorter than its metadata.");

            result[cursor..cursor + fetch].copy_from_slice(&data[offset..data_end]);

            cursor += fetch; // 修改结果写入位置
            last_chunk += 1; // 修改为下一个块
            offset = (offset + fetch) % MAX_BUCKET_SIZE as usize; // 修改并检查新的块偏移位置
            size -= fetch; // 修改剩余的数据
        }

        std::borrow::Cow::Owned(result)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_reads_and_removes_all_stable_chunks_for_an_asset() {
        let hash = HashDigest([42; 32]);
        let mut data = vec![1; MAX_BUCKET_SIZE as usize];
        data.extend_from_slice(&[2, 3, 4]);
        let asset = AssetData::from(&hash, data.clone());

        assert_eq!(
            asset.slice(&hash, data.len() as u64, MAX_BUCKET_SIZE as usize - 2, 5),
            &data[MAX_BUCKET_SIZE as usize - 2..MAX_BUCKET_SIZE as usize + 3]
        );

        AssetData::remove(&hash, data.len() as u64);

        let assets = init_assets_data();
        assert!(assets.get(&get_key(&hash, 0)).is_none());
        assert!(assets.get(&get_key(&hash, 1)).is_none());
    }
}
