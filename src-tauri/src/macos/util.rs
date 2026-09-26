use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const PRODUCT_DIR: &str = "ClaudeWindowsCN";

pub fn home_dir() -> Result<PathBuf, String> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("无法读取当前用户主目录。")?;
    if !home.is_absolute() || home == Path::new("/") {
        return Err("当前用户主目录无效。".into());
    }
    Ok(home)
}

pub fn local_app_data() -> Result<PathBuf, String> {
    Ok(home_dir()?.join("Library/Application Support"))
}

pub fn data_dir() -> Result<PathBuf, String> {
    Ok(local_app_data()?.join(PRODUCT_DIR))
}

pub fn hide_window(_command: &mut Command) {}

pub fn run_helper_if_requested() -> bool {
    false
}

pub fn download(url: &str, destination: &Path) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("下载地址必须使用 HTTPS。".into());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let stage = destination.with_extension("download.partial");
    let result = (|| {
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .get(url)
            .header("User-Agent", "claude-windows-cn")
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|error| format!("下载失败：{}", error.without_url()))?;
        let expected = response.content_length();
        let mut file = fs::File::create(&stage).map_err(|e| e.to_string())?;
        let received = std::io::copy(&mut response, &mut file).map_err(|e| e.to_string())?;
        if received == 0 || expected.is_some_and(|size| size != received) {
            return Err("下载文件为空或不完整。".into());
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        replace_file(&stage, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}

pub fn sha256(path: &Path) -> Result<String, String> {
    let mut reader = std::io::BufReader::new(
        fs::File::open(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?,
    );
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn replace_file(stage: &Path, target: &Path) -> Result<(), String> {
    // POSIX rename atomically replaces a file in the same filesystem.
    fs::rename(stage, target).map_err(|error| format!("替换 {} 失败：{error}", target.display()))
}
