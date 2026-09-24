use crate::{
    claude,
    operation::{OperationOutcome, OperationState},
    util::{self, ps_path, ps_quote},
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const OWNER_REPO: &str = "chrichuang218/claude-windows-cn";
const ASSET: &str = "claude-windows-cn.exe";
const CHECKSUM: &str = "claude-windows-cn.exe.sha256";
static CACHE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug)]
struct ReleaseInfo {
    version: String,
    asset_url: String,
    checksum_url: String,
    release_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedUpdate {
    pub checked_at: String,
    pub update_available: Option<bool>,
    pub latest_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateResult {
    pub ok: bool,
    pub message: String,
}

pub fn last_update_result() -> Result<Option<UpdateResult>, String> {
    let path = util::data_dir()?
        .join("self-update")
        .join("apply-result.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let result = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("助手更新结果无效：{error}"))?;
    Ok(Some(result))
}

#[derive(Deserialize)]
struct ReleaseResponse {
    status: u16,
    content: String,
}

fn parse_release_response(json: &str, allow_missing: bool) -> Result<Option<Release>, String> {
    let response: ReleaseResponse =
        serde_json::from_str(json).map_err(|error| format!("GitHub API 响应无效：{error}"))?;
    match response.status {
        404 if allow_missing => Ok(None),
        200 => serde_json::from_str(&response.content)
            .map(Some)
            .map_err(|error| format!("GitHub Release 数据无效：{error}")),
        code => Err(format!("GitHub API HTTP {code}")),
    }
}

fn release_api(url: &str, allow_missing: bool) -> Result<Option<Release>, String> {
    let script = format!(
        "$ProgressPreference='SilentlyContinue'; try {{ $response=Invoke-WebRequest -Uri {} -Headers @{{'User-Agent'='claude-windows-cn'; 'Accept'='application/vnd.github+json'}} -UseBasicParsing -TimeoutSec 30; @{{status=[int]$response.StatusCode;content=[string]$response.Content}} | ConvertTo-Json -Compress }} catch {{ $response=$_.Exception.Response; if (-not $response) {{ throw }}; @{{status=[int]$response.StatusCode;content=''}} | ConvertTo-Json -Compress }}",
        ps_quote(url)
    );
    let json = util::powershell(&script)?;
    parse_release_response(&json, allow_missing)
}

fn latest_release() -> Result<Option<ReleaseInfo>, String> {
    let latest = format!("https://github.com/{OWNER_REPO}/releases/latest");
    let redirect_script = format!(
        "$request=[System.Net.HttpWebRequest]::Create({}); $request.AllowAutoRedirect=$false; $request.UserAgent='claude-windows-cn'; try {{ $response=$request.GetResponse() }} catch [System.Net.WebException] {{ $response=$_.Exception.Response; if (-not $response) {{ throw }} }}; try {{ if ([int]$response.StatusCode -eq 404) {{ '' }} else {{ $response.Headers['Location'] }} }} finally {{ $response.Close() }}",
        ps_quote(&latest)
    );
    let tag = util::powershell(&redirect_script)
        .ok()
        .and_then(|redirect| {
            redirect.split_once("/releases/tag/").map(|(_, value)| {
                value
                    .split(['?', '#', '/'])
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
        })
        .filter(|value| !value.is_empty());
    let release = match tag {
        Some(tag) => release_api(
            &format!("https://api.github.com/repos/{OWNER_REPO}/releases/tags/{tag}"),
            false,
        ),
        None => release_api(
            &format!("https://api.github.com/repos/{OWNER_REPO}/releases/latest"),
            true,
        ),
    }
    .map_err(|error| {
        if error.contains("HTTP 403") {
            "GitHub API 限流或拒绝访问。".to_string()
        } else {
            format!("查询助手 Release 失败：{error}")
        }
    })?;
    release.map(validate_release_assets).transpose()
}

fn validate_release_assets(release: Release) -> Result<ReleaseInfo, String> {
    let executable = release
        .assets
        .iter()
        .find(|asset| asset.name == ASSET)
        .ok_or_else(|| format!("最新 Release 缺少 {ASSET}。"))?;
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUM)
        .ok_or_else(|| format!("最新 Release 缺少 {CHECKSUM}。"))?;
    for asset in [executable, checksum] {
        if !asset.browser_download_url.starts_with(&format!(
            "https://github.com/{OWNER_REPO}/releases/download/"
        )) {
            return Err("Release 文件下载地址不属于本助手仓库。".into());
        }
    }
    let version = release.tag_name.trim_start_matches('v').to_string();
    if version.is_empty()
        || !version
            .chars()
            .all(|value| value.is_ascii_digit() || value == '.')
    {
        return Err("Release 版本号无法识别。".into());
    }
    Ok(ReleaseInfo {
        version,
        asset_url: executable.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
        release_url: release.html_url,
    })
}

pub fn check(operation: &OperationState) -> Result<OperationOutcome, String> {
    operation.step("查询助手新版本");
    let result = latest_release();
    save_assistant_cache(&assistant_check_cache(&result))?;
    assistant_check_outcome(result?, operation)
}

fn assistant_check_outcome(
    release: Option<ReleaseInfo>,
    operation: &OperationState,
) -> Result<OperationOutcome, String> {
    let Some(release) = release else {
        return Ok(OperationOutcome {
            message: "暂无可用的助手更新。".into(),
            update_available: Some(false),
            latest_version: None,
        });
    };
    let available = claude::compare_versions(&release.version, env!("CARGO_PKG_VERSION"))
        == std::cmp::Ordering::Greater;
    operation.log(format!(
        "当前版本：{}；最新版本：{}；来源：{}",
        env!("CARGO_PKG_VERSION"),
        release.version,
        release.release_url
    ));
    Ok(OperationOutcome::update(
        if available {
            format!("发现助手新版本 {}。", release.version)
        } else {
            format!("助手已是最新版本 {}。", release.version)
        },
        available,
        release.version,
    ))
}

pub fn install(operation: &OperationState) -> Result<OperationOutcome, String> {
    operation.step("查询助手 Release");
    let release = latest_release()?.ok_or("暂无可安装的助手发行版本。")?;
    if claude::compare_versions(&release.version, env!("CARGO_PKG_VERSION"))
        != std::cmp::Ordering::Greater
    {
        return Ok(OperationOutcome::done(format!(
            "助手已是最新版本 {}。",
            env!("CARGO_PKG_VERSION")
        )));
    }
    let work = util::data_dir()?.join("self-update");
    fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let executable = work.join(format!("claude-windows-cn-{}.exe", release.version));
    let checksum = work.join(CHECKSUM);
    operation.step("下载助手程序及 SHA256 摘要");
    util::download(&release.asset_url, &executable)?;
    util::download(&release.checksum_url, &checksum)?;
    let actual = verify_checksum(&executable, &checksum)?;
    operation.log(format!("下载地址：{}；SHA256：{actual}", release.asset_url));
    operation.step("运行新版助手无副作用自检");
    run_self_test(&executable)?;
    let target = env::current_exe().map_err(|error| error.to_string())?;
    if target.file_name().and_then(|name| name.to_str()) != Some(ASSET) {
        return Err("当前程序名称不是正式助手 exe，不能原位自更新。".into());
    }
    operation.step("准备替换助手程序");
    let script = work.join("apply-update.ps1");
    let backup = work.join("claude-windows-cn.exe.bak");
    let result = work.join("apply-result.json");
    if result.exists() {
        fs::remove_file(&result).map_err(|error| error.to_string())?;
    }
    let content = updater_script(std::process::id(), &target, &executable, &backup, &result);
    util::write_script(&script, &content)?;
    let elevated = target.starts_with(
        env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_default(),
    );
    let args = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\"",
        script.display()
    );
    let launch = format!(
        "Start-Process -FilePath 'powershell.exe' -ArgumentList {} {} -WindowStyle Hidden | Out-Null",
        ps_quote(args), if elevated { "-Verb RunAs" } else { "" }
    );
    util::powershell(&launch).map_err(|error| format!("更新替换进程未启动：{error}"))?;
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(2));
        std::process::exit(0);
    });
    Ok(OperationOutcome::done(
        "新版助手已通过校验和自检，替换进程已启动；程序退出后执行更新。",
    ))
}

