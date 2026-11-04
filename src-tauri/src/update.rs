use crate::db;
use crate::logging;
use crate::settings;
use crate::state::AppState;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Result of the most recent check. Installation only accepts this value, so the
/// UI can never hand the backend an arbitrary download URL or checksum.
static LAST_CHECK: Mutex<Option<UpdateInfo>> = Mutex::new(None);

fn cache(info: &UpdateInfo) {
    if let Ok(mut slot) = LAST_CHECK.lock() {
        *slot = Some(info.clone());
    }
}

/// Result of the last check, if any; lets a freshly opened window show it without network access.
pub fn last_result() -> Option<UpdateInfo> {
    LAST_CHECK.lock().ok().and_then(|slot| slot.clone())
}

/// Repository that hosts the releases used for updates.
const REPO: &str = "Jaspersol/LiteScriptManager";
const USER_AGENT: &str = "LiteScriptManager";
/// Do not hit the GitHub API more than once per interval during automatic checks.
const AUTO_CHECK_INTERVAL_MS: i64 = 6 * 60 * 60 * 1000;
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const PROGRESS_STEP_BYTES: u64 = 128 * 1024;
const DOWNLOAD_HOSTS: &[&str] = &[
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "githubusercontent.com",
];

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_name: Option<String>,
    pub notes: Option<String>,
    pub published_at: Option<String>,
    pub release_url: Option<String>,
    pub asset_name: Option<String>,
    pub asset_size: Option<u64>,
    pub download_url: Option<String>,
    /// API endpoint that also serves the same asset (used when github.com is unreachable).
    pub mirror_url: Option<String>,
    pub sha256: Option<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct ProgressPayload {
    downloaded: u64,
    total: Option<u64>,
}

/// Runs an update check on behalf of the UI and records the timestamp.
pub async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    let info = fetch_latest(app).await?;
    record_check(app);
    cache(&info);
    Ok(info)
}

/// Automatic check used on startup: obeys the setting and the interval throttle.
pub async fn auto_check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        if !settings::get_bool(&conn, "auto_check_update", true) {
            return Ok(None);
        }
        let last = settings::get_i64(&conn, "last_update_check_ms", 0);
        if db::now_ms().saturating_sub(last) < AUTO_CHECK_INTERVAL_MS {
            return Ok(None);
        }
    }
    match fetch_latest(app).await {
        Ok(info) => {
            record_check(app);
            cache(&info);
            Ok(info.has_update.then_some(info))
        }
        Err(error) => {
            logging::app_log("WARN", &format!("自动检查更新失败: {error}"));
            record_check(app);
            Ok(None)
        }
    }
}

fn record_check(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>()
        && let Ok(conn) = state.db.lock()
    {
        let _ = db::set_setting(&conn, "last_update_check_ms", &db::now_ms().to_string());
    }
}

fn latest_release_api() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

async fn fetch_latest(app: &AppHandle) -> Result<UpdateInfo, String> {
    let client = http_client()?;
    let response = client
        .get(latest_release_api())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("连接 GitHub 失败: {}", describe_error(&e)))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err("GitHub 仓库还没有发布任何版本".to_string());
    }
    if !status.is_success() {
        if status == reqwest::StatusCode::FORBIDDEN {
            let limited = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim() == "0")
                .unwrap_or(false);
            if limited {
                return Err("GitHub API 访问次数已用完，请稍后再试".to_string());
            }
        }
        return Err(format!("检查更新失败：HTTP {}", status.as_u16()));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析发布信息失败: {e}"))?;
    let current = app.package_info().version.to_string();
    let mut info = parse_release(&current, &body);
    if let Some(name) = info.asset_name.clone()
        && let Some(checksum_url) = checksum_asset_url(&body, &name)
        && let Ok(Some(checksum)) = fetch_checksum(&client, &checksum_url).await
    {
        info.sha256 = Some(checksum);
    }
    Ok(info)
}

fn parse_release(current: &str, body: &serde_json::Value) -> UpdateInfo {
    let tag = text_field(body, "tag_name").unwrap_or_default();
    let latest = normalise_version(&tag);
    let asset = pick_installer(body);
    UpdateInfo {
        current_version: current.to_string(),
        latest_version: if latest.is_empty() {
            tag.clone()
        } else {
            latest.clone()
        },
        has_update: is_newer(current, &latest),
        release_name: text_field(body, "name")
            .or_else(|| Some(tag).filter(|value| !value.is_empty())),
        notes: text_field(body, "body"),
        published_at: text_field(body, "published_at"),
        release_url: text_field(body, "html_url"),
        asset_name: asset.as_ref().map(|item| item.name.clone()),
        asset_size: asset.as_ref().map(|item| item.size),
        download_url: asset.as_ref().map(|item| item.url.clone()),
        mirror_url: asset.as_ref().and_then(|item| item.mirror.clone()),
        sha256: None,
        checked_at: db::now_ms(),
    }
}

