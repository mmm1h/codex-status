use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, c_void};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::{PCWSTR, w};

const RELEASE_API: &str = "https://api.github.com/repos/mmm1h/codex-status/releases/latest";
const RELEASE_ASSET_PREFIX: &str = "https://github.com/mmm1h/codex-status/releases/download/";
const MAX_METADATA_BYTES: usize = 512 * 1024;
const MAX_EXECUTABLE_BYTES: usize = 32 * 1024 * 1024;
const UPDATE_WAIT_MS: u32 = 30_000;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("Windows update service failed: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("Update network response was invalid")]
    InvalidResponse,
    #[error("Update metadata could not be parsed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Update file could not be prepared: {0}")]
    Io(#[from] std::io::Error),
    #[error("Downloaded update did not match its GitHub digest")]
    DigestMismatch,
    #[error("Update helper did not receive a safe target")]
    UnsafeTarget,
    #[error("The running CodexStatus process did not exit in time")]
    ParentStillRunning,
}

#[derive(Debug, Clone)]
pub struct StagedUpdate {
    pub executable: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

struct InternetHandle(*mut c_void);

impl InternetHandle {
    fn new(value: *mut c_void) -> Result<Self, UpdateError> {
        if value.is_null() {
            Err(windows::core::Error::from_thread().into())
        } else {
            Ok(Self(value))
        }
    }
}

impl Drop for InternetHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

struct HttpClient {
    session: InternetHandle,
}

impl HttpClient {
    fn new() -> Result<Self, UpdateError> {
        let agent = wide0(format!("CodexStatus/{}", env!("CARGO_PKG_VERSION")));
        let session = unsafe {
            InternetHandle::new(WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ))?
        };
        unsafe {
            WinHttpSetTimeouts(session.0, 5_000, 5_000, 10_000, 15_000)?;
        }
        Ok(Self { session })
    }

    fn get(&self, url: &str, accept: &str, limit: usize) -> Result<Vec<u8>, UpdateError> {
        let (host, path) = split_https_url(url).ok_or(UpdateError::InvalidResponse)?;
        let host = wide0(host);
        let path = wide0(path);
        let connection = unsafe {
            InternetHandle::new(WinHttpConnect(
                self.session.0,
                PCWSTR(host.as_ptr()),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ))?
        };
        let request = unsafe {
            InternetHandle::new(WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            ))?
        };
        let headers: Vec<u16> = format!("Accept: {accept}\r\nX-GitHub-Api-Version: 2022-11-28\r\n")
            .encode_utf16()
            .collect();
        unsafe {
            WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)?;
            WinHttpReceiveResponse(request.0, ptr::null_mut())?;
        }

        let mut status = 0_u32;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        let mut index = 0_u32;
        unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&mut status as *mut u32).cast()),
                &mut status_size,
                &mut index,
            )?;
        }
        if status != 200 {
            return Err(UpdateError::InvalidResponse);
        }

        let mut body = Vec::new();
        loop {
            let mut available = 0_u32;
            unsafe {
                WinHttpQueryDataAvailable(request.0, &mut available)?;
            }
            if available == 0 {
                break;
            }
            let available = available as usize;
            if body.len().saturating_add(available) > limit {
                return Err(UpdateError::InvalidResponse);
            }
            let start = body.len();
            body.resize(start + available, 0);
            let mut read = 0_u32;
            unsafe {
                WinHttpReadData(
                    request.0,
                    body[start..].as_mut_ptr().cast(),
                    available as u32,
                    &mut read,
                )?;
            }
            body.truncate(start + read as usize);
            if read == 0 {
                break;
            }
        }
        Ok(body)
    }
}

