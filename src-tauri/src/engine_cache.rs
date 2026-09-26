use crate::{operation::OperationState, util};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Serialize, Deserialize)]
pub(crate) struct EngineCache {
    pub source: String,
    pub etag: Option<String>,
    pub sha256: String,
}

pub(crate) fn download_engine(
    url: &str,
    cache_dir: &Path,
    zip: &Path,
    operation: &OperationState,
) -> Result<EngineCache, String> {
    let cached = fs::read(cache_dir.join("metadata.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<EngineCache>(&bytes).ok())
        .filter(|cache| {
            cache.source == url
                && cache.sha256.len() == 64
                && cache.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                && util::sha256(&cache_dir.join(format!("{}.zip", cache.sha256)))
                    .is_ok_and(|sha| sha == cache.sha256)
        });
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|error| error.without_url().to_string())?;
    let mut request = client.get(url).header("Cache-Control", "no-cache");
    if let Some(etag) = cached.as_ref().and_then(|cache| cache.etag.as_ref()) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let mut response = request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("检查最新汉化引擎失败：{}", error.without_url()))?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let cache = cached
            .filter(|cache| cache.etag.is_some())
            .ok_or("服务器返回未修改，但没有可验证的汉化引擎缓存。")?;
        fs::copy(cache_dir.join(format!("{}.zip", cache.sha256)), zip)
            .map_err(|error| error.to_string())?;
        if util::sha256(zip)? != cache.sha256 {
            return Err("汉化引擎缓存读取期间发生变化，已停止操作。".into());
        }
        operation.log("已在线确认汉化引擎未变化，校验通过，复用本地缓存。");
        return Ok(cache);
    }
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!("汉化引擎下载响应异常：{}", response.status()));
    }
    operation.log("下载最新汉化引擎");
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let expected = response.content_length();
    let mut file = fs::File::create(zip).map_err(|error| error.to_string())?;
    let received = std::io::copy(&mut response, &mut file)
        .map_err(|error| format!("下载汉化引擎失败：{error}"))?;
    if received == 0 || expected.is_some_and(|length| length != received) {
        return Err("汉化引擎下载不完整，已停止操作。".into());
    }
    drop(file);
    Ok(EngineCache {
        source: url.into(),
        etag,
        sha256: util::sha256(zip)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };
    #[test]
    fn engine_cache_revalidates_and_rejects_corruption_and_network_errors() {
        use std::io::{Read, Write};
        let root = env::temp_dir().join(format!(
            "engine-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/engine", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (status, etag, body, conditional) in [
                ("200 OK", "v1", "archive-one", false),
                ("304 Not Modified", "v1", "", true),
                ("200 OK", "v2", "archive-two", true),
                ("200 OK", "v2", "archive-two", false),
                ("503 Service Unavailable", "v2", "", true),
                ("304 Not Modified", "v2", "", false),
                ("200 OK", "", "archive-three", false),
                ("200 OK", "", "archive-three", false),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(request.contains("cache-control: no-cache"));
                assert_eq!(request.contains("if-none-match:"), conditional);
                let etag_header = if etag.is_empty() {
                    String::new()
                } else {
                    format!("ETag: \"{etag}\"\r\n")
                };
                write!(stream, "HTTP/1.1 {status}\r\n{etag_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let operation = OperationState::new();
        let zip = root.join("run.zip");
        let publish = |cache: &EngineCache| {
            fs::copy(&zip, root.join(format!("{}.zip", cache.sha256))).unwrap();
            fs::write(
                root.join("metadata.json"),
                serde_json::to_vec(cache).unwrap(),
            )
            .unwrap();
        };
        let first = download_engine(&url, &root, &zip, &operation).unwrap();
        publish(&first);
        let second = download_engine(&url, &root, &zip, &operation).unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert!(operation
            .snapshot()
            .logs
            .iter()
            .any(|line| line.contains("复用本地缓存")));
        let changed = download_engine(&url, &root, &zip, &operation).unwrap();
        assert_ne!(first.sha256, changed.sha256);
        publish(&changed);
        fs::write(root.join(format!("{}.zip", changed.sha256)), "broken").unwrap();
        let repaired = download_engine(&url, &root, &zip, &operation).unwrap();
        assert_eq!(changed.sha256, repaired.sha256);
        publish(&repaired);
        assert!(download_engine(&url, &root, &zip, &operation).is_err());
        fs::remove_file(root.join("metadata.json")).unwrap();
        assert!(download_engine(&url, &root, &zip, &operation).is_err());
        let no_etag = download_engine(&url, &root, &zip, &operation).unwrap();
        assert!(no_etag.etag.is_none());
        publish(&no_etag);
        assert_eq!(
            download_engine(&url, &root, &zip, &operation)
                .unwrap()
                .sha256,
            no_etag.sha256
        );
        server.join().unwrap();
        assert!(download_engine(&url, &root, &zip, &operation).is_err());
        assert_eq!(
            util::sha256(&root.join(format!("{}.zip", repaired.sha256))).unwrap(),
            repaired.sha256
        );
        fs::remove_dir_all(root).unwrap();
    }
}
