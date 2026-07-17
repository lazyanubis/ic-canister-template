use std::{borrow::Cow, collections::HashMap};

use crate::stable::{Business, State};

#[derive(serde::Serialize)]
struct ExploreHeader<'a> {
    key: &'a str,
    value: &'a str,
}

#[derive(serde::Serialize)]
struct ExploreFile<'a> {
    path: &'a str,
    size: u64,
    headers: Vec<ExploreHeader<'a>>,
    created: i128,
    modified: i128,
    hash: &'a str,
}

pub const HTML: &str = include_str!("../web/index.html");
pub const CSS: &str = include_str!("../web/index.css");

pub fn explore<'a>(headers: &mut HashMap<&'a str, Cow<'a, str>>, state: &State) -> Vec<u8> {
    headers.insert("Content-Type", "text/html".into());

    let files = state.business_files();
    let files = files
        .iter()
        .map(|file| ExploreFile {
            path: &file.path,
            size: file.size,
            headers: file
                .headers
                .iter()
                .map(|(key, value)| ExploreHeader { key, value })
                .collect(),
            created: file.created.into_inner() / 1000000,
            modified: file.modified.into_inner() / 1000000,
            hash: &file.hash,
        })
        .collect::<Vec<_>>();
    let json = ic_canister_kit::common::trap(serde_json::to_string(&files));
    let json = escape_json_for_script(&json);

    HTML.replace("/* CSS */", CSS)
        .replace("const _files = [];", &format!("const _files = {};", json))[..]
        .into()
}

fn escape_json_for_script(json: &str) -> String {
    json.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
mod tests {
    use super::escape_json_for_script;

    #[test]
    fn escapes_script_breakout_characters() {
        let escaped = escape_json_for_script(r#"["</script><script>alert(1)</script>"]"#);
        assert!(!escaped.contains("</script>"));
        assert!(escaped.contains("\\u003c/script\\u003e"));
    }
}
