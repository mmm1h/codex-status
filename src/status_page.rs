//! Lightweight, on-demand access to OpenAI's public service status.
//!
//! This module deliberately owns no timer or background worker. Callers decide
//! when to refresh and can use [`StatusPageError`] to apply their own backoff.

use serde::Deserialize;
use std::ffi::{OsStr, c_void};
use std::ptr;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_DISABLE_AUTHENTICATION, WINHTTP_DISABLE_COOKIES, WINHTTP_DISABLE_REDIRECTS,
    WINHTTP_FLAG_SECURE, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

/// The public page a user can open for details.
pub const STATUS_PAGE_HOME: &str = "https://status.openai.com/";

/// OpenAI's public, credential-free status summary endpoint.
pub const STATUS_SUMMARY_URL: &str = "https://status.openai.com/api/v2/summary.json";

const STATUS_HOST: &str = "status.openai.com";
const STATUS_PATH: &str = "/api/v2/summary.json";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_SUMMARY_CHARS: usize = 180;
const MAX_TIMESTAMP_CHARS: usize = 64;

/// A compact status suitable for a tray badge or menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Operational,
    Degraded,
    Outage,
    Unknown,
}

impl ServiceStatus {
    fn severity(self) -> u8 {
        match self {
            Self::Operational => 0,
            Self::Unknown => 1,
            Self::Degraded => 2,
            Self::Outage => 3,
        }
    }

    fn worst(self, other: Self) -> Self {
        if other.severity() > self.severity() { other } else { self }
    }
}

/// One non-sensitive status snapshot returned by the official public page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatusSnapshot {
    /// The worst of the overall OpenAI status and any explicit Codex status.
    pub status: ServiceStatus,
    /// A short official description, incident title, or affected component list.
    pub summary: String,
    /// ISO-8601 timestamp supplied by the status page, when available.
    pub updated_at: Option<String>,
    /// True only when the payload explicitly identifies a Codex surface as affected.
    pub codex_affected: bool,
}