fn updater_script(pid: u32, target: &Path, staged: &Path, backup: &Path, result: &Path) -> String {
    format!(
        r#"$ErrorActionPreference='Stop'
        $target={target}; $staged={staged}; $backup={backup}; $result={result}
        try {{
            Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue
            if (-not (Test-Path -LiteralPath $staged)) {{ throw '新版程序不存在。' }}
            Copy-Item -LiteralPath $target -Destination $backup -Force -ErrorAction Stop
            Copy-Item -LiteralPath $staged -Destination $target -Force -ErrorAction Stop
            $process=Start-Process -FilePath $target -PassThru -ErrorAction Stop
            if (-not $process) {{ throw '新版助手未启动。' }}
            @{{ ok=$true; message='更新成功。' }} | ConvertTo-Json -Compress | Set-Content -LiteralPath $result -Encoding UTF8
        }} catch {{
            $reason=$_.Exception.Message
            if (Test-Path -LiteralPath $backup) {{
                try {{ Copy-Item -LiteralPath $backup -Destination $target -Force -ErrorAction Stop }}
                catch {{ $reason += '; 回退失败：' + $_.Exception.Message }}
            }}
            @{{ ok=$false; message=$reason }} | ConvertTo-Json -Compress | Set-Content -LiteralPath $result -Encoding UTF8
        }}
    "#,
        pid = pid,
        target = ps_path(target),
        staged = ps_path(staged),
        backup = ps_path(backup),
        result = ps_path(result),
    )
}

