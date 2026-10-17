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

export type EngineKind = "direct_http" | "aria2" | "yt_dlp";

export interface Download {
  id: string;
  url: string;
  filename: string;
  destination: string;
  status: DownloadStatus;
  priority: DownloadPriority;
  engine: EngineKind;
  progress: DownloadProgress;
  error: DownloadError | null;
  created_at: number;
  updated_at: number;
}

export interface AddDownloadRequest {
  url: string;
  /** Optional engine selection. Omitting defaults to Direct HTTP on the backend. */
  engine_kind?: EngineKind;
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
}

// ---------------------------------------------------------------------------
// Engine configuration
// ---------------------------------------------------------------------------

export interface DirectHttpConfigDto {
  enabled: boolean;
}

export interface Aria2ConfigDto {
  enabled: boolean;
  executable_path: string | null;
  rpc_port: number;
  /** True when a non-empty secret is stored. The actual secret is never returned. */
  has_secret: boolean;
}

export interface YtDlpConfigDto {
  enabled: boolean;
  executable_path: string | null;
}

export interface EngineConfigDto {
  direct_http: DirectHttpConfigDto;
  aria2: Aria2ConfigDto;
  yt_dlp: YtDlpConfigDto;
  resource_policy: ResourcePolicyDto;
}

export type ResourceProfile = "unlimited" | "normal" | "limited" | "night" | "custom";

export interface EngineBandwidthLimitsDto {
  direct_http_bytes_per_second: number | null;
  hls_bytes_per_second: number | null;
  aria2_bytes_per_second: number | null;
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

export interface SaveAria2ConfigRequest {
  enabled: boolean;
  executable_path: string | null;
  rpc_port: number;
  /** null = leave existing secret unchanged. "" = clear secret. "token" = set. */
  rpc_secret_update: string | null;
}

export interface SaveYtDlpConfigRequest {
  enabled: boolean;
  executable_path: string | null;
}

export interface SaveEngineConfigRequest {
  aria2: SaveAria2ConfigRequest;
  yt_dlp: SaveYtDlpConfigRequest;
  resource_policy: ResourcePolicyDto;
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
  aria2: EngineInstanceStatus;
  yt_dlp: EngineInstanceStatus;
}
