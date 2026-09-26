include!("../util_shared.rs");

use std::{
    env, fs,
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

pub(crate) fn shell_quote(value: impl AsRef<std::ffi::OsStr>) -> String {
    format!(
        "'{}'",
        value.as_ref().to_string_lossy().replace('\'', "'\\''")
    )
}

pub(crate) fn run_shell(script: &str, elevated: bool) -> Result<String, String> {
    let mut command = if elevated {
        let mut command = Command::new("/usr/bin/osascript");
        command
            .args([
                "-e",
                "on run argv",
                "-e",
                "do shell script (item 1 of argv) with administrator privileges",
                "-e",
                "end run",
                "--",
            ])
            .arg(format!("/bin/sh -eu -c {}", shell_quote(script)));
        command
    } else {
        let mut command = Command::new("/bin/sh");
        command.args(["-eu", "-c", script]);
        command
    };
    let output = command
        .output()
        .map_err(|error| format!("启动 macOS 文件操作失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "macOS 文件操作失败（退出码 {:?}）：{}{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
