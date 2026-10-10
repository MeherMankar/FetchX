export type DownloadStatus =
  | "queued"
  | "preparing"
  | "downloading"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

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