pub fn check_and_stage(updates_directory: &Path) -> Result<Option<StagedUpdate>, UpdateError> {
    let Some(asset_channel) = update_asset_channel() else {
        return Ok(None);
    };
    let client = HttpClient::new()?;
    let metadata = client.get(RELEASE_API, "application/vnd.github+json", MAX_METADATA_BYTES)?;
    let Some((version, asset, digest)) =
        select_asset(&metadata, env!("CARGO_PKG_VERSION"), asset_channel)?
    else {
        return Ok(None);
    };
    let bytes = client.get(
        &asset.browser_download_url,
        "application/octet-stream",
        MAX_EXECUTABLE_BYTES,
    )?;
    if bytes.len() as u64 != asset.size || bytes.len() < 64 * 1024 || !bytes.starts_with(b"MZ") {
        return Err(UpdateError::InvalidResponse);
    }
    if sha256_hex(&bytes) != digest {
        return Err(UpdateError::DigestMismatch);
    }

    let version_directory = updates_directory.join(format!("v{version}"));
    fs::create_dir_all(&version_directory)?;
    let executable = version_directory.join("CodexStatus.exe");
    let temporary = version_directory.join("CodexStatus.download");
    fs::write(&temporary, bytes)?;
    if executable.exists() {
        fs::remove_file(&executable)?;
    }
    fs::rename(temporary, &executable)?;
    Ok(Some(StagedUpdate { executable }))
}

