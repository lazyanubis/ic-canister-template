pub use ic_canister_kit::types::*;
use serde::{Deserialize, Serialize};

#[allow(unused)]
pub use super::super::{Business, MutableBusiness, ParsePermission, ScheduleTask};

#[allow(unused)]
pub use super::super::business::*;
#[allow(unused)]
pub use super::business::*;
#[allow(unused)]
pub use super::permission::*;
#[allow(unused)]
pub use super::schedule::schedule_task;

mod _init;
pub use _init::*;
mod _upgrade;
pub use _upgrade::*;
mod _topic;
pub use _topic::*;
mod _canister_kit;
pub use _canister_kit::*;

// 业务类型
mod common;
pub use common::*;
mod assets;
pub use assets::*;
mod upload;
pub use upload::*;

pub const MAX_ASSET_FILE_SIZE: u64 = 256 * 1024 * 1024;
pub const MAX_ASSET_PATH_BYTES: usize = 1024;
pub const MAX_ASSET_HEADER_COUNT: usize = 64;
pub const MAX_ASSET_HEADER_BYTES: usize = 64 * 1024;
pub const MAX_ASSET_UPLOAD_BATCH_SIZE: usize = 64;
pub const MAX_ASSET_DELETE_BATCH_SIZE: usize = 1000;
pub const MAX_ASSET_UPLOAD_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ASSET_DELETE_PATH_BYTES: usize = 64 * 1024;
const MAX_ACTIVE_UPLOADS: usize = 16;
const MAX_ASSET_STATE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_DIRECT_DOWNLOAD_SIZE: u64 = ic_canister_kit::http::MAX_RESPONSE_LENGTH as u64;

// 能序列化的和不能序列化的放在一起
// 其中不能序列化的采用如下注解
// #[serde(skip)] 默认初始化方式
// #[serde(skip, default="init_xxx_data")] 指定初始化方式
// ! 如果使用 ic-stable-structures 提供的稳定内存，不能变更 memory_id 的使用类型，否则会出现各个版本不兼容，数据会被清空
#[derive(Serialize, Deserialize)]
pub struct InnerState {
    pub canister_kit: CanisterKit, // 框架需要的数据 // ? 堆内存 序列化

    // 业务数据
    pub hashed: bool, // 是否相信上传的 hash 值，true -> 直接采用接口传递的 hash 值， false -> 数据上传完成后，需要罐子再 hash 一次 // ? 堆内存 序列化

    pub assets: HashMap<HashDigest, AssetData>, // key 是 hash // ? 堆内存 序列化
    pub files: HashMap<String, AssetFile>,      // key 是 path // ? 堆内存 序列化
    hashes: HashMap<HashDigest, HashedPath>, // key 是 hash, value 是 path, 没有 path 的数据是没有保存意义的 // ? 堆内存 序列化

    uploading: HashMap<String, UploadingFile>, // key 是 path // ? 堆内存 序列化
}

impl Default for InnerState {
    fn default() -> Self {
        ic_cdk::println!("v001.InnerState::default()");
        Self {
            canister_kit: Default::default(),

            // 业务数据
            hashed: Default::default(),

            assets: Default::default(),
            files: Default::default(),
            hashes: Default::default(),

            uploading: Default::default(),
        }
    }
}

impl InnerState {
    pub fn do_init(&mut self, _arg: InitArg) {
        // maybe do something
    }

    pub fn do_upgrade(&mut self, _arg: UpgradeArg) {
        // maybe do something
    }