struct ReleaseAsset {
    name: String,
    size: u64,
    url: String,
    mirror: Option<String>,
}

fn pick_installer(body: &serde_json::Value) -> Option<ReleaseAsset> {
    let assets: Vec<ReleaseAsset> = body
        .get("assets")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(ReleaseAsset {
                        name: text_field(item, "name")?,
                        size: item
                            .get("size")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0),
                        url: text_field(item, "browser_download_url")?,
                        mirror: text_field(item, "url"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let is_exe = |asset: &ReleaseAsset| {
        let name = asset.name.to_ascii_lowercase();
        name.ends_with(".exe") && !name.ends_with(".sig") && !name.contains("uninstall")
    };
    assets
        .iter()
        .find(|asset| is_exe(asset) && asset.name.to_ascii_lowercase().contains("setup"))
        .or_else(|| assets.iter().find(|asset| is_exe(asset)))
        .map(|asset| ReleaseAsset {
            name: asset.name.clone(),
            size: asset.size,
            url: asset.url.clone(),
            mirror: asset.mirror.clone(),
        })
}

fn checksum_asset_url(body: &serde_json::Value, asset_name: &str) -> Option<String> {
    let wanted = format!("{asset_name}.sha256");
    body.get("assets")
        .and_then(|value| value.as_array())
        .and_then(|items| {
            items.iter().find(|item| {
                text_field(item, "name")
                    .map(|name| name.eq_ignore_ascii_case(&wanted))
                    .unwrap_or(false)
            })
        })
        .and_then(|item| text_field(item, "browser_download_url"))
}

async fn fetch_checksum(client: &reqwest::Client, url: &str) -> Result<Option<String>, String> {
    if validate_download_url(url).is_err() {
        return Ok(None);
    }
    let text = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_digest(&text))
}

fn parse_digest(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '*')
        .find(|token| token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|token| token.to_ascii_lowercase())
}

fn text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn normalise_version(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).trim().to_string()
}

/// Parses `1.2.3`, `v1.2`, `1.2.3-rc.1`; anything else is treated as unknown.
fn parse_version(raw: &str) -> Option<semver::Version> {
    let cleaned = normalise_version(raw);
    if let Ok(version) = semver::Version::parse(&cleaned) {
        return Some(version);
    }
    let split_at = cleaned.find(['-', '+']).unwrap_or(cleaned.len());
    let (core, suffix) = cleaned.split_at(split_at);
    let mut numbers: Vec<&str> = core.split('.').collect();
    if numbers.is_empty() || numbers.len() > 3 {
        return None;
    }
    while numbers.len() < 3 {
        numbers.push("0");
    }
    semver::Version::parse(&format!("{}{suffix}", numbers.join("."))).ok()
}

/// Unknown versions never trigger an update, so a malformed tag cannot loop the check.
fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// Flattens the reqwest error chain so the UI can show the real cause.
fn describe_error(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(&format!("（{cause}）"));
        source = cause.source();
    }
    message
}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(format!(
            "{USER_AGENT}/{} (+https://github.com/{REPO})",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(10))
}

/// Client for short API calls.
fn http_client() -> Result<reqwest::Client, String> {
    client_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("初始化网络客户端失败: {e}"))
}

/// Client for the installer download: no overall timeout, only per-read stalls fail.
fn download_client() -> Result<reqwest::Client, String> {
    client_builder()
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("初始化网络客户端失败: {e}"))
}

/// Returns the lowercased host of an HTTPS URL, ignoring any userinfo and port.
fn https_host(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let host = authority.split(':').next()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn validate_download_url(url: &str) -> Result<(), String> {
    let host = https_host(url).ok_or_else(|| "更新地址必须是 HTTPS".to_string())?;
    let allowed = DOWNLOAD_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")));
    if !allowed {
        return Err("更新地址不在允许的 GitHub 域名内".to_string());
    }
    Ok(())
}

fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or("update");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').trim().to_string();
    if cleaned.to_ascii_lowercase().ends_with(".exe") {
        cleaned
    } else {
        "LiteScriptManager-setup.exe".to_string()
    }
}

fn update_dir() -> PathBuf {
    std::env::temp_dir().join("LiteScriptManager-update")
}