fn select_asset(
    metadata: &[u8],
    current_version: &str,
    asset_channel: UpdateAssetChannel,
) -> Result<Option<(String, ReleaseAsset, String)>, UpdateError> {
    let release: Release = serde_json::from_slice(metadata)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name);
    let Some(latest) = parse_version(version) else {
        return Err(UpdateError::InvalidResponse);
    };
    let Some(current) = parse_version(current_version) else {
        return Err(UpdateError::InvalidResponse);
    };
    if latest <= current {
        return Ok(None);
    }

    let expected_name = asset_channel.asset_name(version);
    let Some(asset) = release.assets.into_iter().find(|asset| asset.name == expected_name) else {
        return Err(UpdateError::InvalidResponse);
    };
    if !asset.browser_download_url.starts_with(RELEASE_ASSET_PREFIX)
        || asset.browser_download_url.contains("/../")
        || asset.size == 0
        || asset.size > MAX_EXECUTABLE_BYTES as u64
    {
        return Err(UpdateError::InvalidResponse);
    }
    let Some(digest) = asset.digest.as_deref().and_then(|value| value.strip_prefix("sha256:"))
    else {
        return Err(UpdateError::InvalidResponse);
    };
    let digest = digest.to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(UpdateError::InvalidResponse);
    }
    Ok(Some((version.to_owned(), asset, digest)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateAssetChannel {
    Installed,
    Portable,
}

impl UpdateAssetChannel {
    fn asset_name(self, version: &str) -> String {
        match self {
            Self::Installed => format!("CodexStatus-v{version}-windows-x64.exe"),
            Self::Portable => format!("CodexStatus-v{version}-windows-x64-portable.exe"),
        }
    }
}

fn update_asset_channel() -> Option<UpdateAssetChannel> {
    update_asset_channel_for(env!("CODEX_STATUS_CHANNEL"))
}

pub fn updates_supported() -> bool {
    update_asset_channel().is_some()
}

fn update_asset_channel_for(channel: &str) -> Option<UpdateAssetChannel> {
    match channel {
        "stable" => Some(UpdateAssetChannel::Installed),
        "portable" => Some(UpdateAssetChannel::Portable),
        "beta" | "development" => None,
        _ => None,
    }
}

pub fn launch_staged_update(update: &StagedUpdate) -> Result<(), UpdateError> {
    let target = std::env::current_exe()?;
    Command::new(&update.executable)
        .arg("--apply-update")
        .arg(std::process::id().to_string())
        .arg(target)
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    Ok(())
}

pub fn apply_update_silently(parent_pid: u32, target: &Path) {
    let result = apply_update(parent_pid, target);
    if result.is_err() && target.is_file() {
        let _ = launch_target(target);
    }
}

fn apply_update(parent_pid: u32, target: &Path) -> Result<(), UpdateError> {
    validate_target(target)?;
    if let Ok(process) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) } {
        let waited = unsafe { WaitForSingleObject(process, UPDATE_WAIT_MS) };
        unsafe {
            let _ = CloseHandle(process);
        }
        if waited != WAIT_OBJECT_0 {
            return Err(UpdateError::ParentStillRunning);
        }
    }

    let staged = std::env::current_exe()?;
    if staged == target {
        return Err(UpdateError::UnsafeTarget);
    }
    let pending = target.with_extension("update");
    fs::copy(staged, &pending)?;
    let pending_wide = wide0(pending.as_os_str());
    let target_wide = wide0(target.as_os_str());
    unsafe {
        MoveFileExW(
            PCWSTR(pending_wide.as_ptr()),
            PCWSTR(target_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
    }
    launch_target(target)?;
    Ok(())
}

fn launch_target(target: &Path) -> Result<(), std::io::Error> {
    Command::new(target).arg("--background").creation_flags(CREATE_NO_WINDOW.0).spawn().map(|_| ())
}

fn validate_target(target: &Path) -> Result<(), UpdateError> {
    if !target.is_absolute() {
        return Err(UpdateError::UnsafeTarget);
    }
    let name = target.file_name().and_then(OsStr::to_str).unwrap_or_default().to_ascii_lowercase();
    if !matches!(name.as_str(), "codexstatus.exe" | "codex-status.exe") {
        return Err(UpdateError::UnsafeTarget);
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn split_https_url(url: &str) -> Option<(&str, &str)> {
    let remainder = url.strip_prefix("https://")?;
    let slash = remainder.find('/')?;
    let host = &remainder[..slash];
    let path = &remainder[slash..];
    if host.is_empty() || host.contains([':', '@', '\\']) || !path.starts_with('/') {
        return None;
    }
    Some((host, path))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn wide0(value: impl AsRef<OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metadata(tag: &str, digest: &str, asset_channel: UpdateAssetChannel) -> Vec<u8> {
        let version = tag.strip_prefix('v').unwrap_or(tag);
        serde_json::to_vec(&json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "assets": [{
                "name": asset_channel.asset_name(version),
                "browser_download_url": format!(
                    "https://github.com/mmm1h/codex-status/releases/download/{tag}/{}",
                    asset_channel.asset_name(version)
                ),
                "size": 100_000,
                "digest": format!("sha256:{digest}")
            }]
        }))
        .unwrap()
    }

    #[test]
    fn selects_only_a_newer_stable_release_with_a_digest() {
        let digest = "a".repeat(64);
        let selected = select_asset(
            &metadata("v0.2.0", &digest, UpdateAssetChannel::Portable),
            "0.1.2",
            UpdateAssetChannel::Portable,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.0, "0.2.0");
        assert_eq!(selected.2, digest);
        assert!(
            select_asset(
                &metadata("v0.2.0", &"b".repeat(64), UpdateAssetChannel::Portable),
                "0.2.0",
                UpdateAssetChannel::Portable,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn rejects_missing_or_malformed_digest() {
        assert!(
            select_asset(
                &metadata("v0.2.0", "short", UpdateAssetChannel::Portable),
                "0.1.2",
                UpdateAssetChannel::Portable,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_assets_that_violate_download_safety_boundaries() {
        let digest = "a".repeat(64);
        let base: serde_json::Value =
            serde_json::from_slice(&metadata("v0.2.0", &digest, UpdateAssetChannel::Portable))
                .unwrap();

        let mut wrong_name = base.clone();
        wrong_name["assets"][0]["name"] = json!("CodexStatus.exe");
        assert!(
            select_asset(
                &serde_json::to_vec(&wrong_name).unwrap(),
                "0.1.2",
                UpdateAssetChannel::Portable,
            )
            .is_err()
        );

        let mut wrong_url = base.clone();
        wrong_url["assets"][0]["browser_download_url"] =
            json!("https://example.com/CodexStatus-v0.2.0-windows-x64.exe");
        assert!(
            select_asset(
                &serde_json::to_vec(&wrong_url).unwrap(),
                "0.1.2",
                UpdateAssetChannel::Portable,
            )
            .is_err()
        );

        let mut traversal_url = base.clone();
        traversal_url["assets"][0]["browser_download_url"] = json!(
            "https://github.com/mmm1h/codex-status/releases/download/v0.2.0/../CodexStatus.exe"
        );
        assert!(
            select_asset(
                &serde_json::to_vec(&traversal_url).unwrap(),
                "0.1.2",
                UpdateAssetChannel::Portable,
            )
            .is_err()
        );

        let mut oversized = base;
        oversized["assets"][0]["size"] = json!(MAX_EXECUTABLE_BYTES as u64 + 1);
        assert!(
            select_asset(
                &serde_json::to_vec(&oversized).unwrap(),
                "0.1.2",
                UpdateAssetChannel::Portable,
            )
            .is_err()
        );
    }

    #[test]
    fn installed_and_portable_channels_select_different_assets() {
        assert_eq!(
            UpdateAssetChannel::Installed.asset_name("0.7.0"),
            "CodexStatus-v0.7.0-windows-x64.exe"
        );
        assert_eq!(
            UpdateAssetChannel::Portable.asset_name("0.7.0"),
            "CodexStatus-v0.7.0-windows-x64-portable.exe"
        );
    }

    #[test]
    fn development_and_beta_builds_never_install_stable_assets() {
        assert_eq!(update_asset_channel_for("stable"), Some(UpdateAssetChannel::Installed));
        assert_eq!(update_asset_channel_for("portable"), Some(UpdateAssetChannel::Portable));
        assert_eq!(update_asset_channel_for("development"), None);
        assert_eq!(update_asset_channel_for("beta"), None);
    }

    #[test]
    fn v0_6_1_installed_client_selects_the_stable_asset_from_a_new_release() {
        let digest = "a".repeat(64);
        let release = serde_json::to_vec(&json!({
            "tag_name": "v0.7.0",
            "draft": false,
            "prerelease": false,
            "assets": [
                {
                    "name": "CodexStatus-v0.7.0-windows-x64-portable.exe",
                    "browser_download_url": "https://github.com/mmm1h/codex-status/releases/download/v0.7.0/CodexStatus-v0.7.0-windows-x64-portable.exe",
                    "size": 100_000,
                    "digest": format!("sha256:{digest}")
                },
                {
                    "name": "CodexStatus-v0.7.0-windows-x64.exe",
                    "browser_download_url": "https://github.com/mmm1h/codex-status/releases/download/v0.7.0/CodexStatus-v0.7.0-windows-x64.exe",
                    "size": 100_000,
                    "digest": format!("sha256:{digest}")
                }
            ]
        }))
        .unwrap();

        let (_, asset, _) =
            select_asset(&release, "0.6.1", UpdateAssetChannel::Installed).unwrap().unwrap();

        assert_eq!(asset.name, "CodexStatus-v0.7.0-windows-x64.exe");
    }

    #[test]
    fn parses_only_plain_three_part_versions() {
        assert_eq!(parse_version("10.2.31"), Some((10, 2, 31)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3-beta"), None);
    }

    #[test]
    fn splits_only_simple_https_urls() {
        assert_eq!(
            split_https_url("https://api.github.com/repos/mmm1h/codex-status"),
            Some(("api.github.com", "/repos/mmm1h/codex-status"))
        );
        assert!(split_https_url("http://api.github.com/test").is_none());
        assert!(split_https_url("https://user@api.github.com/test").is_none());
    }

    #[test]
    fn hashes_bytes_as_lowercase_sha256() {
        assert_eq!(
            sha256_hex(b"CodexStatus"),
            "1348bd7daee4282c641059f8cdd9fe96ae24f501c0cd32fbdabb8c1e60eea85c"
        );
    }

    #[test]
    #[ignore = "requires access to the public GitHub API"]
    fn live_github_release_metadata_is_readable_with_winhttp() {
        let client = HttpClient::new().unwrap();
        let bytes =
            client.get(RELEASE_API, "application/vnd.github+json", MAX_METADATA_BYTES).unwrap();
        let release: Release = serde_json::from_slice(&bytes).unwrap();
        assert!(release.tag_name.starts_with('v'));
        assert!(!release.draft);
    }
}
