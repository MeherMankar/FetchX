import type { Download, DownloadStatus } from "../api/types";

export function isTerminal(status: DownloadStatus): boolean {
  return status === "completed" || status === "failed" || status === "cancelled" || status === "stopped";
}

export function isActive(status: DownloadStatus): boolean {
  return status === "downloading"
    || status === "preparing"
    || status === "metadata_acquiring"
    || status === "recovering"
    || status === "seeding";
}

export function canPause(status: DownloadStatus): boolean {
  return status === "queued" || status === "preparing" || status === "downloading"
    || status === "metadata_acquiring" || status === "recovering" || status === "seeding";
}

export function canResume(status: DownloadStatus): boolean {
  return status === "paused";
}

export function canCancel(status: DownloadStatus): boolean {
  return canPause(status) || status === "paused";
}

export function canChangePriority(status: DownloadStatus): boolean {
  return !isTerminal(status);
}

export function canSchedule(status: DownloadStatus): boolean {
  return status === "queued" || status === "paused";
}

export function canRetry(status: DownloadStatus): boolean {
  return status === "failed" || status === "cancelled" || status === "stopped";
}

// ---------------------------------------------------------------------------
// Summary counts derived from download list
// ---------------------------------------------------------------------------

export interface StatusCounts {
  total: number;
  active: number;
  queued: number;
  paused: number;
  completed: number;
  failed: number;
  cancelled: number;
  scheduled: number; // has an enabled schedule (maintained by caller)
}

export function countStatuses(downloads: Download[]): Omit<StatusCounts, "scheduled"> {
  let active = 0, queued = 0, paused = 0, completed = 0, failed = 0, cancelled = 0;
  for (const d of downloads) {
    switch (d.status) {
      case "downloading":
      case "preparing":
      case "metadata_acquiring":
      case "recovering":
      case "seeding":
        active++;
        break;
      case "queued":
        queued++;
        break;
      case "paused":
        paused++;
        break;
      case "completed":
      case "stopped":
        completed++;
        break;
      case "failed":
        failed++;
        break;
      case "cancelled":
        cancelled++;
        break;
    }
  }
  return { total: downloads.length, active, queued, paused, completed, failed, cancelled };
}