/// Opens a download stream, trying every candidate endpoint before giving up.
async fn open_stream(
    client: &reqwest::Client,
    candidates: &[String],
) -> Result<reqwest::Response, String> {
    let mut errors: Vec<String> = Vec::new();
    for url in candidates {
        if validate_download_url(url).is_err() {
            errors.push(format!("{url}（域名不在允许范围内）"));
            continue;
        }
        let is_asset_api =
            url.starts_with("https://api.github.com/") && url.contains("/releases/assets/");
        for attempt in 0..2 {
            let mut request = client.get(url);
            if is_asset_api {
                request = request.header(reqwest::header::ACCEPT, "application/octet-stream");
            }
            match request.send().await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    errors.push(format!("HTTP {}", response.status().as_u16()));
                    break;
                }
                Err(error) => {
                    errors.push(describe_error(&error));
                    if attempt == 0 {
                        tokio::time::sleep(Duration::from_millis(700)).await;
                    }
                }
            }
        }
    }
    Err(format!("下载更新失败：{}", errors.join("；")))
}

/// Streams the installer to disk and verifies it looks like a Windows executable.
async fn download_to_file(
    candidates: &[String],
    target: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(u64, String), String> {
    let client = download_client()?;
    let mut response = open_stream(&client, candidates).await?;
    let total = response.content_length().filter(|value| *value > 0);
    if total.unwrap_or(0) > MAX_INSTALLER_BYTES {
        return Err("更新包体积异常，已停止下载".to_string());
    }

    let mut file = std::fs::File::create(target).map_err(|e| format!("写入更新包失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut reported: u64 = 0;
    let mut head: Vec<u8> = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(target);
                return Err(format!("下载中断: {}", describe_error(&error)));
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if head.len() < 2 {
            head.extend_from_slice(&chunk[..chunk.len().min(2 - head.len())]);
        }
        if let Err(error) = file.write_all(&chunk) {
            drop(file);
            let _ = std::fs::remove_file(target);
            return Err(format!("写入更新包失败: {error}"));
        }
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if downloaded > MAX_INSTALLER_BYTES {
            drop(file);
            let _ = std::fs::remove_file(target);
            return Err("更新包体积异常，已停止下载".to_string());
        }
        if downloaded - reported >= PROGRESS_STEP_BYTES || Some(downloaded) == total {
            reported = downloaded;
            on_progress(downloaded, total);
        }
    }
    file.flush().map_err(|e| format!("写入更新包失败: {e}"))?;
    drop(file);

    if downloaded == 0 {
        let _ = std::fs::remove_file(target);
        return Err("下载的更新包为空".to_string());
    }
    if let Some(total) = total
        && downloaded != total
    {
        let _ = std::fs::remove_file(target);
        return Err(format!("下载不完整：{downloaded}/{total} 字节"));
    }
    if head.as_slice() != b"MZ" {
        let _ = std::fs::remove_file(target);
        return Err("下载的文件不是有效的 Windows 安装程序".to_string());
    }
    Ok((downloaded, hex_digest(&hasher.finalize())))
}

/// Downloads the installer announced by the last check, verifies it and hands it to NSIS.
pub async fn install_latest(app: AppHandle) -> Result<(), String> {
    let info = last_result().ok_or_else(|| "请先检查更新".to_string())?;
    if !info.has_update {
        return Err("当前已是最新版本".to_string());
    }
    let url = info
        .download_url
        .ok_or_else(|| "该版本没有提供安装包".to_string())?;
    let file_name = info
        .asset_name
        .ok_or_else(|| "该版本没有提供安装包".to_string())?;
    let expected_sha256 = info.sha256;
    let mut candidates = vec![url];
    if let Some(mirror) = info.mirror_url.filter(|value| !value.trim().is_empty()) {
        candidates.push(mirror);
    }
    if let Some(first) = candidates.first() {
        validate_download_url(first)?;
    }
    let dir = update_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建更新目录失败: {e}"))?;
    let target = dir.join(safe_file_name(&file_name));
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }

    let emitter = app.clone();
    let (downloaded, digest) = download_to_file(&candidates, &target, |downloaded, total| {
        let _ = emitter.emit("update-progress", ProgressPayload { downloaded, total });
    })
    .await?;

    if let Some(expected) = expected_sha256.filter(|value| !value.trim().is_empty())
        && !digest.eq_ignore_ascii_case(expected.trim())
    {
        let _ = std::fs::remove_file(&target);
        return Err("更新包校验失败：SHA256 不匹配".to_string());
    }

    let _ = app.emit(
        "update-progress",
        ProgressPayload {
            downloaded,
            total: Some(downloaded),
        },
    );
    if let Err(error) = launch_installer(&target) {
        let _ = std::fs::remove_file(&target);
        return Err(error);
    }
    logging::app_log(
        "INFO",
        &format!("安装更新 {digest}，应用退出等待安装程序完成"),
    );
    // 先让命令成功返回，前端才能提示"正在安装"，随后再退出让安装程序替换文件。
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        crate::windows::exit_app(&handle);
    });
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(windows)]
fn launch_installer(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let file = wide(path.as_os_str().to_string_lossy().as_ref());
    let operation = wide("open");
    // Silent install, keep shortcuts, restart the application afterwards.
    let parameters = wide("/S /UPDATE /R");
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(format!("启动安装程序失败：错误码 {}", result as isize));
    }
    Ok(())
}

