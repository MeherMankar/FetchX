use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_DIAGNOSTIC_EVENT_LIMIT: usize = 128;
const MAX_EVENT_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticHealth {
    Healthy,
    Degraded,
    Unhealthy,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticComponent {
    Application,
    Core,
    Queue,
    Downloader,
    Storage,
    Rpc,
    Plugins,
    Android,
    Cli,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticEvent {
    pub timestamp_unix_ms: u64,
    pub component: DiagnosticComponent,
    pub event: String,
    pub level: DiagnosticLevel,
    pub request_id: Option<String>,
    pub download_id: Option<String>,
    pub plugin_id: Option<String>,
    pub operation_id: Option<String>,
    pub error_code: Option<String>,
}

impl DiagnosticEvent {
    #[must_use]
    pub fn new(component: DiagnosticComponent, event: &str, level: DiagnosticLevel) -> Self {
        Self {
            timestamp_unix_ms: unix_timestamp_ms(),
            component,
            event: bounded_identifier(event, MAX_EVENT_NAME_BYTES),
            level,
            request_id: None,
            download_id: None,
            plugin_id: None,
            operation_id: None,
            error_code: None,
        }
    }

    #[must_use]
    pub fn with_request_id(mut self, value: impl Into<String>) -> Self {
        self.request_id = Some(value.into());
        self
    }

    #[must_use]
    pub fn with_download_id(mut self, value: impl Into<String>) -> Self {
        self.download_id = Some(value.into());
        self
    }

    #[must_use]
    pub fn with_plugin_id(mut self, value: &str) -> Self {
        self.plugin_id = safe_plugin_id(value);
        self
    }

    #[must_use]
    pub fn with_operation_id(mut self, value: impl Into<String>) -> Self {
        self.operation_id = Some(value.into());
        self
    }

    #[must_use]
    pub fn with_error_code(mut self, value: &str) -> Self {
        self.error_code = Some(bounded_identifier(value, 48));
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticMetrics {
    pub downloads_started: u64,
    pub downloads_completed: u64,
    pub downloads_failed: u64,
    pub engine_failures: u64,
    pub downloads_cancelled: u64,
    pub downloads_retried: u64,
    pub queue_enqueued: u64,
    pub queue_dequeued: u64,
    pub queue_paused: u64,
    pub queue_requeued: u64,
    pub rpc_requests: u64,
    pub rpc_successes: u64,
    pub rpc_failures: u64,
    pub rpc_authentication_failures: u64,
    pub rpc_authorization_failures: u64,
    pub rpc_rate_limited: u64,
    pub plugin_starts: u64,
    pub plugin_crashes: u64,
    pub plugin_restarts: u64,
    pub plugins_disabled: u64,
    pub plugin_verification_failures: u64,
    pub storage_migration_attempts: u64,
    pub storage_migration_failures: u64,
    pub storage_integrity_failures: u64,
    pub storage_backup_failures: u64,
    pub storage_export_failures: u64,
    pub storage_import_failures: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCounter {
    DownloadsStarted,
    DownloadsCompleted,
    DownloadsFailed,
    EngineFailures,
    DownloadsCancelled,
    DownloadsRetried,
    QueueEnqueued,
    QueueDequeued,
    QueuePaused,
    QueueRequeued,
    RpcRequests,
    RpcSuccesses,
    RpcFailures,
    RpcAuthenticationFailures,
    RpcAuthorizationFailures,
    RpcRateLimited,
    PluginStarts,
    PluginCrashes,
    PluginRestarts,
    PluginsDisabled,
    PluginVerificationFailures,
    StorageMigrationAttempts,
    StorageMigrationFailures,
    StorageIntegrityFailures,
    StorageBackupFailures,
    StorageExportFailures,
    StorageImportFailures,
}

#[derive(Clone)]
pub struct DiagnosticsRecorder {
    state: Arc<Mutex<RecorderState>>,
    event_limit: usize,
}

#[derive(Default)]
struct RecorderState {
    metrics: DiagnosticMetrics,
    events: VecDeque<DiagnosticEvent>,
}

impl Default for DiagnosticsRecorder {
    fn default() -> Self {
        Self::new(DEFAULT_DIAGNOSTIC_EVENT_LIMIT)
    }
}

impl DiagnosticsRecorder {
    #[must_use]
    pub fn new(event_limit: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(RecorderState::default())),
            event_limit: event_limit.max(1),
        }
    }

    pub fn record(&self, mut event: DiagnosticEvent) {
        event.event = bounded_identifier(&event.event, MAX_EVENT_NAME_BYTES);
        event.request_id = event.request_id.and_then(|id| safe_uuid(&id));
        event.download_id = event.download_id.and_then(|id| safe_uuid(&id));
        event.plugin_id = event.plugin_id.and_then(|id| safe_plugin_id(&id));
        event.operation_id = event.operation_id.and_then(|id| safe_uuid(&id));
        event.error_code = event.error_code.map(|code| bounded_identifier(&code, 48));

        let mut state = self.lock_state();
        if self.event_limit == 0 {
            return;
        }
        while state.events.len() >= self.event_limit {
            state.events.pop_front();
        }
        state.events.push_back(event);
    }

    pub fn increment(&self, counter: DiagnosticCounter) {
        let mut state = self.lock_state();
        let value = match counter {
            DiagnosticCounter::DownloadsStarted => &mut state.metrics.downloads_started,
            DiagnosticCounter::DownloadsCompleted => &mut state.metrics.downloads_completed,
            DiagnosticCounter::DownloadsFailed => &mut state.metrics.downloads_failed,
            DiagnosticCounter::EngineFailures => &mut state.metrics.engine_failures,
            DiagnosticCounter::DownloadsCancelled => &mut state.metrics.downloads_cancelled,
            DiagnosticCounter::DownloadsRetried => &mut state.metrics.downloads_retried,
            DiagnosticCounter::QueueEnqueued => &mut state.metrics.queue_enqueued,
            DiagnosticCounter::QueueDequeued => &mut state.metrics.queue_dequeued,
            DiagnosticCounter::QueuePaused => &mut state.metrics.queue_paused,
            DiagnosticCounter::QueueRequeued => &mut state.metrics.queue_requeued,
            DiagnosticCounter::RpcRequests => &mut state.metrics.rpc_requests,
            DiagnosticCounter::RpcSuccesses => &mut state.metrics.rpc_successes,
            DiagnosticCounter::RpcFailures => &mut state.metrics.rpc_failures,
            DiagnosticCounter::RpcAuthenticationFailures => {
                &mut state.metrics.rpc_authentication_failures
            }
            DiagnosticCounter::RpcAuthorizationFailures => {
                &mut state.metrics.rpc_authorization_failures
            }
            DiagnosticCounter::RpcRateLimited => &mut state.metrics.rpc_rate_limited,
            DiagnosticCounter::PluginStarts => &mut state.metrics.plugin_starts,
            DiagnosticCounter::PluginCrashes => &mut state.metrics.plugin_crashes,
            DiagnosticCounter::PluginRestarts => &mut state.metrics.plugin_restarts,
            DiagnosticCounter::PluginsDisabled => &mut state.metrics.plugins_disabled,
            DiagnosticCounter::PluginVerificationFailures => {
                &mut state.metrics.plugin_verification_failures
            }
            DiagnosticCounter::StorageMigrationAttempts => {
                &mut state.metrics.storage_migration_attempts
            }
            DiagnosticCounter::StorageMigrationFailures => {
                &mut state.metrics.storage_migration_failures
            }
            DiagnosticCounter::StorageIntegrityFailures => {
                &mut state.metrics.storage_integrity_failures
            }
            DiagnosticCounter::StorageBackupFailures => &mut state.metrics.storage_backup_failures,
            DiagnosticCounter::StorageExportFailures => &mut state.metrics.storage_export_failures,
            DiagnosticCounter::StorageImportFailures => &mut state.metrics.storage_import_failures,
        };
        *value = value.saturating_add(1);
    }

    #[must_use]
    pub fn metrics(&self) -> DiagnosticMetrics {
        self.lock_state().metrics
    }

    #[must_use]
    pub fn recent_events(&self) -> Vec<DiagnosticEvent> {
        self.lock_state().events.iter().cloned().collect()
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, RecorderState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub fn global_diagnostics() -> &'static DiagnosticsRecorder {
    static RECORDER: OnceLock<DiagnosticsRecorder> = OnceLock::new();
    RECORDER.get_or_init(DiagnosticsRecorder::default)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticComponentStatus {
    pub health: DiagnosticHealth,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicationDiagnostics {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
    pub target_os: String,
    pub target_arch: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueueDiagnostics {
    pub health: DiagnosticHealth,
    pub total_downloads: u64,
    pub active_downloads: u64,
    pub queued_downloads: u64,
    pub paused_downloads: u64,
    pub failed_downloads: u64,
    pub completed_downloads: u64,
    pub cancelled_downloads: u64,
    pub max_active_downloads: Option<u64>,
    pub global_connection_limit: Option<u64>,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: Option<u64>,
    pub current_speed_bytes_per_second: u64,
    pub average_speed_bytes_per_second: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngineDiagnostics {
    pub engine: String,
    pub health: DiagnosticHealth,
    pub active_downloads: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StorageDiagnostics {
    pub health: DiagnosticHealth,
    pub schema_version: Option<i64>,
    pub database_check_latency_ms: Option<u64>,
    pub migration_state: Option<String>,
    pub integrity: Option<String>,
    pub database_size_bytes: Option<u64>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RpcDiagnostics {
    pub health: DiagnosticHealth,
    pub active_connections: Option<u64>,
    pub metrics: DiagnosticMetrics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginDiagnostics {
    pub health: DiagnosticHealth,
    pub discovered_plugins: Option<u64>,
    pub running_plugins: Option<u64>,
    pub metrics: DiagnosticMetrics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticsSnapshot {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub health: DiagnosticHealth,
    pub application: ApplicationDiagnostics,
    pub components: Vec<DiagnosticComponentStatus>,
    pub queue: QueueDiagnostics,
    pub engines: Vec<EngineDiagnostics>,
    pub storage: StorageDiagnostics,
    pub rpc: RpcDiagnostics,
    pub plugins: PluginDiagnostics,
    pub metrics: DiagnosticMetrics,
    pub recent_events: Vec<DiagnosticEvent>,
}

#[must_use]
pub fn sanitize_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return "[invalid-url]".to_owned();
    };
    if matches!(url.scheme(), "magnet" | "data") {
        return format!("{}:<redacted>", url.scheme());
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let sanitized = url.to_string();
    if sanitized.len() > 512 {
        format!("{}…", &sanitized[..512])
    } else {
        sanitized
    }
}

#[must_use]
pub fn redact_sensitive_text(value: &str) -> String {
    if value.contains("-----BEGIN ") && value.contains("PRIVATE KEY-----") {
        return "[REDACTED PRIVATE KEY]".to_owned();
    }
    value
        .split('\n')
        .map(|line| {
            let mut output = Vec::new();
            let mut redact_following = 0_u8;
            let mut redact_cookie = false;
            for token in line.split_whitespace() {
                if redact_cookie {
                    if token.starts_with("http://") || token.starts_with("https://") {
                        redact_cookie = false;
                    } else if let Some((label, _)) = sensitive_assignment_label(token) {
                        let label_in_source = token
                            .find(['=', ':'])
                            .map(|boundary| &token[..boundary])
                            .unwrap_or_default();
                        if label_in_source
                            .chars()
                            .any(|character| character.is_ascii_uppercase())
                            && !matches!(label.as_str(), "session" | "token" | "secret")
                        {
                            redact_cookie = false;
                        } else {
                            output.push("[REDACTED]".to_owned());
                            continue;
                        }
                    } else {
                        output.push("[REDACTED]".to_owned());
                        continue;
                    }
                }
                if redact_following > 0 {
                    if sensitive_assignment_label(token).is_some() {
                        redact_following = 0;
                    } else {
                        output.push("[REDACTED]".to_owned());
                        redact_following -= 1;
                        continue;
                    }
                }
                if let Some(sanitized) = sanitize_url_token(token) {
                    output.push(sanitized);
                    continue;
                }
                if let Some((label, boundary)) = sensitive_assignment_label(token) {
                    let delimiter = &token[boundary..=boundary];
                    if label == "cookie" || label == "set-cookie" {
                        output.push(format!("{}{}[REDACTED]", &token[..boundary], delimiter));
                        redact_cookie = true;
                    } else if token.len() > boundary + 1 {
                        output.push(format!("{}{}[REDACTED]", &token[..boundary], delimiter));
                    } else {
                        output.push(format!("{}{}", &token[..boundary], delimiter));
                        redact_following = if label == "authorization" { 2 } else { 1 };
                    }
                    continue;
                }
                output.push(token.to_owned());
            }
            output.join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sanitize_url_token(token: &str) -> Option<String> {
    let url_start = token
        .find("://")
        .map(|scheme_end| {
            token[..scheme_end]
                .rfind(|character: char| {
                    !character.is_ascii_alphanumeric() && !"+-.".contains(character)
                })
                .map_or(0, |separator| separator + 1)
        })
        .into_iter()
        .chain(
            [token.find("magnet:"), token.find("data:")]
                .into_iter()
                .flatten(),
        )
        .min()?;
    Some(format!(
        "{}{}",
        &token[..url_start],
        sanitize_url(&token[url_start..])
    ))
}

fn sensitive_assignment_label(token: &str) -> Option<(String, usize)> {
    let boundary = token.find(['=', ':'])?;
    let label = token[..boundary]
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .to_ascii_lowercase();
    is_sensitive_label(&label).then_some((label, boundary))
}

fn is_sensitive_label(label: &str) -> bool {
    matches!(
        label,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "api_key"
            | "api-key"
            | "apikey"
            | "password"
            | "passwd"
            | "token"
            | "access_token"
            | "refresh_token"
            | "secret"
            | "session"
            | "session_string"
            | "private_key"
            | "credential"
            | "credentials"
    )
}

fn bounded_identifier(value: &str, max_bytes: usize) -> String {
    if value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return "unspecified".to_owned();
    }
    value.to_owned()
}

fn safe_uuid(value: &str) -> Option<String> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(|id| id.hyphenated().to_string())
}

fn safe_plugin_id(value: &str) -> Option<String> {
    if value.len() <= 128
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_events_are_bounded_and_keep_safe_correlation_ids() {
        let recorder = DiagnosticsRecorder::new(2);
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        let third = uuid::Uuid::new_v4().to_string();
        recorder.record(
            DiagnosticEvent::new(DiagnosticComponent::Core, "started", DiagnosticLevel::Info)
                .with_download_id(first.clone())
                .with_operation_id(uuid::Uuid::new_v4().to_string()),
        );
        recorder.record(
            DiagnosticEvent::new(DiagnosticComponent::Core, "paused", DiagnosticLevel::Info)
                .with_download_id(second.clone()),
        );
        recorder.record(
            DiagnosticEvent::new(
                DiagnosticComponent::Core,
                "completed",
                DiagnosticLevel::Info,
            )
            .with_download_id(third.clone()),
        );

        let events = recorder.recent_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].download_id.as_deref(), Some(second.as_str()));
        assert_eq!(events[1].download_id.as_deref(), Some(third.as_str()));
        assert_eq!(
            events[0].event, "paused",
            "fixed event names remain available for diagnosis"
        );
    }

    #[test]
    fn metrics_are_saturating_and_serializable() -> Result<(), serde_json::Error> {
        let recorder = DiagnosticsRecorder::default();
        recorder.increment(DiagnosticCounter::DownloadsStarted);
        recorder.increment(DiagnosticCounter::EngineFailures);
        recorder.increment(DiagnosticCounter::RpcAuthenticationFailures);
        let metrics = recorder.metrics();
        assert_eq!(metrics.downloads_started, 1);
        assert_eq!(metrics.engine_failures, 1);
        assert_eq!(metrics.rpc_authentication_failures, 1);
        let encoded = serde_json::to_string(&metrics)?;
        assert!(encoded.contains("\"downloads_started\":1"));
        Ok(())
    }

    #[test]
    fn diagnostic_snapshot_schema_round_trips() -> Result<(), serde_json::Error> {
        let snapshot = DiagnosticsSnapshot {
            schema_version: DIAGNOSTICS_SCHEMA_VERSION,
            generated_at_unix_ms: 10,
            health: DiagnosticHealth::Degraded,
            application: ApplicationDiagnostics {
                name: "FetchX".to_owned(),
                version: "0.1.0".to_owned(),
                protocol_version: "fetchx.rpc.v1".to_owned(),
                target_os: "windows".to_owned(),
                target_arch: "x86_64".to_owned(),
            },
            components: vec![DiagnosticComponentStatus {
                health: DiagnosticHealth::Unavailable,
                error_code: Some("STORAGE_UNAVAILABLE".to_owned()),
            }],
            queue: QueueDiagnostics {
                health: DiagnosticHealth::Unavailable,
                total_downloads: 0,
                active_downloads: 0,
                queued_downloads: 0,
                paused_downloads: 0,
                failed_downloads: 0,
                completed_downloads: 0,
                cancelled_downloads: 0,
                max_active_downloads: None,
                global_connection_limit: None,
                bytes_downloaded: 0,
                bytes_uploaded: None,
                current_speed_bytes_per_second: 0,
                average_speed_bytes_per_second: 0,
            },
            engines: Vec::new(),
            storage: StorageDiagnostics {
                health: DiagnosticHealth::Unavailable,
                schema_version: None,
                database_check_latency_ms: None,
                migration_state: None,
                integrity: None,
                database_size_bytes: None,
                error_code: Some("STORAGE_UNAVAILABLE".to_owned()),
            },
            rpc: RpcDiagnostics {
                health: DiagnosticHealth::Unavailable,
                active_connections: None,
                metrics: DiagnosticMetrics::default(),
            },
            plugins: PluginDiagnostics {
                health: DiagnosticHealth::Unavailable,
                discovered_plugins: None,
                running_plugins: None,
                metrics: DiagnosticMetrics::default(),
            },
            metrics: DiagnosticMetrics::default(),
            recent_events: Vec::new(),
        };
        let encoded = serde_json::to_string(&snapshot)?;
        let decoded: DiagnosticsSnapshot = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, snapshot);
        Ok(())
    }

    #[test]
    fn diagnostic_redaction_removes_credentials_urls_and_private_keys() {
        let text = concat!(
            "Authorization: Bearer bearer-secret ",
            "Cookie: session= cookie-secret ",
            "API_KEY=api-secret Password: password-secret ",
            "PRIVATE_KEY=private-secret SESSION_STRING=session-secret ",
            "https://user:pass@example.com/path?token=url-secret#frag"
        );
        let redacted = redact_sensitive_text(text);
        for secret in [
            "bearer-secret",
            "cookie-secret",
            "api-secret",
            "password-secret",
            "private-secret",
            "session-secret",
            "user:pass",
            "url-secret",
            "frag",
        ] {
            assert!(!redacted.contains(secret), "{redacted}");
        }
        assert!(redacted.contains("example.com/path"));
        let private_key = "-----BEGIN PRIVATE KEY-----\\nsensitive\\n-----END PRIVATE KEY-----";
        assert!(!redact_sensitive_text(private_key).contains("sensitive"));
    }

    #[test]
    fn redacts_wrapped_urls_and_non_http_url_schemes() {
        let redacted = redact_sensitive_text(
            "url=(https://user:pass@example.com/path?token=secret) magnet:?xt=private data:text/plain,secret",
        );
        for secret in [
            "user",
            "pass",
            "token=secret",
            "private",
            "text/plain,secret",
        ] {
            assert!(!redacted.contains(secret), "{redacted}");
        }
        assert!(redacted.contains("example.com/path"));
        assert!(redacted.contains("magnet:<redacted>"));
        assert!(redacted.contains("data:<redacted>"));
    }

    #[test]
    fn url_sanitization_drops_userinfo_query_and_fragment() {
        let url = sanitize_url("https://name:password@example.com/path?api_key=secret#session");
        assert_eq!(url, "https://example.com/path");
        assert_eq!(sanitize_url("not a url"), "[invalid-url]");
        assert_eq!(sanitize_url("magnet:?xt=secret"), "magnet:<redacted>");
    }
}
