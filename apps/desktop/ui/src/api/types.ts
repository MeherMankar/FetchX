export type DownloadStatus =
  | "queued"
  | "preparing"
  | "downloading"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled"
  | "metadata_acquiring"
  | "seeding"
  | "recovering"
  | "stopped";

export type DownloadPriority = "low" | "normal" | "high";

export type DownloadErrorCode = "source" | "destination" | "engine" | "internal";

export interface AppInfo {
  name: string;
  version: string;
  ipc_version: number;
}

export interface BrowserRegistrationResult {
  browser: string;
  detected: boolean;
  native_host_registered: boolean;
  extension_registered: boolean;
  error: string | null;
}

export interface BrowserExtensionStatus {
  extension_id_chromium: string;
  extension_id_firefox: string;
  chromium_extension_path: string;
  firefox_extension_path: string;
  chromium_extension_installed: boolean;
  firefox_extension_installed: boolean;
  native_host_binary_path: string | null;
  browsers: BrowserRegistrationResult[];
}

export interface DownloadProgress {
  downloaded_bytes: number;
  total_bytes: number | null;
  speed_bytes_per_second: number | null;
  estimated_remaining_seconds: number | null;
  percentage_basis_points: number | null;
}

export interface DownloadError {
  code: DownloadErrorCode;
  message: string;
}

export type EngineKind = "direct_http" | "yt_dlp" | "native_torrent" | "metalink" | "ed2k";

export interface Download {
  id: string;
  url: string;
  filename: string;
  destination: string;
  status: DownloadStatus;
  priority: DownloadPriority;
  engine: EngineKind | "aria2";
  progress: DownloadProgress;
  error: DownloadError | null;
  queue_position?: number;
  created_at: number;
  updated_at: number;
  bandwidth_limit?: number | null;
}

export interface DownloadRuntimeStatus {
  id: string;
  status: DownloadStatus;
  progress: DownloadProgress;
  updated_at: number;
}

export type MediaJobStatus =
  | "preparing"
  | "processing"
  | "completed"
  | "failed"
  | "cancelled";

export interface MediaProcessJob {
  job_id: string;
  download_id: string;
  status: MediaJobStatus;
  output_time_seconds: number;
  duration_seconds: number | null;
  output_path: string | null;
  output_size_bytes: number | null;
  error: string | null;
}

export interface DuplicateDownloadMatch {
  id: string;
  filename: string;
  status: DownloadStatus;
  updated_at: number;
}

export interface AddDownloadRequest {
  url: string;
  package_name?: string;
  /** Optional engine selection. Omitting defaults to Direct HTTP on the backend. */
  engine_kind?: EngineKind;
  /** Extra tracker announce URLs for torrent/magnet downloads. */
  extra_trackers?: string[];
  /** Referer URL forwarded as a request header. */
  referer?: string;
  /** User-Agent header override for this download. */
  user_agent?: string;
  /** Raw Cookie header value for this download. */
  cookies?: string;
  /** Per-download proxy URL that overrides the global proxy setting. */
  proxy_url?: string;
  /** Per-download subtitle download override. Omitting defers to the global setting. */
  download_subtitles?: boolean;
  /** Per-download subtitle languages, comma-separated (e.g. "en,fr"). */
  subtitle_langs?: string;
  /** When set for an HLS download, download the specific variant URL instead of the master playlist. */
  hls_variant_url?: string;
}

export interface BatchDownloadResult {
  input_url: string;
  download_id: string | null;
  error: string | null;
}

export interface TorrentTelemetry {
  download_speed_bps: number | null;
  upload_speed_bps: number | null;
  downloaded_bytes: number | null;
  uploaded_bytes: number | null;
  peer_count: number | null;
  completed_peer_count: number | null;
  seed_count: number | null;
  share_ratio: number | null;
  seeding: boolean | null;
  tracker_uris: string[] | null;
  connected_peers: number | null;
  total_peers: number | null;
}

export interface TorrentInspection {
  info_hash: string;
  name: string;
  total_size: number;
  files: Array<{ index: number; path: string; size: number }>;
}

