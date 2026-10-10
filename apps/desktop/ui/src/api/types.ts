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

export interface Download {
  id: string;
  url: string;
  filename: string;
  destination: string;
  status: DownloadStatus;
  priority: DownloadPriority;
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