fn parse_checksum(raw: &str) -> Result<String, String> {
    let parts = raw
        .trim_start_matches('\u{feff}')
        .split_whitespace()
        .collect::<Vec<_>>();
    let hash = parts.first().copied().unwrap_or("");
    if hash.len() != 64 || !hash.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err("Release SHA256 文件内容无效。".into());
    }
    if parts.len() > 1 && parts[1].trim_start_matches('*') != ASSET {
        return Err("Release SHA256 文件指向其他程序。".into());
    }
    Ok(hash.to_ascii_lowercase())
}

fn verify_checksum(executable: &Path, checksum: &Path) -> Result<String, String> {
    let expected =
        parse_checksum(&fs::read_to_string(checksum).map_err(|error| error.to_string())?)?;
    let actual = util::sha256(executable)?;
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "助手更新摘要不匹配：期望 {expected}，实际 {actual}。"
        ));
    }
    Ok(actual)
}

fn run_self_test(executable: &Path) -> Result<(), String> {
    let mut command = Command::new(executable);
    util::hide_window(&mut command);
    let mut child = command
        .arg("--self-test")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("新版助手自检启动失败：{error}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("新版助手自检失败，退出码 {:?}。", status.code()))
            };
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            return Err("新版助手自检超时。".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn self_test() -> bool {
    env!("CARGO_PKG_NAME") == "claude-windows-cn"
        && env::current_exe().ok().is_some_and(|path| path.is_file())
        && std::panic::catch_unwind(|| {
            let _: tauri::Context<tauri::Wry> = tauri::generate_context!();
        })
        .is_ok()
}

fn cache_path() -> Result<PathBuf, String> {
    Ok(util::data_dir()?.join("daily-update.json"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyCache {
    assistant: Option<CachedUpdate>,
    claude: Option<CachedUpdate>,
    checked_epoch: u64,
}

fn read_cache() -> Result<DailyCache, String> {
    let path = cache_path()?;
    if !path.exists() {
        return Ok(DailyCache {
            assistant: None,
            claude: None,
            checked_epoch: 0,
        });
    }
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("读取每日更新记录失败：{error}"))
}

fn write_cache(cache: &DailyCache) -> Result<(), String> {
    let path = cache_path()?;
    fs::create_dir_all(path.parent().ok_or("更新记录目录无效。")?)
        .map_err(|error| error.to_string())?;
    let stage = path.with_extension("json.tmp");
    fs::write(
        &stage,
        serde_json::to_vec_pretty(cache).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    util::replace_file(&stage, &path)
}

fn save_assistant_cache(update: &CachedUpdate) -> Result<(), String> {
    let _guard = CACHE_LOCK.lock().map_err(|error| error.to_string())?;
    let mut cache = read_cache()?;
    cache.assistant = Some(update.clone());
    write_cache(&cache)
}

pub fn cached_assistant_update() -> Result<Option<CachedUpdate>, String> {
    let _guard = CACHE_LOCK.lock().map_err(|error| error.to_string())?;
    Ok(read_cache()?.assistant)
}

pub fn cached_claude_update() -> Result<Option<CachedUpdate>, String> {
    let _guard = CACHE_LOCK.lock().map_err(|error| error.to_string())?;
    Ok(read_cache()?.claude)
}

pub fn save_claude_check(result: &Result<(bool, String), String>) -> Result<(), String> {
    let _guard = CACHE_LOCK.lock().map_err(|error| error.to_string())?;
    let mut cache = read_cache()?;
    cache.claude = Some(claude_check_cache(result));
    write_cache(&cache)
}

fn claude_check_cache(result: &Result<(bool, String), String>) -> CachedUpdate {
    match result {
        Ok((available, version)) => CachedUpdate {
            checked_at: now_string(),
            update_available: Some(*available),
            latest_version: Some(version.clone()),
            error: None,
        },
        Err(error) => CachedUpdate {
            checked_at: now_string(),
            update_available: None,
            latest_version: None,
            error: Some(error.clone()),
        },
    }
}

fn assistant_check_cache(result: &Result<Option<ReleaseInfo>, String>) -> CachedUpdate {
    match result {
        Ok(release) => CachedUpdate {
            checked_at: now_string(),
            update_available: Some(release.as_ref().is_some_and(|info| {
                claude::compare_versions(&info.version, env!("CARGO_PKG_VERSION"))
                    == std::cmp::Ordering::Greater
            })),
            latest_version: release.as_ref().map(|info| info.version.clone()),
            error: None,
        },
        Err(error) => CachedUpdate {
            checked_at: now_string(),
            update_available: None,
            latest_version: None,
            error: Some(error.clone()),
        },
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_string() -> String {
    now_epoch().to_string()
}

pub fn start_daily_scheduler() {
    thread::spawn(|| loop {
        if let Ok(config) = crate::assistant::load_config() {
            if config.daily_update_check {
                let _ = daily_check_if_due();
            }
        }
        thread::sleep(Duration::from_secs(60 * 60));
    });
}

pub fn trigger_daily_check() {
    thread::spawn(|| {
        let _ = daily_check_if_due();
    });
}

fn daily_check_if_due() -> Result<(), String> {
    let previous_claude = {
        let _guard = CACHE_LOCK.lock().map_err(|error| error.to_string())?;
        let cache = read_cache()?;
        if now_epoch().saturating_sub(cache.checked_epoch) < 24 * 60 * 60 {
            return Ok(());
        }
        cache.claude
    };
    let assistant = Some(assistant_check_cache(&latest_release()));
    let claude = Some(claude_check_cache(&claude::daily_update_info()));
    let _guard = CACHE_LOCK.lock().map_err(|error| error.to_string())?;
    let mut cache = read_cache()?;
    if now_epoch().saturating_sub(cache.checked_epoch) < 24 * 60 * 60 {
        return Ok(());
    }
    cache.assistant = assistant;
    // A manual check may finish while the daily network requests are pending.
    if cache.claude == previous_claude {
        cache.claude = claude;
    }
    cache.checked_epoch = now_epoch();
    write_cache(&cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_release_is_a_normal_check_result_but_other_failures_remain_errors() {
        let missing = r#"{"status":404,"content":""}"#;
        assert!(parse_release_response(missing, true).unwrap().is_none());
        // A tag that disappeared after the redirect is not proof of no releases.
        assert!(parse_release_response(missing, false).is_err());
        for status in [403, 429, 500] {
            let json = format!(r#"{{"status":{status},"content":""}}"#);
            assert!(parse_release_response(&json, true).is_err());
        }
        assert!(parse_release_response(r#"{"status":200,"content":"null"}"#, true).is_err());
        assert!(parse_release_response("invalid response", true).is_err());
        let cached = assistant_check_cache(&Ok(None));
        assert_eq!(cached.update_available, Some(false));
        assert_eq!(cached.latest_version, None);
        assert_eq!(cached.error, None);
        let outcome = assistant_check_outcome(None, &OperationState::new()).unwrap();
        assert_eq!(outcome.update_available, Some(false));
        assert_eq!(outcome.latest_version, None);
        assert_eq!(outcome.message, "暂无可用的助手更新。");
        let failed = assistant_check_cache(&Err("network unavailable".into()));
        assert_eq!(failed.update_available, None);
        assert_eq!(failed.latest_version, None);
        assert_eq!(failed.error.as_deref(), Some("network unavailable"));
    }

    #[test]
    fn claude_check_cache_preserves_success_and_clears_stale_results_on_error() {
        for available in [true, false] {
            let cached = claude_check_cache(&Ok((available, "2.7032.0".into())));
            let restored: CachedUpdate =
                serde_json::from_slice(&serde_json::to_vec(&cached).unwrap()).unwrap();
            assert_eq!(restored.update_available, Some(available));
            assert_eq!(restored.latest_version.as_deref(), Some("2.7032.0"));
            assert!(restored.error.is_none());
            assert!(!restored.checked_at.is_empty());
        }
        let failed = claude_check_cache(&Err("network unavailable".into()));
        assert_eq!(failed.update_available, None);
        assert_eq!(failed.latest_version, None);
        assert_eq!(failed.error.as_deref(), Some("network unavailable"));
    }

    fn fake_release(names: &[&str]) -> Release {
        Release {
            tag_name: "v9.9.9".into(),
            html_url: format!("https://github.com/{OWNER_REPO}/releases/tag/v9.9.9"),
            assets: names
                .iter()
                .map(|name| Asset {
                    name: (*name).into(),
                    browser_download_url: format!(
                        "https://github.com/{OWNER_REPO}/releases/download/v9.9.9/{name}"
                    ),
                })
                .collect(),
        }
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = env::temp_dir().join(format!(
                "claude-cn-self-update-test-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in [
                "new.exe",
                "claude-windows-cn.exe",
                "checksum.sha256",
                "old.bak",
                "result.json",
                "update.ps1",
            ] {
                let _ = fs::remove_file(self.path(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn release_requires_exe_and_checksum_assets() {
        assert!(validate_release_assets(fake_release(&[ASSET, CHECKSUM])).is_ok());
        assert!(validate_release_assets(fake_release(&[CHECKSUM]))
            .unwrap_err()
            .contains(ASSET));
        assert!(validate_release_assets(fake_release(&[ASSET]))
            .unwrap_err()
            .contains(CHECKSUM));
    }

    #[test]
    fn checksum_requires_own_exe_name() {
        let hash = "a".repeat(64);
        assert!(parse_checksum(&format!("{hash}  {ASSET}")).is_ok());
        assert!(parse_checksum(&format!("{hash}  other.exe")).is_err());
        assert!(parse_checksum("broken").is_err());
    }

    #[test]
    fn checksum_mismatch_and_invalid_exe_fail_before_replacement() {
        let fixture = Fixture::new();
        let exe = fixture.path("new.exe");
        let checksum = fixture.path("checksum.sha256");
        fs::write(&exe, b"not a Windows executable").unwrap();
        fs::write(&checksum, format!("{}  {ASSET}", "a".repeat(64))).unwrap();
        assert!(verify_checksum(&exe, &checksum)
            .unwrap_err()
            .contains("摘要不匹配"));
        assert!(run_self_test(&exe).is_err());
    }

    #[test]
    fn rollback_script_restores_even_when_target_exists() {
        let script = updater_script(
            42,
            Path::new("C:\\tool.exe"),
            Path::new("C:\\next.exe"),
            Path::new("C:\\old.exe"),
            Path::new("C:\\result.json"),
        );
        assert!(script.contains("Copy-Item -LiteralPath $backup -Destination $target -Force"));
    }

    #[cfg(windows)]
    #[test]
    fn failed_local_replacement_restores_old_exe() {
        let fixture = Fixture::new();
        let target = fixture.path("claude-windows-cn.exe");
        let staged = fixture.path("new.exe");
        let backup = fixture.path("old.bak");
        let result = fixture.path("result.json");
        let script = fixture.path("update.ps1");
        fs::write(&target, b"old executable bytes").unwrap();
        fs::write(&staged, b"invalid new executable bytes").unwrap();
        // A nonexistent PID lets the script enter the replacement branch without waiting.
        util::write_script(
            &script,
            &updater_script(2_000_000_000, &target, &staged, &backup, &result),
        )
        .unwrap();
        util::run_script(&script).unwrap();
        let recorded: UpdateResult = serde_json::from_str(
            fs::read_to_string(&result)
                .unwrap()
                .trim_start_matches('\u{feff}'),
        )
        .unwrap();
        assert!(!recorded.ok, "invalid executable must fail to start");
        assert!(!recorded.message.is_empty());
        assert_eq!(fs::read(&backup).unwrap(), b"old executable bytes");
        assert_eq!(fs::read(&target).unwrap(), b"old executable bytes");
    }
}