export interface TorrentDownloadMetadata {
  metadata_state: string;
  metadata: {
    info_hash: string;
    name: string;
    total_size: number;
    files: Array<{ path: string[]; size: number }>;
    trackers: string[];
    piece_length: number;
    piece_count: number;
  } | null;
  selected_file_indexes: number[];
  selected_total_bytes: number;
  selection_confirmed: boolean;
}

export interface YtDlpFormat {
  format_id: string;
  label: string;
  extension: string | null;
  file_size_bytes: number | null;
}

export interface HlsVariantDto {
  url: string;
  bandwidth_bps: number;
  resolution: string | null;
  codecs: string | null;
  label: string;
  is_audio_only: boolean;
}

export interface SetPriorityRequest {
  priority: DownloadPriority;
}

export interface ScheduleDownloadRequest {
  /** Unix milliseconds at which the download should start. */
  scheduled_at_ms: number;
}

export interface DownloadSchedule {
  download_id: string;
  /** Unix milliseconds at which the download is scheduled to start. */
  scheduled_at_ms: number;
  enabled: boolean;
}

export interface DeleteDownloadResponse {
  download_id: string;
  removed: boolean;
}

export interface IpcError {
  code: "invalid_request" | "not_found" | "conflict" | "internal";
  message: string;
}

export interface DownloadUpdatedEvent {
  download: Download;
}

export interface DownloadRemovedEvent {
  download_id: string;
}

export interface QueueSnapshot {
  active_count: number;
  queued_count: number;
  paused_count: number;
  used_connections: number;
  total_connections: number;
  dispatch_paused: boolean;
  manual_dispatch_paused: boolean;
  schedule_dispatch_paused: boolean;
}

// ---------------------------------------------------------------------------
// Engine configuration
// ---------------------------------------------------------------------------

export interface DirectHttpConfigDto {
  enabled: boolean;
}

export interface YtDlpConfigDto {
  enabled: boolean;
  download_subtitles: boolean;
  subtitle_langs: string;
}

export interface MetalinkConfigDto {
  enabled: boolean;
}

export interface EngineConfigDto {
  direct_http: DirectHttpConfigDto;
  yt_dlp: YtDlpConfigDto;
  resource_policy: ResourcePolicyDto;
  weekly_queue_schedule: WeeklyQueueScheduleDto;
  os_notifications_enabled: boolean;
  native_builtins: NativeBuiltinsConfigDto;
  metalink: MetalinkConfigDto;
  auto_retry_count: number;
  auto_retry_delay_seconds: number;
  virus_scan_on_complete: boolean;
  mark_of_the_web: boolean;
  post_queue_action: PostQueueActionDto;
}

export interface NativeBuiltinsConfigDto {
  filename_template: string;
  page_scraper_enabled: boolean;
  url_resolver_enabled: boolean;
}

export type ResourceProfile = "unlimited" | "normal" | "limited" | "night" | "custom";

export interface EngineBandwidthLimitsDto {
  direct_http_bytes_per_second: number | null;
  hls_bytes_per_second: number | null;
  yt_dlp_bytes_per_second: number | null;
}

export interface DesktopProxyConfigDto {
  kind: "http" | "https" | "socks5";
  host: string;
  port: number;
  username: string | null;
}

export interface ResourcePolicyDto {
  profile: ResourceProfile;
  max_active_downloads: number;
  global_connection_limit: number;
  per_download_connection_limit: number;
  global_bandwidth_bytes_per_second: number | null;
  per_download_bandwidth_bytes_per_second: number | null;
  engine_bandwidth_limits: EngineBandwidthLimitsDto;
  desktop_proxy: DesktopProxyConfigDto | null;
  weekly_bandwidth_schedule?: WeeklyBandwidthScheduleDto;
}

export interface WeeklyQueueScheduleDto {
  enabled: boolean;
  /** Monday through Sunday. */
  weekdays: boolean[];
  /** Minutes after midnight in the system's local time zone. */
  start_minute: number;
  /** Minutes after midnight in the system's local time zone. */
  end_minute: number;
}

export interface SaveYtDlpConfigRequest {
  enabled: boolean;
  download_subtitles: boolean;
  subtitle_langs: string;
}

