use std::{borrow::Cow, collections::HashMap};

use percent_encoding::percent_decode_str;
use regex::Regex;

use ic_canister_kit::http::MAX_RESPONSE_LENGTH;

use crate::explore::explore;
use crate::stable::State;
use crate::types::*;

// https://github.com/dfinity/examples/blob/8b01d548d8548a9d4558a7a1dbb49234d02d7d03/motoko/http_counter/src/main.mo

// #[ic_cdk::update]
// fn http_request_update(request: CustomHttpRequest) -> CustomHttpResponse {
//     todo!()
// }

// 请求数据
#[ic_cdk::query]
fn http_request(request: CustomHttpRequest) -> CustomHttpResponse {
    crate::stable::with_state(|state| inner_http_request(state, request))
}

#[inline]
fn inner_http_request(state: &State, req: CustomHttpRequest) -> CustomHttpResponse {
    let mut split_url = req.url.split('?');
    let request_headers = req.headers;

    let path = split_url.next().unwrap_or("/"); // 分割出 url，默认是 /
    let path = percent_decode_str(path).decode_utf8().unwrap_or(Cow::Borrowed(path));
    let params = split_url.next().unwrap_or(""); // 请求参数
    let params = percent_decode_str(params)
        .decode_utf8()
        .unwrap_or(Cow::Borrowed(params));

    // ic_cdk::println!("============== path: {} -> {}", req.url, path);
    // for (key, value) in request_headers.iter() {
    //     ic_cdk::println!("header: {}: {}", key, value);
    // }

    let mut code = 200; // 响应码默认是 200
    let mut headers: HashMap<&str, Cow<str>> = HashMap::new();
    let body: Vec<u8>;
    let mut streaming_strategy: Option<StreamingStrategy> = None;

    if path == "/" {
        body = explore(&mut headers, state); // 主页内容
    } else {
        // 根据路径找文件
        let file = state.business_assets_get_file(path.as_ref());
        if let Some(file) = file {
            let asset = state.business_assets_get(&file.hash);
            if let Some(asset) = asset {
                let (_body, _streaming_strategy): (Vec<u8>, Option<StreamingStrategy>) =
                    toast(&path, &params, &request_headers, file, asset, &mut code, &mut headers); // 有对应的文件
                body = _body;
                streaming_strategy = _streaming_strategy;
            } else {
                body = not_found(&mut code, &mut headers);
            }
        } else {
            body = not_found(&mut code, &mut headers);
        }
    }

    CustomHttpResponse {
        status_code: code,
        headers: headers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body,
        streaming_strategy,
        upgrade: None,
    }
}

#[inline]
fn toast<'a>(
    path: &str,
    params: &str,
    request_headers: &HashMap<String, String>,
    file: &'a AssetFile,
    asset: &AssetData,
    code: &mut u16,
    headers: &mut HashMap<&'a str, Cow<'a, str>>,
) -> (Vec<u8>, Option<StreamingStrategy>) {
    // 1. 设置 header
    let (offset, size, streaming_strategy) = set_headers(path, params, request_headers, file, code, headers);

    // 2. 返回指定的内容
    (
        (asset.slice(&file.hash, file.size, offset, size)).to_vec(),
        streaming_strategy,
    )
}