impl ServiceStatusSnapshot {
    /// A safe value for callers to display while backing off after an error.
    pub fn unavailable() -> Self {
        Self {
            status: ServiceStatus::Unknown,
            summary: "OpenAI status unavailable".to_owned(),
            updated_at: None,
            codex_affected: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StatusPageError {
    #[error("OpenAI status request failed")]
    Windows(#[from] windows::core::Error),
    #[error("OpenAI status endpoint was not allowed")]
    EndpointNotAllowed,
    #[error("OpenAI status endpoint returned HTTP {0}")]
    HttpStatus(u32),
    #[error("OpenAI status response exceeded {MAX_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("OpenAI status response was invalid")]
    InvalidResponse(#[from] serde_json::Error),
}

struct InternetHandle(*mut c_void);

impl InternetHandle {
    fn new(value: *mut c_void) -> Result<Self, StatusPageError> {
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

/// Fetches one snapshot and then releases every WinHTTP handle.
///
/// No credentials, cookies, response bodies, or logs are persisted. On error,
/// callers can display [`ServiceStatusSnapshot::unavailable`] and retry later.
pub fn fetch_service_status() -> Result<ServiceStatusSnapshot, StatusPageError> {
    let bytes = fetch_summary_bytes(STATUS_SUMMARY_URL)?;
    parse_status_response(&bytes)
}

/// Parses an OpenAI status summary while enforcing the same limit as the network path.
///
/// This is public to make deterministic integration tests possible without network access.
pub fn parse_status_response(bytes: &[u8]) -> Result<ServiceStatusSnapshot, StatusPageError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(StatusPageError::ResponseTooLarge);
    }

    let response: StatusResponse = serde_json::from_slice(bytes)?;
    Ok(summarize(response))
}

fn fetch_summary_bytes(url: &str) -> Result<Vec<u8>, StatusPageError> {
    let (host, path) = split_allowed_endpoint(url).ok_or(StatusPageError::EndpointNotAllowed)?;
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
        // Short, bounded calls allow the application to back off without a resident runtime.
        WinHttpSetTimeouts(session.0, 4_000, 4_000, 5_000, 8_000)?;
    }

    let host = wide0(host);
    let path = wide0(path);
    let connection = unsafe {
        InternetHandle::new(WinHttpConnect(
            session.0,
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

    // WinHTTP redirects are disabled so a valid endpoint cannot escape the host allowlist.
    let disabled =
        WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
    unsafe {
        WinHttpSetOption(
            Some(request.0.cast_const()),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&disabled.to_ne_bytes()),
        )?;
    }

    let headers: Vec<u16> =
        "Accept: application/json\r\nCache-Control: no-cache\r\n".encode_utf16().collect();
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
        return Err(StatusPageError::HttpStatus(status));
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
        if body.len().saturating_add(available) > MAX_RESPONSE_BYTES {
            return Err(StatusPageError::ResponseTooLarge);
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

fn split_allowed_endpoint(url: &str) -> Option<(&str, &str)> {
    let remainder = url.strip_prefix("https://")?;
    let slash = remainder.find('/')?;
    let host = &remainder[..slash];
    let path = &remainder[slash..];
    if host.eq_ignore_ascii_case(STATUS_HOST) && path == STATUS_PATH {
        Some((STATUS_HOST, STATUS_PATH))
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    #[serde(default)]
    page: Page,
    #[serde(default)]
    status: PageStatus,
    #[serde(default)]
    components: Vec<Component>,
    #[serde(default)]
    incidents: Vec<Incident>,
}

#[derive(Debug, Default, Deserialize)]
struct Page {
    #[serde(default)]
    updated_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct PageStatus {
    #[serde(default)]
    description: String,
    #[serde(default)]
    indicator: String,
}

#[derive(Debug, Default, Deserialize)]
struct Component {
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    updated_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct Incident {
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    impact: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    components: Vec<Component>,
    #[serde(default)]
    incident_updates: Vec<IncidentUpdate>,
}

#[derive(Debug, Default, Deserialize)]
struct IncidentUpdate {
    #[serde(default)]
    body: String,
}

fn summarize(response: StatusResponse) -> ServiceStatusSnapshot {
    let page_status = status_from_indicator(&response.status.indicator);
    let mut global_status = page_status;
    let mut codex_status = ServiceStatus::Operational;
    let mut codex_summary: Option<(ServiceStatus, String)> = None;
    let mut global_summary: Option<(ServiceStatus, String)> = None;
    let mut affected_components = Vec::new();
    let mut updated_at = compact_timestamp(&response.page.updated_at);

    for component in &response.components {
        update_latest(&mut updated_at, &component.updated_at);
        if !is_codex_component(&component.name) {
            continue;
        }
        let status = status_from_component(&component.status);
        codex_status = codex_status.worst(status);
        if matches!(status, ServiceStatus::Degraded | ServiceStatus::Outage) {
            affected_components.push(compact_text(&component.name, MAX_SUMMARY_CHARS));
        }
    }

    for incident in &response.incidents {
        update_latest(&mut updated_at, &incident.updated_at);
        if incident_is_resolved(&incident.status) {
            continue;
        }

        let incident_status = status_from_impact(&incident.impact);
        global_status = global_status.worst(incident_status);
        if matches!(incident_status, ServiceStatus::Degraded | ServiceStatus::Outage) {
            choose_summary(&mut global_summary, incident_status, &incident.name);
        }

        let explicitly_codex = incident_mentions_codex(incident);
        if explicitly_codex {
            codex_status = codex_status.worst(incident_status);
            choose_summary(&mut codex_summary, incident_status, &incident.name);
        }
    }

    let codex_affected = matches!(codex_status, ServiceStatus::Degraded | ServiceStatus::Outage);
    let status = global_status.worst(codex_status);
    let summary = codex_summary
        .filter(|(summary_status, _)| summary_status.severity() >= codex_status.severity())
        .map(|(_, summary)| summary)
        .or_else(|| {
            if affected_components.is_empty() {
                None
            } else {
                Some(compact_text(
                    &format!("Affected: {}", affected_components.join(", ")),
                    MAX_SUMMARY_CHARS,
                ))
            }
        })
        .or_else(|| {
            global_summary
                .filter(|(summary_status, _)| summary_status.severity() >= page_status.severity())
                .map(|(_, summary)| summary)
        })
        .or_else(|| {
            (status == page_status)
                .then(|| nonempty_compact(&response.status.description, MAX_SUMMARY_CHARS))
                .flatten()
        })
        .unwrap_or_else(|| match status {
            ServiceStatus::Operational => "All systems operational".to_owned(),
            ServiceStatus::Degraded => "OpenAI service is degraded".to_owned(),
            ServiceStatus::Outage => "OpenAI service outage".to_owned(),
            ServiceStatus::Unknown => "OpenAI status unknown".to_owned(),
        });

    ServiceStatusSnapshot { status, summary, updated_at, codex_affected }
}

fn status_from_indicator(value: &str) -> ServiceStatus {
    match normalized(value).as_str() {
        "none" | "operational" => ServiceStatus::Operational,
        "minor" | "maintenance" | "under_maintenance" => ServiceStatus::Degraded,
        "major" | "critical" => ServiceStatus::Outage,
        _ => ServiceStatus::Unknown,
    }
}

fn choose_summary(
    current: &mut Option<(ServiceStatus, String)>,
    status: ServiceStatus,
    text: &str,
) {
    let Some(text) = nonempty_compact(text, MAX_SUMMARY_CHARS) else {
        return;
    };
    if current
        .as_ref()
        .is_none_or(|(current_status, _)| status.severity() > current_status.severity())
    {
        *current = Some((status, text));
    }
}

fn status_from_component(value: &str) -> ServiceStatus {
    match normalized(value).as_str() {
        "operational" => ServiceStatus::Operational,
        "degraded_performance" | "partial_outage" | "under_maintenance" | "maintenance" => {
            ServiceStatus::Degraded
        }
        "major_outage" => ServiceStatus::Outage,
        _ => ServiceStatus::Unknown,
    }
}

fn status_from_impact(value: &str) -> ServiceStatus {
    match normalized(value).as_str() {
        "none" => ServiceStatus::Operational,
        "minor" | "maintenance" => ServiceStatus::Degraded,
        "major" | "critical" => ServiceStatus::Outage,
        _ => ServiceStatus::Unknown,
    }
}

fn incident_is_resolved(value: &str) -> bool {
    matches!(normalized(value).as_str(), "resolved" | "completed" | "postmortem")
}

fn incident_mentions_codex(incident: &Incident) -> bool {
    is_codex_text(&incident.name)
        || incident.components.iter().any(|component| is_codex_component(&component.name))
        || incident.incident_updates.iter().any(|update| is_codex_text(&update.body))
}

fn is_codex_component(name: &str) -> bool {
    let name = searchable_text(name);
    has_word(&name, "codex") || name == "cli" || name.contains("vs code extension")
}

fn is_codex_text(text: &str) -> bool {
    let text = searchable_text(text);
    has_word(&text, "codex") || has_word(&text, "cli") || text.contains("vs code extension")
}

fn normalized(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn searchable_text(value: &str) -> String {
    let words =
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { ' ' }
            })
            .collect::<String>();
    words.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_word(text: &str, needle: &str) -> bool {
    text.split_whitespace().any(|word| word == needle)
}

fn compact_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut output: String = compact.chars().take(max_chars.saturating_sub(1)).collect();
    output.push('\u{2026}');
    output
}

fn nonempty_compact(value: &str, max_chars: usize) -> Option<String> {
    let value = compact_text(value, max_chars);
    (!value.is_empty()).then_some(value)
}

fn compact_timestamp(value: &str) -> Option<String> {
    nonempty_compact(value, MAX_TIMESTAMP_CHARS)
}

fn update_latest(current: &mut Option<String>, candidate: &str) {
    let Some(candidate) = compact_timestamp(candidate) else {
        return;
    };
    if current.as_ref().is_none_or(|current| candidate > *current) {
        *current = Some(candidate);
    }
}

fn wide0(value: impl AsRef<OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(indicator: &str, description: &str) -> serde_json::Value {
        json!({
            "page": { "updated_at": "2026-07-27T05:00:00Z" },
            "status": { "indicator": indicator, "description": description },
            "components": [],
            "incidents": []
        })
    }

    fn parse(value: serde_json::Value) -> ServiceStatusSnapshot {
        parse_status_response(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn parses_an_operational_summary() {
        let snapshot = parse(response("none", "All Systems Operational"));
        assert_eq!(snapshot.status, ServiceStatus::Operational);
        assert_eq!(snapshot.summary, "All Systems Operational");
        assert_eq!(snapshot.updated_at.as_deref(), Some("2026-07-27T05:00:00Z"));
        assert!(!snapshot.codex_affected);
    }

    #[test]
    fn detects_a_degraded_codex_component() {
        let mut value = response("none", "All Systems Operational");
        value["components"] = json!([
            { "name": "Images", "status": "operational" },
            {
                "name": "Codex API",
                "status": "degraded_performance",
                "updated_at": "2026-07-27T05:05:00Z"
            }
        ]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Degraded);
        assert!(snapshot.codex_affected);
        assert!(snapshot.summary.contains("Codex API"));
        assert_eq!(snapshot.updated_at.as_deref(), Some("2026-07-27T05:05:00Z"));
    }

    #[test]
    fn detects_a_codex_incident_from_its_title() {
        let mut value = response("minor", "Minor Service Outage");
        value["incidents"] = json!([{
            "name": "Codex requests are failing",
            "status": "investigating",
            "impact": "major",
            "updated_at": "2026-07-27T06:00:00Z"
        }]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Outage);
        assert!(snapshot.codex_affected);
        assert_eq!(snapshot.summary, "Codex requests are failing");
    }

    #[test]
    fn detects_codex_from_incident_components_or_updates() {
        let mut value = response("minor", "Minor Service Outage");
        value["incidents"] = json!([{
            "name": "Elevated error rates",
            "status": "identified",
            "impact": "minor",
            "components": [{ "name": "CLI", "status": "partial_outage" }],
            "incident_updates": [{ "body": "Codex recovery is in progress." }]
        }]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Degraded);
        assert!(snapshot.codex_affected);
        assert_eq!(snapshot.summary, "Elevated error rates");
    }

    #[test]
    fn recognizes_all_current_codex_component_names_without_false_cli_matches() {
        for name in
            ["Codex in ChatGPT Desktop", "Codex API", "Codex Web", "VS Code extension", "CLI"]
        {
            assert!(is_codex_component(name), "missed {name}");
        }
        assert!(!is_codex_component("Client applications"));
        assert!(!is_codex_text("Client errors increased"));
    }

    #[test]
    fn recognizes_a_global_outage_without_claiming_codex_is_explicitly_affected() {
        let mut value = response("critical", "Major OpenAI outage");
        value["incidents"] = json!([{
            "name": "Image generation unavailable",
            "status": "investigating",
            "impact": "critical"
        }]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Outage);
        assert!(!snapshot.codex_affected);
        assert_eq!(snapshot.summary, "Image generation unavailable");
    }

    #[test]
    fn summary_tracks_the_highest_severity_instead_of_the_first_incident() {
        let mut value = response("critical", "Major OpenAI outage");
        value["incidents"] = json!([
            {
                "name": "Minor image delays",
                "status": "monitoring",
                "impact": "minor"
            },
            {
                "name": "Login unavailable",
                "status": "investigating",
                "impact": "critical"
            }
        ]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Outage);
        assert_eq!(snapshot.summary, "Login unavailable");
    }

    #[test]
    fn ignores_resolved_codex_incidents() {
        let mut value = response("none", "All Systems Operational");
        value["incidents"] = json!([{
            "name": "Codex outage resolved",
            "status": "resolved",
            "impact": "critical"
        }]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Operational);
        assert!(!snapshot.codex_affected);
    }

    #[test]
    fn returns_unknown_for_an_unrecognized_status_schema() {
        let snapshot = parse(response("brand_new_indicator", "Status is changing"));
        assert_eq!(snapshot.status, ServiceStatus::Unknown);
        assert!(!snapshot.codex_affected);
    }

    #[test]
    fn an_unknown_codex_component_does_not_report_all_systems_operational() {
        let mut value = response("none", "All Systems Operational");
        value["components"] = json!([{
            "name": "Codex API",
            "status": "brand_new_component_status"
        }]);
        let snapshot = parse(value);
        assert_eq!(snapshot.status, ServiceStatus::Unknown);
        assert_eq!(snapshot.summary, "OpenAI status unknown");
        assert!(!snapshot.codex_affected);
    }

    #[test]
    fn rejects_invalid_and_oversized_responses() {
        assert!(matches!(
            parse_status_response(b"not json"),
            Err(StatusPageError::InvalidResponse(_))
        ));
        let oversized = vec![b' '; MAX_RESPONSE_BYTES + 1];
        assert!(matches!(
            parse_status_response(&oversized),
            Err(StatusPageError::ResponseTooLarge)
        ));
    }

    #[test]
    fn compacts_and_limits_untrusted_summary_text() {
        let mut value = response("minor", "ignored");
        value["incidents"] = json!([{
            "name": format!("Codex\n{}", "x".repeat(MAX_SUMMARY_CHARS * 2)),
            "status": "investigating",
            "impact": "minor"
        }]);
        let snapshot = parse(value);
        assert!(!snapshot.summary.contains('\n'));
        assert_eq!(snapshot.summary.chars().count(), MAX_SUMMARY_CHARS);
        assert!(snapshot.summary.ends_with('\u{2026}'));
    }

    #[test]
    fn endpoint_allowlist_is_exact_and_https_only() {
        assert_eq!(split_allowed_endpoint(STATUS_SUMMARY_URL), Some((STATUS_HOST, STATUS_PATH)));
        assert_eq!(
            split_allowed_endpoint("https://STATUS.OPENAI.COM/api/v2/summary.json"),
            Some((STATUS_HOST, STATUS_PATH))
        );
        assert!(split_allowed_endpoint("http://status.openai.com/api/v2/summary.json").is_none());
        assert!(split_allowed_endpoint("https://evil.example/api/v2/summary.json").is_none());
        assert!(
            split_allowed_endpoint("https://status.openai.com.evil.example/api/v2/summary.json")
                .is_none()
        );
        assert!(
            split_allowed_endpoint("https://status.openai.com/api/v2/summary.json?next=evil")
                .is_none()
        );
    }
}
