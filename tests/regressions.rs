//! Resource lifecycle and HTTP protocol regressions.

use candid::{Decode, Encode, encode_one};
use pocket_ic::PocketIc;

mod service;
mod util;

use service::*;

const INIT_CYCLES: u128 = 2 * 10_u128.pow(12);
const WASM_MODULE_NEXT: &[u8] = include_bytes!("../sources/source_opt.wasm.gz");
const STREAMING_TEST_FILE_SIZE: usize = 3 * 1024 * 1024;
const UPLOAD_TEST_CHUNK_SIZE: usize = 1024 * 1024;

fn request(
    pic: &PocketIc,
    canister_id: candid::Principal,
    sender: candid::Principal,
    request: CustomHttpRequest,
) -> CustomHttpResponse {
    let bytes = pic
        .query_call(canister_id, sender, "http_request", Encode!(&request).unwrap())
        .unwrap();
    Decode!(&bytes, CustomHttpResponse).unwrap()
}

fn upload_arg(path: &str, hash: Vec<u8>, chunk: Vec<u8>, size: u64, index: u32, chunk_size: u32) -> UploadingArg {
    UploadingArg {
        hash: hash.into(),
        chunk: chunk.into(),
        path: path.to_string(),
        size,
        headers: vec![],
        index,
        chunk_size,
    }
}