#[inline]
fn set_headers<'a>(
    path: &str,
    params: &str,
    request_headers: &HashMap<String, String>,
    file: &'a AssetFile,
    code: &mut u16,
    headers: &mut HashMap<&'a str, Cow<'a, str>>,
) -> (usize, usize, Option<StreamingStrategy>) {
    let size = match usize::try_from(file.size) {
        Ok(size) => size,
        Err(_) => ic_cdk::trap("Asset size exceeds the platform address range."),
    };

    // 用户设置的响应头先写入，Canister 管理的协议头随后覆盖。
    for (name, value) in file.headers.iter() {
        headers.insert(name, value.into());
    }

    // let mut gzip = false;
    // let mut content_type = "";
    // for (key, value) in file.headers.iter() {
    //     if &key.to_lowercase() == "content-type" {
    //         content_type = value;
    //     }
    //     if &key.to_lowercase() == "content-encoding" && value == "gzip" {
    //         gzip = true;
    //     }
    // }

    // 文件名下载
    if let Ok(reg) = Regex::new(r"attachment=(.*\..*)?(&.*)?$") {
        for cap in reg.captures_iter(params) {
            let mut file_name = cap.get(1).map(|m| &params[m.start()..m.end()]).unwrap_or("");
            if file_name.is_empty() {
                file_name = file.path.split('/').next_back().unwrap_or_default();
            }
            if !file_name.is_empty() {
                let file_name = sanitize_attachment_filename(file_name);
                headers.insert(
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", file_name).into(),
                ); // 下载文件名
            }
        }
    }

    // ! 这个时间库无法编译
    // use chrono::{TimeZone, Utc};
    // let modified = Utc.timestamp_nanos(file.modified as i64);
    // headers.insert("Last-Modified", modified.to_rfc2822().into());

    // 额外增加的请求头
    headers.insert("ETag", file.hash.hex().into()); // 缓存标识

    // 访问控制
    // headers.insert("Access-Control-Allow-Origin", "*".into());
    // headers.insert(
    //     "Access-Control-Allow-Methods",
    //     "HEAD, GET, POST, OPTIONS".into(),
    // );
    // headers.insert("Access-Control-Max-Age", "86400".into());

    headers.insert("Accept-Ranges", "bytes".into());

    // Range 设置，当前只支持单段 bytes 范围。
    let range = request_headers
        .iter()
        .find(|(key, _)| &key.to_lowercase() == "range")
        .map(|(_, value)| value.trim());
    let (offset, offset_end, ranged) = match range {
        Some(range) => match parse_http_range(range, size) {
            Ok((offset, offset_end)) => (offset, offset_end, true),
            Err(()) => {
                *code = 416;
                headers.insert("Content-Range", format!("bytes */{size}").into());
                return (size, 0, None);
            }
        },
        None => (0, size, false),
    };

    // ic_cdk::println!("---------- {} {} ----------", start, end);
    // 如果过长, 需要阶段显示
    let mut streaming_end = offset_end; // ! 末尾位置 不包含
    let mut streaming_strategy: Option<StreamingStrategy> = None;
    if offset.saturating_add(MAX_RESPONSE_LENGTH) < streaming_end {
        // 响应的范围太大了, 缩短为最大长度, 此时应当开启流式响应
        streaming_end = offset.saturating_add(MAX_RESPONSE_LENGTH); // ! 末尾位置 不包含
        streaming_strategy = Some(to_streaming_strategy(
            path.to_string(),
            streaming_end as u64,
            offset_end as u64,
        ));
    }

    if ranged {
        // https://developer.mozilla.org/zh-CN/docs/Web/HTTP/Headers/Content-Range
        // Content-Range: bytes 0-499/10000
        headers.insert(
            "Content-Range",
            format!("bytes {}-{}/{}", offset, offset_end - 1, size).into(), // 流式响应也要设置正确的内容范围
        );
    }

    // ! 长度设置了会出错
    // headers.insert("Content-Length", format!("{}", offset_end - offset).into()); // ? 这个应该是本次返回的长度

    // 如果是视频可能需要返回其他的
    *code = if ranged { 206 } else { 200 };

    (offset, streaming_end - offset, streaming_strategy)
}

fn parse_http_range(range: &str, size: usize) -> Result<(usize, usize), ()> {
    let range = range.strip_prefix("bytes=").ok_or(())?;
    if size == 0 || range.contains(',') {
        return Err(());
    }
    let (start, end) = range.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix = end.parse::<usize>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        return Ok((size.saturating_sub(suffix.min(size)), size));
    }

    let start = start.parse::<usize>().map_err(|_| ())?;
    if start >= size {
        return Err(());
    }
    if end.is_empty() {
        return Ok((start, size));
    }

    let end = end.parse::<usize>().map_err(|_| ())?;
    if end < start {
        return Err(());
    }
    Ok((start, end.min(size - 1) + 1))
}

fn sanitize_attachment_filename(file_name: &str) -> String {
    file_name
        .chars()
        .map(|character| match character {
            '"' | '\\' | '\r' | '\n' => '_',
            character => character,
        })
        .collect()
}

// 找不到对应的文件
#[inline]
fn not_found<'a>(code: &mut u16, headers: &mut HashMap<&'a str, Cow<'a, str>>) -> Vec<u8> {
    *code = 404;

    headers.insert("Content-Type", "text/plain".into());

    b"Not found"[..].into()
}

