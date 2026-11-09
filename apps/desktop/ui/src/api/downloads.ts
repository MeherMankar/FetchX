import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AddDownloadRequest,
  Aria2RecoveryAssessment,
  BatchDownloadResult,
  DeleteDownloadResponse,
  Download,
  DuplicateDownloadMatch,
  DownloadRuntimeStatus,
  DownloadRemovedEvent,
  DownloadSchedule,
  DownloadUpdatedEvent,
  QueueSnapshot,
  ScheduleDownloadRequest,
  SetPriorityRequest,
  TorrentInspection,
  TorrentDownloadMetadata,
  TorrentTelemetry,
  YtDlpFormat,
} from "./types";
import type { DownloadPriority } from "./types";

export function listDownloads(): Promise<Download[]> {
  return invoke<Download[]>("list_downloads");
}

export function listRuntimeStatuses(): Promise<DownloadRuntimeStatus[]> {
  return invoke<DownloadRuntimeStatus[]>("list_runtime_statuses");
}

export function assessAria2Recovery(id: string): Promise<Aria2RecoveryAssessment | null> {
  return invoke<Aria2RecoveryAssessment | null>("assess_aria2_recovery", { id });
}

export function restartAria2HttpWithNative(
  id: string,
  currentUrl: string,
  destination: string,
  confirmedDirectFile: boolean,
): Promise<Download> {
  return invoke<Download>("restart_aria2_http_with_native", {
    id,
    current_url: currentUrl,
    destination,
    confirmed_direct_file: confirmedDirectFile,
  });
}

export function restartAria2TorrentWithNative(
  id: string,
  currentMagnetUri: string,
  destination: string,
  confirmedCurrentSource: boolean,
): Promise<Download> {
  return invoke<Download>("restart_aria2_torrent_with_native", {
    id,
    current_magnet_uri: currentMagnetUri,
    destination,
    confirmed_current_source: confirmedCurrentSource,
  });
}

export function addDownload(request: AddDownloadRequest): Promise<Download> {
  return invoke<Download>("add_download", { request });
}

export function addDownloadBatch(options: {
  urls: string[];
  packageName?: string;
  engine: "direct_http" | "yt_dlp";
  priority: DownloadPriority;
  formatId?: string;
  extension?: string | null;
  audioFormat?: "mp3" | "m4a";
}): Promise<BatchDownloadResult[]> {
  return invoke<BatchDownloadResult[]>("add_download_batch", {
    urls: options.urls,
    package_name: options.packageName ?? null,
    engine: options.engine,
    priority: options.priority,
    format_id: options.formatId ?? null,
    extension: options.extension ?? null,
    audio_format: options.audioFormat ?? null,
  });
}

export function extractDownloadLinks(text: string): Promise<string[]> {
  return invoke<string[]>("extract_download_links", { text });
}

export function suggestDownloadFilename(url: string): Promise<string> {
  return invoke<string>("suggest_download_filename", { url });
}

export function suggestDownloadPackageName(url: string, filename: string): Promise<string> {
  return invoke<string>("suggest_download_package_name", { url, filename });
}

export function normalizeDownloadUrl(url: string): Promise<string> {
  return invoke<string>("normalize_download_url", { url });
}

export function isDuplicateDownloadUrl(url: string, existingUrls: string[]): Promise<boolean> {
  return invoke<boolean>("is_duplicate_download_url", { url, existing_urls: existingUrls });
}

export function matchDownloadCategory(
  filename: string,
  url?: string,
  pageUrl?: string,
): Promise<string | null> {
  return invoke<string | null>("match_download_category", {
    filename,
    url: url ?? null,
    page_url: pageUrl ?? null,
  });
}

export function addTorrentDownload(
  torrentPath: string,
  selectedFileIndexes?: number[],
  engineKind?: "aria2" | "native_torrent",
): Promise<Download> {
  return invoke<Download>("add_torrent_download", {
    torrent_path: torrentPath,
    selected_file_indexes: selectedFileIndexes ?? null,
    engine_kind: engineKind ?? null,
  });
}

export function inspectTorrentFile(torrentPath: string): Promise<TorrentInspection> {
  return invoke<TorrentInspection>("inspect_torrent_file", { torrent_path: torrentPath });
}

export function addMediaDownload(
  url: string,
  formatId?: string,
  extension?: string | null,
  audioFormat?: "mp3" | "m4a",
): Promise<Download> {
  return invoke<Download>("add_media_download", {
    url,
    format_id: formatId ?? null,
    extension: extension ?? null,
    audio_format: audioFormat ?? null,
  });
}

export function discoverYtDlpFormats(url: string): Promise<YtDlpFormat[]> {
  return invoke<YtDlpFormat[]>("discover_ytdlp_formats", { url });
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

export function moveDownloadUpInQueue(downloadId: string): Promise<void> {
  return invoke<void>("move_download_up_in_queue", { download_id: downloadId });
}

export function moveDownloadDownInQueue(downloadId: string): Promise<void> {
  return invoke<void>("move_download_down_in_queue", { download_id: downloadId });
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

export function pauseQueueDispatch(): Promise<QueueSnapshot> {
  return invoke<QueueSnapshot>("pause_queue_dispatch");
}

export function resumeQueueDispatch(): Promise<QueueSnapshot> {
  return invoke<QueueSnapshot>("resume_queue_dispatch");
}

export function findDuplicateDownload(url: string): Promise<DuplicateDownloadMatch | null> {
  return invoke<DuplicateDownloadMatch | null>("find_duplicate_download", { url });
}

export function getTorrentTelemetry(downloadId: string): Promise<TorrentTelemetry | null> {
  return invoke<TorrentTelemetry | null>("get_torrent_telemetry", { id: downloadId });
}

export function getTorrentDownloadMetadata(
  downloadId: string,
): Promise<TorrentDownloadMetadata | null> {
  return invoke<TorrentDownloadMetadata | null>("get_torrent_download_metadata", {
    id: downloadId,
  });
}

export function selectTorrentFiles(downloadId: string, fileIndexes: number[]): Promise<void> {
  return invoke<void>("select_torrent_files", { id: downloadId, file_indexes: fileIndexes });
}

export function setTorrentFileSelected(
  downloadId: string,
  fileIndex: number,
  selected: boolean,
): Promise<void> {
  return invoke<void>("set_torrent_file_selected", {
    id: downloadId,
    file_index: fileIndex,
    selected,
  });
}

export function setTorrentSelectionAll(
  downloadId: string,
  selected: boolean,
): Promise<void> {
  return invoke<void>("set_torrent_selection_all", { id: downloadId, selected });
}

export function setTorrentDirectorySelected(
  downloadId: string,
  directory: string,
  selected: boolean,
): Promise<void> {
  return invoke<void>("set_torrent_directory_selected", {
    id: downloadId,
    directory,
    selected,
  });
}

export function stopTorrentSeeding(downloadId: string): Promise<void> {
  return invoke<void>("stop_torrent_seeding", { id: downloadId });
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