#[ignore]
#[test]
fn test_asset_and_http_regressions() {
    let pic = PocketIc::new();
    let (controller, ..) = util::get_identity();
    let canister_id = pic.create_canister_with_settings(Some(controller), None);
    pic.add_cycles(canister_id, INIT_CYCLES);
    pic.install_canister(
        canister_id,
        WASM_MODULE_NEXT.to_vec(),
        encode_one(None::<()>).unwrap(),
        Some(controller),
    );
    let service = PocketedCanisterId::new(canister_id, &pic).sender(controller);

    // 删除必须取消未完成上传；删除后只补最后一块不能重新发布文件。
    service
        .business_upload(vec![upload_arg("/partial.bin", vec![0; 32], vec![1, 2], 4, 0, 2)])
        .unwrap();
    service.business_delete(vec!["/partial.bin".to_string()]).unwrap();
    service
        .business_upload(vec![upload_arg("/partial.bin", vec![0; 32], vec![3, 4], 4, 1, 2)])
        .unwrap();
    assert!(service.business_download("/partial.bin".to_string()).is_err());
    service
        .business_upload(vec![upload_arg("/partial.bin", vec![0; 32], vec![1, 2], 4, 0, 2)])
        .unwrap();
    assert_eq!(
        service.business_download("/partial.bin".to_string()).unwrap(),
        vec![1, 2, 3, 4]
    );

    // 同一路径上传参数改变时，必须用新布局替换旧缓存。
    service
        .business_upload(vec![upload_arg("/restart.bin", vec![1; 32], vec![1, 2], 4, 0, 2)])
        .unwrap();
    service
        .business_upload(vec![upload_arg("/restart.bin", vec![2; 32], vec![5, 6, 7], 6, 0, 3)])
        .unwrap();
    service
        .business_upload(vec![upload_arg("/restart.bin", vec![2; 32], vec![8, 9, 10], 6, 1, 3)])
        .unwrap();
    assert_eq!(
        service.business_download("/restart.bin".to_string()).unwrap(),
        vec![5, 6, 7, 8, 9, 10]
    );

    // hash 快速复用必须更新 size，并清除旧 hash 的资源和反向索引。
    service
        .business_upload(vec![upload_arg("/old.bin", vec![0; 32], vec![11, 12, 13], 3, 0, 3)])
        .unwrap();
    let old_hash = service
        .business_files()
        .unwrap()
        .into_iter()
        .find(|file| file.path == "/old.bin")
        .unwrap()
        .hash;
    service
        .business_upload(vec![upload_arg(
            "/shared.bin",
            vec![0; 32],
            vec![21, 22, 23, 24],
            4,
            0,
            4,
        )])
        .unwrap();
    let shared_hash = service
        .business_files()
        .unwrap()
        .into_iter()
        .find(|file| file.path == "/shared.bin")
        .unwrap()
        .hash;
    service.business_hashed_update(true).unwrap();
    service
        .business_upload(vec![upload_arg(
            "/old.bin",
            hex::decode(&shared_hash).unwrap(),
            vec![21, 22, 23, 24],
            4,
            0,
            4,
        )])
        .unwrap();
    let old_file = service
        .business_files()
        .unwrap()
        .into_iter()
        .find(|file| file.path == "/old.bin")
        .unwrap();
    assert_eq!(old_file.size, 4);
    assert_eq!(old_file.hash, shared_hash);
    service
        .business_upload(vec![upload_arg(
            "/old-hash-reused.bin",
            hex::decode(old_hash).unwrap(),
            vec![31, 32, 33],
            3,
            0,
            3,
        )])
        .unwrap();
    assert_eq!(
        service.business_download("/old-hash-reused.bin".to_string()).unwrap(),
        vec![31, 32, 33]
    );

    // 超过直接 query 响应上限的堆内文件必须能分片读取，并通过 HTTP streaming 完整下载。
    let streaming_path = "/streaming.bin";
    let streaming_data = (0..STREAMING_TEST_FILE_SIZE)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    for (index, chunk) in streaming_data.chunks(UPLOAD_TEST_CHUNK_SIZE).enumerate() {
        service
            .business_upload(vec![upload_arg(
                streaming_path,
                vec![0; 32],
                chunk.to_vec(),
                streaming_data.len() as u64,
                index as u32,
                UPLOAD_TEST_CHUNK_SIZE as u32,
            )])
            .unwrap();
    }
    let slice_offset = 2 * 1024 * 1024 - 2;
    assert_eq!(
        service
            .business_download_by(streaming_path.to_string(), slice_offset as u64, 5)
            .unwrap(),
        streaming_data[slice_offset..slice_offset + 5]
    );
    let response = request(
        &pic,
        canister_id,
        controller,
        CustomHttpRequest {
            url: streaming_path.to_string(),
            method: "GET".to_string(),
            body: vec![].into(),
            headers: vec![],
        },
    );
    assert_eq!(response.status_code, 200);
    let mut downloaded = response.body.into_vec();
    let mut token = match response.streaming_strategy {
        Some(StreamingStrategy::Callback { token, .. }) => Some(token),
        None => panic!("large asset response should use HTTP streaming"),
    };
    while let Some(current) = token {
        let bytes = pic
            .query_call(canister_id, controller, "http_streaming", encode_one(current).unwrap())
            .unwrap();
        let response = Decode!(&bytes, StreamingCallbackHttpResponse).unwrap();
        downloaded.extend_from_slice(&response.body);
        token = response.token;
    }
    assert_eq!(downloaded, streaming_data);
    service.business_delete(vec![streaming_path.to_string()]).unwrap();
    assert!(service.business_download(streaming_path.to_string()).is_err());

    // 单段 Range 返回 206 和请求的字节；越界范围返回 416。
    let response = request(
        &pic,
        canister_id,
        controller,
        CustomHttpRequest {
            url: "/partial.bin".to_string(),
            method: "GET".to_string(),
            body: vec![].into(),
            headers: vec![("Range".to_string(), "bytes=1-2".to_string())],
        },
    );
    assert_eq!(response.status_code, 206);
    assert_eq!(response.body, vec![2, 3]);
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| { name.eq_ignore_ascii_case("Content-Range") && value == "bytes 1-2/4" })
    );
    let response = request(
        &pic,
        canister_id,
        controller,
        CustomHttpRequest {
            url: "/partial.bin".to_string(),
            method: "GET".to_string(),
            body: vec![].into(),
            headers: vec![("Range".to_string(), "bytes=10-".to_string())],
        },
    );
    assert_eq!(response.status_code, 416);
    assert!(response.body.is_empty());

    // 伪造的 streaming token 应返回空响应，而不是让 query trap。
    let token = StreamingCallbackToken {
        path: "/partial.bin".to_string(),
        token: vec![
            ("start".to_string(), "4".to_string()),
            ("end".to_string(), "2".to_string()),
        ],
    };
    let bytes = pic
        .query_call(canister_id, controller, "http_streaming", encode_one(token).unwrap())
        .unwrap();
    let response = Decode!(&bytes, StreamingCallbackHttpResponse).unwrap();
    assert!(response.body.is_empty());
    assert!(response.token.is_none());

    // Explorer 注入的数据必须经过 JSON 和 script 上下文转义。
    let malicious_path = "/</script><script>alert(1)</script>.txt";
    service
        .business_upload(vec![upload_arg(malicious_path, vec![7; 32], vec![1], 1, 0, 1)])
        .unwrap();
    let response = request(
        &pic,
        canister_id,
        controller,
        CustomHttpRequest {
            url: "/".to_string(),
            method: "GET".to_string(),
            body: vec![].into(),
            headers: vec![],
        },
    );
    let body = String::from_utf8(response.body.into_vec()).unwrap();
    assert!(!body.contains(malicious_path));
    assert!(body.contains("\\u003c/script\\u003e"));

    // 大小、Header、上传批次和删除批次均由 Canister 最终拒绝。
    assert!(
        service
            .business_upload(vec![
                upload_arg("/too-large.bin", vec![8; 32], vec![1], u64::MAX, 0, 1,)
            ])
            .is_err()
    );
    let mut invalid_header = upload_arg("/invalid-header.bin", vec![9; 32], vec![1], 1, 0, 1);
    invalid_header.headers = vec![("X-Test".to_string(), "ok\r\nbad".to_string())];
    assert!(service.business_upload(vec![invalid_header]).is_err());
    assert!(
        service
            .business_upload(
                (0..65)
                    .map(|index| upload_arg(&format!("/batch-{index}.bin"), vec![10; 32], vec![1], 1, 0, 1))
                    .collect()
            )
            .is_err()
    );
    assert!(service.business_delete(vec!["/none".to_string(); 1001]).is_err());
}