export interface SaveEngineConfigRequest {
  yt_dlp: SaveYtDlpConfigRequest;
  resource_policy: ResourcePolicyDto;
  weekly_queue_schedule: WeeklyQueueScheduleDto;
  native_builtins?: NativeBuiltinsConfigDto;
  metalink?: MetalinkConfigDto;
  auto_retry_count?: number;
  auto_retry_delay_seconds?: number;
  virus_scan_on_complete?: boolean;
  mark_of_the_web?: boolean;
  post_queue_action?: PostQueueActionDto;
}

export interface DiagnosticResult {
  available: boolean;
  version: string | null;
  message: string;
}

export interface EngineIpcError {
  code: "invalid_config";
  message: string;
}

// ---------------------------------------------------------------------------
// Engine runtime status
// ---------------------------------------------------------------------------

export interface EngineInstanceStatus {
  enabled: boolean;
  available: boolean;
  version: string | null;
  /** Human-readable error when available == false and enabled == true. */
  error: string | null;
}

export interface EngineStatus {
  direct_http: EngineInstanceStatus;
  yt_dlp: EngineInstanceStatus;
  native_torrent: EngineInstanceStatus;
}


// ---------------------------------------------------------------------------
// Real-time queue stats (from QueueStats event)
// ---------------------------------------------------------------------------

export interface QueueStatsEvent {
  speed_bps: number;
  active_count: number;
  queued_count: number;
  estimated_completion_seconds: number | null;
}

// ---------------------------------------------------------------------------
// Notification event
// ---------------------------------------------------------------------------

export interface NotificationEvent {
  title: string;
  body: string;
}

// ---------------------------------------------------------------------------
// Bandwidth schedule (T22)
// ---------------------------------------------------------------------------

export interface BandwidthWindowDto {
  weekdays: boolean[];
  start_minute: number;
  end_minute: number;
  limit_bps: number;
}

export interface WeeklyBandwidthScheduleDto {
  windows: BandwidthWindowDto[];
}

// ---------------------------------------------------------------------------
// Post-download action (feature2)
// ---------------------------------------------------------------------------

export type PostDownloadActionKind =
  | "nothing"
  | "open_file"
  | "open_folder"
  | "run_command"
  | "extract_archive";

export interface PostDownloadActionDto {
  kind: PostDownloadActionKind;
  /** Present only when kind is "run_command". */
  command?: string;
  /** Present only when kind is "extract_archive". */
  delete_after?: boolean;
}

// ---------------------------------------------------------------------------
// Post-queue action
// ---------------------------------------------------------------------------

export type PostQueueActionDto = "nothing" | "shutdown" | "sleep" | "hibernate";

// ---------------------------------------------------------------------------
// Folder categories (folder-categories feature)
// ---------------------------------------------------------------------------

export interface CategoryRuleDto {
  folder: string;
  extensions: string;
}

export interface FolderCategoriesConfigDto {
  enabled: boolean;
  compressed: CategoryRuleDto;
  documents: CategoryRuleDto;
  music: CategoryRuleDto;
  programs: CategoryRuleDto;
  video: CategoryRuleDto;
  images: CategoryRuleDto;
  custom: string;
}

export interface UpdateCheckResult {
  available: boolean;
  current_version: string;
  latest_version: string;
  release_url: string;
  release_notes: string;
}

// ---------------------------------------------------------------------------
// Webhook configuration
// ---------------------------------------------------------------------------

export interface WebhookConfigDto {
  enabled: boolean;
  url: string;
  on_complete: boolean;
  on_fail: boolean;
  secret: string;
}

// ---------------------------------------------------------------------------
// Checksum verification
// ---------------------------------------------------------------------------

export interface ChecksumVerificationResult {
  /** Hex SHA-256 from the sidecar file, or null if no sidecar exists. */
  expected: string | null;
  /** Hex SHA-256 computed from the file, or null if the file is missing. */
  actual: string | null;
  /** True when hashes match, false on mismatch, null when no expected hash. */
  matches: boolean | null;
  /** File size in bytes, or null if the file is missing. */
  file_size: number | null;
}

// ---------------------------------------------------------------------------
// HTTP authentication
// ---------------------------------------------------------------------------

/** Authentication scheme a user selected when providing credentials. */
export type HttpAuthType = "basic" | "digest" | "ntlm";
