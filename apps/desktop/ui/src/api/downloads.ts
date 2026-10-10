import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AddDownloadRequest,
  DeleteDownloadResponse,
  Download,
  DownloadRemovedEvent,
  DownloadSchedule,
  DownloadUpdatedEvent,
  QueueSnapshot,
  ScheduleDownloadRequest,
  SetPriorityRequest,
} from "./types";
import type { DownloadPriority } from "./types";

export function listDownloads(): Promise<Download[]> {
  return invoke<Download[]>("list_downloads");
}

export function addDownload(request: AddDownloadRequest): Promise<Download> {
  return invoke<Download>("add_download", { request });
}

export function pauseDownload(downloadId: string): Promise<Download> {
  return invoke<Download>("pause_download", { download_id: downloadId });
}

export function resumeDownload(downloadId: string): Promise<Download> {
  return invoke<Download>("resume_download", { download_id: downloadId });
}

export function cancelDownload(downloadId: string): Promise<Download> {
  return invoke<Download>("cancel_download", { download_id: downloadId });
}

export function deleteDownload(downloadId: string): Promise<DeleteDownloadResponse> {
  return invoke<DeleteDownloadResponse>("delete_download", { download_id: downloadId });
}

export function setDownloadPriority(
  downloadId: string,
  priority: DownloadPriority,
): Promise<Download> {
  const request: SetPriorityRequest = { priority };
  return invoke<Download>("set_download_priority", { download_id: downloadId, request });
}

export function scheduleDownload(
  downloadId: string,
  scheduledAtMs: number,
): Promise<DownloadSchedule> {
  const request: ScheduleDownloadRequest = { scheduled_at_ms: scheduledAtMs };
  return invoke<DownloadSchedule>("schedule_download", { download_id: downloadId, request });
}

export function removeDownloadSchedule(downloadId: string): Promise<void> {
  return invoke<void>("remove_download_schedule", { download_id: downloadId });
}

export function getDownloadSchedule(downloadId: string): Promise<DownloadSchedule | null> {
  return invoke<DownloadSchedule | null>("get_download_schedule", { download_id: downloadId });
}

export function retryDownload(downloadId: string): Promise<Download> {
  return invoke<Download>("retry_download", { download_id: downloadId });
}

export function getQueueSnapshot(): Promise<QueueSnapshot> {
  return invoke<QueueSnapshot>("get_queue_snapshot");
}

export async function listenForDownloadChanges(
  onUpdated: (download: Download) => void,
  onRemoved: (downloadId: string) => void,
): Promise<UnlistenFn> {
  const unlistenUpdated = await listen<DownloadUpdatedEvent>("download_updated", (event) => {
    onUpdated(event.payload.download);
  });
  try {
    const unlistenRemoved = await listen<DownloadRemovedEvent>("download_removed", (event) => {
      onRemoved(event.payload.download_id);
    });
    return () => {
      unlistenUpdated();
      unlistenRemoved();
    };
  } catch (error) {
    unlistenUpdated();
    throw error;
  }
}