#[cfg(not(windows))]
fn launch_installer(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动安装程序失败: {e}"))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Opens the release page in the default browser; restricted to github.com itself.
pub fn open_release_page(url: &str) -> Result<(), String> {
    if https_host(url).as_deref() != Some("github.com") {
        return Err("只允许打开 GitHub 链接".to_string());
    }
    crate::spawn_opener("explorer.exe", url.trim())
}

#[cfg(test)]
mod tests {
    use super::{
        is_newer, normalise_version, parse_digest, parse_version, safe_file_name,
        validate_download_url,
    };

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v0.1.0", "v0.1.1"));
        assert!(!is_newer("0.2.0", "0.2"));
        assert!(is_newer("0.2", "0.2.1"));
        assert!(is_newer("0.2.0", "0.3.0-beta.1"));
    }

    #[test]
    fn unknown_versions_never_trigger_updates() {
        assert!(!is_newer("0.2.0", "nightly"));
        assert!(!is_newer("0.2.0", ""));
        assert_eq!(parse_version("1.2"), Some(semver::Version::new(1, 2, 0)));
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn strips_version_prefix() {
        assert_eq!(normalise_version("v1.2.3"), "1.2.3");
        assert_eq!(normalise_version(" 2.0.0 "), "2.0.0");
    }

    #[test]
    fn only_allows_github_download_hosts() {
        assert!(validate_download_url("https://github.com/a/b/releases/download/v1/x.exe").is_ok());
        assert!(validate_download_url("https://objects.githubusercontent.com/x").is_ok());
        assert!(validate_download_url("http://github.com/a/b").is_err());
        assert!(validate_download_url("https://evil.example.com/github.com/x.exe").is_err());
        assert!(validate_download_url("https://github.com.evil.example/x.exe").is_err());
    }

    #[test]
    fn sanitises_download_file_names() {
        assert_eq!(
            safe_file_name("LiteScriptManager_0.2.0_x64-setup.exe"),
            "LiteScriptManager_0.2.0_x64-setup.exe"
        );
        assert_eq!(
            safe_file_name("../../etc/passwd"),
            "LiteScriptManager-setup.exe"
        );
        assert_eq!(safe_file_name("weird name.exe"), "weird_name.exe");
    }

    #[test]
    fn release_page_must_be_github() {
        assert_eq!(
            super::https_host("https://Github.com/a?x=1"),
            Some("github.com".to_string())
        );
        assert_eq!(
            super::https_host("https://user:pw@github.com:443/a"),
            Some("github.com".to_string())
        );
        assert_eq!(super::https_host("http://github.com/a"), None);
        assert_eq!(
            super::https_host("https://github.com.evil.dev/a"),
            Some("github.com.evil.dev".to_string())
        );
    }

    #[test]
    fn reads_checksum_files() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_digest(&format!("{digest}  LiteScriptManager-setup.exe")),
            Some(digest.clone())
        );
        assert_eq!(parse_digest("not a checksum"), None);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn fetches_the_real_release() {
        let client = http_client().expect("client");
        let response = client
            .get(latest_release_api())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .expect("request");
        eprintln!("status: {}", response.status());
        let body: serde_json::Value = response.json().await.expect("json");
        let info = parse_release("0.1.0", &body);
        eprintln!("{info:#?}");
        println!("asset url: {:?}", info.download_url);
        validate_download_url(info.download_url.as_deref().unwrap()).expect("allowed host");
        let info_future = parse_release("0.0.9", &body);
        assert!(info_future.has_update);
        assert!(!info.has_update);
    }

    #[tokio::test]
    #[ignore]
    async fn downloads_the_real_installer() {
        let body: serde_json::Value = http_client()
            .expect("client")
            .get(latest_release_api())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .expect("request")
            .json()
            .await
            .expect("json");
        let info = parse_release("0.0.1", &body);
        let mut candidates = vec![info.download_url.clone().expect("download url")];
        candidates.extend(info.mirror_url.clone());
        let target = std::env::temp_dir().join("lsm-update-test.exe");
        let mut steps = 0;
        let (size, digest) = download_to_file(&candidates, &target, |downloaded, total| {
            steps += 1;
            if steps == 1 {
                eprintln!("first progress: {downloaded}/{total:?}");
            }
        })
        .await
        .expect("download");
        eprintln!("downloaded {size} bytes, sha256 {digest}, steps {steps}");
        assert!(size > 100_000);
        assert_eq!(info.asset_size.unwrap_or(0), size);
        assert_eq!(digest.len(), 64);
        std::fs::remove_file(&target).expect("cleanup");
    }
}
