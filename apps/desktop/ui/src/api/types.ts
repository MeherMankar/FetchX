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

export type EngineKind = "direct_http" | "yt_dlp" | "native_torrent";

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
  executable_path: string | null;
}

export interface EngineConfigDto {
  direct_http: DirectHttpConfigDto;
  yt_dlp: YtDlpConfigDto;
  resource_policy: ResourcePolicyDto;
  weekly_queue_schedule: WeeklyQueueScheduleDto;
}

export type ResourceProfile = "unlimited" | "normal" | "limited" | "night" | "custom";

export interface EngineBandwidthLimitsDto {
  direct_http_bytes_per_second: number | null;
  hls_bytes_per_second: number | null;
  yt_dlp_bytes_per_second: number | null;
}

export interface ResourcePolicyDto {
  profile: ResourceProfile;
  max_active_downloads: number;
  global_connection_limit: number;
  per_download_connection_limit: number;
  global_bandwidth_bytes_per_second: number | null;
  per_download_bandwidth_bytes_per_second: number | null;
  engine_bandwidth_limits: EngineBandwidthLimitsDto;
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
  executable_path: string | null;
}

export interface SaveEngineConfigRequest {
  yt_dlp: SaveYtDlpConfigRequest;
  resource_policy: ResourcePolicyDto;
  weekly_queue_schedule: WeeklyQueueScheduleDto;
}

export interface DiagnosticResult {
  available: boolean;
  executable: string | null;
  version: string | null;
  message: string;
}

export interface EngineIpcError {
  code: "invalid_config" | "executable_not_found" | "executable_invalid";
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

export type ManagedHelperId = "yt_dlp";

export interface HelperStatus {
  helper: ManagedHelperId;
  installed: boolean;
  executable_path: string | null;
  version: string | null;
  architecture: string | null;
  available: boolean;
  validation: string;
  install_supported: boolean;
}

export interface HelperInstallProgress {
  helper: ManagedHelperId;
  phase: "downloading" | "verifying" | "installing" | "completed" | "failed" | "cancelled";
  received_bytes: number;
  total_bytes: number | null;
  message: string | null;
}