    fn hash(file: &UploadingFile) -> HashDigest {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&file.data[0..(file.size as usize)]);
        let digest: [u8; 32] = hasher.finalize().into();
        HashDigest(digest)
    }
    fn clean_hash_path(&mut self, hash: HashDigest, path: &str) {
        if let Some(HashedPath(path_set)) = self.hashes.get_mut(&hash) {
            path_set.remove(path);
        }
        if !self.files.values().any(|file| file.hash == hash) {
            self.hashes.remove(&hash);
            self.assets.remove(&hash);
        }
    }
    fn put_file(&mut self, path: String, headers: Vec<(String, String)>, hash: HashDigest, size: u64) {
        // 3. 插入 files: path -> hash
        let now = ic_canister_kit::times::now();
        let old_hash = self.files.get(&path).map(|file| file.hash);
        if let Some(exist) = self.files.get_mut(&path) {
            exist.modified = now;
            exist.headers = headers;
            exist.hash = hash;
            exist.size = size;
        } else {
            self.files.insert(
                path.clone(),
                AssetFile {
                    path: path.clone(),
                    created: now,
                    modified: now,
                    headers,
                    hash,
                    size,
                },
            );
        }

        if let Some(old_hash) = old_hash
            && old_hash != hash
        {
            self.clean_hash_path(old_hash, &path);
        }

        // 4. 插入 hashes: hash -> [path]
        self.hashes.entry(hash).or_default();
        if let Some(hash_path) = self.hashes.get_mut(&hash) {
            hash_path.0.insert(path);
        }
    }
    fn put_assets(&mut self, file: UploadingFile) {
        // 0. 先清空同路径的文件
        self.clean_file(&file.path);
        // 1. 计算 hash
        let hash = if self.hashed {
            file.hash // hashed true 直接使用
        } else {
            Self::hash(&file) // hashed false 要计算一次
        };
        // 2. 插入 assets: hash -> data
        self.assets
            .entry(hash)
            .or_insert_with(|| AssetData::from(&hash, file.data));

        self.put_file(file.path, file.headers, hash, file.size); // 存完毕 assets 数据了，然后要对文件建立代理索引
    }
    pub fn clean_file(&mut self, path: &str) {
        // 1. 删除文件
        let file = match self.files.remove(path) {
            Some(file) => file,
            None => return,
        };
        // 2. 清除 hashes 和无引用的 assets
        self.clean_hash_path(file.hash, &file.path);
    }
    pub fn files(&self) -> Vec<QueryFile> {
        self.files
            .iter()
            .map(|(path, file)| QueryFile {
                path: path.to_string(),
                size: file.size,
                headers: file.headers.clone(),
                created: file.created,
                modified: file.modified,
                hash: file.hash.hex(),
            })
            .collect()
    }
    pub fn download(&self, path: String) -> Vec<u8> {
        use ic_canister_kit::common::trap;
        let file = trap(self.files.get(&path).ok_or("File not found"));
        assert!(
            file.size <= MAX_DIRECT_DOWNLOAD_SIZE,
            "File is too large for a direct query; use business_download_by or HTTP streaming."
        );
        let asset = trap(self.assets.get(&file.hash).ok_or("File not found"));
        let size = trap(usize::try_from(file.size).map_err(|_| "File size exceeds the platform address range."));
        asset.slice(&file.hash, file.size, 0, size).to_vec()
    }
    pub fn download_by(&self, path: String, offset: u64, size: u64) -> Vec<u8> {
        use ic_canister_kit::common::trap;
        assert!(
            size <= MAX_DIRECT_DOWNLOAD_SIZE,
            "Download size exceeds the direct query response limit."
        );
        let file = trap(self.files.get(&path).ok_or("File not found"));
        let asset = trap(self.assets.get(&file.hash).ok_or("File not found"));
        let offset = trap(usize::try_from(offset).map_err(|_| "Asset offset exceeds the platform address range."));
        let size = trap(usize::try_from(size).map_err(|_| "Asset size exceeds the platform address range."));
        asset.slice(&file.hash, file.size, offset, size).to_vec()
    }

    fn chunks(arg: &UploadingArg) -> u32 {
        let mut chunks = arg.size / arg.chunk_size as u64; // 完整的块数
        if chunks * (arg.chunk_size as u64) < arg.size {
            chunks += 1;
        }
        chunks as u32
    }
    fn offset(arg: &UploadingArg) -> (usize, usize) {
        let chunks = Self::chunks(arg);
        let offset = arg.chunk_size as u64 * arg.index as u64;
        let mut offset_end = offset + arg.chunk_size as u64;
        if arg.index == chunks - 1 {
            offset_end = arg.size;
        }
        (offset as usize, offset_end as usize)
    }
    fn check_path_and_headers(arg: &UploadingArg) {
        // 1. 检查 路径名
        assert!(!arg.path.is_empty(), "must has path");
        assert!(arg.path.starts_with('/'), "path must start with /");
        assert!(arg.path.len() <= MAX_ASSET_PATH_BYTES, "path is too large");
        assert!(
            !arg.path
                .bytes()
                .any(|byte| byte.is_ascii_control() || matches!(byte, b'?' | b'#')),
            "path contains unsupported characters"
        );
        // 2. 检查 headers
        assert!(arg.headers.len() <= MAX_ASSET_HEADER_COUNT, "too many headers");
        let mut header_bytes = 0_usize;
        for (name, value) in &arg.headers {
            assert!(!name.is_empty(), "header name can not be empty");
            assert!(name.len() <= 64, "header name is too large");
            assert!(value.len() <= 1024 * 8, "header value is too large");
            assert!(name.bytes().all(is_http_token_byte), "invalid header name");
            assert!(
                !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n')),
                "invalid header value"
            );
            assert!(!is_reserved_response_header(name), "header is managed by the canister");
            header_bytes = match header_bytes.checked_add(name.len() + value.len()) {
                Some(header_bytes) => header_bytes,
                None => ic_cdk::trap("headers are too large"),
            };
        }
        assert!(header_bytes <= MAX_ASSET_HEADER_BYTES, "headers are too large");
    }
    fn check_size_and_data(arg: &UploadingArg) {
        // 3. 检查 size
        assert!(0 < arg.size, "size can not be 0");
        assert!(
            arg.size <= MAX_ASSET_FILE_SIZE,
            "file size exceeds the configured limit"
        );
        // 4. 检查 chunk_size
        assert!(0 < arg.chunk_size, "chunk size can not be 0");
        assert!(
            arg.chunk_size as usize <= MAX_ASSET_UPLOAD_REQUEST_BYTES,
            "chunk size exceeds the configured limit"
        );
        // 5. 检查 index
        let chunks = Self::chunks(arg);
        assert!(arg.index < chunks, "wrong index");
        // 6. 检查 data
        if arg.index < chunks - 1 || arg.size == arg.chunk_size as u64 * chunks as u64 {
            // 是前面完整的 或者 整好整除
            assert!(arg.chunk.len() as u32 == arg.chunk_size, "wrong chunk length");
        } else {
            // 是剩下的
            assert!(
                arg.chunk.len() as u64 == arg.size % (arg.chunk_size as u64),
                "wrong chunk length"
            );
        }
    }
    fn assure_upload_capacity(&self, arg: &UploadingArg) {
        let replacing_size = self.uploading.get(&arg.path).map(|file| file.size).unwrap_or_default();
        if replacing_size == 0 {
            assert!(self.uploading.len() < MAX_ACTIVE_UPLOADS, "too many active uploads");
        }
        let asset_bytes = self.assets.values().map(|asset| asset.len() as u64).sum::<u64>();
        let uploading_bytes = self.uploading.values().map(|file| file.size).sum::<u64>();
        let projected = asset_bytes
            .checked_add(uploading_bytes.saturating_sub(replacing_size))
            .and_then(|bytes| bytes.checked_add(arg.size));
        assert!(
            projected.is_some_and(|bytes| bytes <= MAX_ASSET_STATE_BYTES),
            "asset and upload data exceed the configured state limit"
        );
    }
    fn assure_uploading(&mut self, arg: &UploadingArg) {
        let chunks = Self::chunks(arg);
        let matches = self.uploading.get(&arg.path).is_some_and(|exist| {
            exist.path == arg.path
                && exist.hash == arg.hash
                && exist.size == arg.size
                && exist.data.len() == arg.size as usize
                && exist.chunk_size == arg.chunk_size
                && exist.chunks == chunks
                && exist.chunked.len() == chunks as usize
        });
        if matches {
            return;
        }

        self.assure_upload_capacity(arg);
        self.uploading.insert(
            arg.path.clone(),
            UploadingFile {
                path: arg.path.clone(),
                headers: arg.headers.clone(),
                hash: arg.hash,
                data: vec![0; arg.size as usize],
                size: arg.size,
                chunk_size: arg.chunk_size,
                chunks,
                chunked: vec![false; chunks as usize],
            },
        );
    }
    pub fn put_uploading(&mut self, arg: UploadingArg) {
        // 1. 检查参数是否有效
        Self::check_path_and_headers(&arg);
        Self::check_size_and_data(&arg);

        // 2. 如果 hashed true 并且已经存在改 hash 值文件了，直接保存即可
        let existing_size = if self.hashed && self.assets.contains_key(&arg.hash) {
            self.files
                .values()
                .find(|file| file.hash == arg.hash)
                .map(|file| file.size)
        } else {
            None
        };
        if let Some(existing_size) = existing_size {
            assert_eq!(
                arg.size, existing_size,
                "uploaded size does not match the existing hash"
            );
            self.clean_uploading(&arg.path);
            self.put_file(arg.path, arg.headers, arg.hash, existing_size);
            return;
        }

        // 3. 确保有缓存空间
        self.assure_uploading(&arg); // 确保该文件已经存在缓存数据了

        // 4. 找的对应的缓存文件
        let mut done = false;
        if let Some(file) = self.uploading.get_mut(&arg.path) {
            // 5. 复制有效的信息
            let (offset, offset_end) = Self::offset(&arg);
            file.headers = arg.headers;
            file.data[offset..offset_end].copy_from_slice(&arg.chunk); // 复制内容
            file.chunked[arg.index as usize] = true;

            // 4. 是否已经完整
            done = file.chunked.iter().all(|c| *c);
        }
        if done && let Some(file) = self.uploading.remove(&arg.path) {
            // 处理这个已经完成的数据
            self.put_assets(file);
        }
    }
    pub fn clean_uploading(&mut self, path: &str) {
        self.uploading.remove(path);
    }
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