#[inline]
fn to_streaming_strategy(path: String, offset: u64, offset_end: u64) -> StreamingStrategy {
    StreamingStrategy::Callback {
        callback: HttpRequestStreamingCallback::new(ic_cdk::api::canister_self(), "http_streaming".into()),
        token: to_streaming_token(path, offset, offset_end),
    }
}
#[inline]
fn to_streaming_token(path: String, offset: u64, offset_end: u64) -> StreamingCallbackToken {
    StreamingCallbackToken {
        path,
        token: {
            let mut token = HashMap::new();
            token.insert("start".into(), offset.to_string()); // ! 新的位置 包含
            token.insert("end".into(), offset_end.to_string()); // ! 末尾位置 不包含
            token
        },
    }
}
#[inline]
fn from_streaming_token(
    StreamingCallbackToken { path, token }: StreamingCallbackToken,
) -> Result<(String, u64, u64), ()> {
    match (
        token.get("start").map(|s| s.parse()),
        token.get("end").map(|e| e.parse()),
    ) {
        (Some(Ok(start)), Some(Ok(end))) => Ok((path, start, end)),
        _ => Err(()),
    }
}

// 流式响应回调
#[ic_cdk::query]
fn http_streaming(token: StreamingCallbackToken) -> StreamingCallbackHttpResponse {
    // ic_cdk::println!(
    //     "http_streaming: {:?} {:?} {:?} {:?} {:?}",
    //     path,
    //     params,
    //     headers,
    //     start,
    //     end,
    // );
    let (path, start, end) = match from_streaming_token(token) {
        Ok((path, start, end)) => (path, start, end),
        _ => return StreamingCallbackHttpResponse::empty(),
    };
    if start >= end {
        return StreamingCallbackHttpResponse::empty();
    }
    crate::stable::with_state(|state| {
        let file = state.business_assets_get_file(&path);
        if let Some(file) = file {
            let asset = state.business_assets_get(&file.hash);
            if let Some(asset) = asset {
                let file_size = match usize::try_from(file.size) {
                    Ok(file_size) => file_size,
                    Err(_) => return StreamingCallbackHttpResponse::empty(),
                };
                let offset = match usize::try_from(start) {
                    Ok(offset) => offset,
                    Err(_) => return StreamingCallbackHttpResponse::empty(),
                };
                let offset_end = match usize::try_from(end) {
                    Ok(offset_end) => offset_end,
                    Err(_) => return StreamingCallbackHttpResponse::empty(),
                };
                if offset >= offset_end || offset_end > file_size {
                    return StreamingCallbackHttpResponse::empty();
                }
                // 如果过长, 需要阶段显示
                let streaming_end = offset.saturating_add(MAX_RESPONSE_LENGTH).min(offset_end);
                return StreamingCallbackHttpResponse {
                    body: asset
                        .slice(&file.hash, file.size, offset, streaming_end - offset)
                        .to_vec(),
                    token: ((streaming_end as u64) < end).then(|| to_streaming_token(path, streaming_end as u64, end)),
                };
            }
        }
        StreamingCallbackHttpResponse {
            body: vec![],
            token: None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_http_range, sanitize_attachment_filename};

    #[test]
    fn parses_supported_byte_ranges() {
        assert_eq!(parse_http_range("bytes=1-3", 10), Ok((1, 4)));
        assert_eq!(parse_http_range("bytes=4-", 10), Ok((4, 10)));
        assert_eq!(parse_http_range("bytes=-3", 10), Ok((7, 10)));
        assert_eq!(parse_http_range("bytes=8-99", 10), Ok((8, 10)));
    }

    #[test]
    fn rejects_unsatisfiable_or_multiple_ranges() {
        assert_eq!(parse_http_range("bytes=10-", 10), Err(()));
        assert_eq!(parse_http_range("bytes=4-2", 10), Err(()));
        assert_eq!(parse_http_range("bytes=0-1,4-5", 10), Err(()));
        assert_eq!(parse_http_range("items=0-1", 10), Err(()));
    }

    #[test]
    fn sanitizes_attachment_header_filename() {
        assert_eq!(sanitize_attachment_filename("a\"b\r\n.txt"), "a_b__.txt");
    }
}