fn is_reserved_response_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept-ranges" | "content-disposition" | "content-length" | "content-range" | "etag" | "transfer-encoding"
    )
}

pub fn check_upload_batch(args: &[UploadingArg]) {
    assert!(
        args.len() <= MAX_ASSET_UPLOAD_BATCH_SIZE,
        "too many upload chunks in one request"
    );
    let chunk_bytes = args
        .iter()
        .try_fold(0_usize, |bytes, arg| bytes.checked_add(arg.chunk.len()));
    assert!(
        chunk_bytes.is_some_and(|bytes| bytes <= MAX_ASSET_UPLOAD_REQUEST_BYTES),
        "upload chunk data exceed the request limit"
    );
}

pub fn check_delete_batch(names: &[String]) {
    assert!(
        names.len() <= MAX_ASSET_DELETE_BATCH_SIZE,
        "too many paths in one delete request"
    );
    assert!(
        names
            .iter()
            .all(|name| !name.is_empty() && name.len() <= MAX_ASSET_PATH_BYTES),
        "invalid delete path"
    );
    let path_bytes = names
        .iter()
        .try_fold(0_usize, |bytes, name| bytes.checked_add(name.len()));
    assert!(
        path_bytes.is_some_and(|bytes| bytes <= MAX_ASSET_DELETE_PATH_BYTES),
        "delete paths exceed the request limit"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uploading(path: &str, hash: HashDigest, size: u64, chunk_size: u32) -> UploadingFile {
        let chunks = size.div_ceil(chunk_size as u64) as u32;
        UploadingFile {
            path: path.to_string(),
            headers: vec![],
            hash,
            data: vec![0; size as usize],
            size,
            chunk_size,
            chunks,
            chunked: vec![false; chunks as usize],
        }
    }

    #[test]
    fn deleting_a_path_cleans_upload_and_orphan_asset() {
        let mut state = InnerState::default();
        let path = "/asset.bin".to_string();
        let hash = HashDigest([1; 32]);
        state.assets.insert(hash, AssetData::from(&hash, vec![1, 2, 3]));
        state.files.insert(
            path.clone(),
            AssetFile {
                path: path.clone(),
                created: 0_i128.into(),
                modified: 0_i128.into(),
                headers: vec![],
                hash,
                size: 3,
            },
        );
        state
            .hashes
            .insert(hash, HashedPath([path.clone()].into_iter().collect()));
        state.uploading.insert(path.clone(), uploading(&path, hash, 4, 2));

        state.clean_uploading(&path);
        state.clean_file(&path);

        assert!(!state.uploading.contains_key(&path));
        assert!(!state.files.contains_key(&path));
        assert!(!state.hashes.contains_key(&hash));
        assert!(!state.assets.contains_key(&hash));
    }

    #[test]
    fn changed_upload_parameters_replace_the_old_buffer() {
        let mut state = InnerState::default();
        let path = "/asset.bin".to_string();
        state
            .uploading
            .insert(path.clone(), uploading(&path, HashDigest([1; 32]), 4, 2));
        let arg = UploadingArg {
            path: path.clone(),
            headers: vec![],
            hash: HashDigest([2; 32]),
            size: 6,
            chunk_size: 3,
            index: 0,
            chunk: vec![1, 2, 3],
        };

        state.assure_uploading(&arg);

        let file = state.uploading.get(&path);
        assert!(file.is_some_and(|file| {
            file.hash == arg.hash && file.size == 6 && file.data.len() == 6 && file.chunk_size == 3
        }));
    }

    #[test]
    #[should_panic(expected = "file size exceeds the configured limit")]
    fn rejects_files_above_the_configured_limit() {
        InnerState::check_size_and_data(&UploadingArg {
            path: "/too-large.bin".to_string(),
            headers: vec![],
            hash: HashDigest([0; 32]),
            size: MAX_ASSET_FILE_SIZE + 1,
            chunk_size: 1,
            index: 0,
            chunk: vec![0],
        });
    }
}
