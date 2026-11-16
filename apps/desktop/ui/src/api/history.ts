import { invoke } from "@tauri-apps/api/core";
import type { Download } from "./types";

export function listCompletedDownloads(limit?: number): Promise<Download[]> {
  return invoke<Download[]>("list_completed_downloads", { limit: limit ?? 200 });
}

export function clearDownloadHistory(): Promise<number> {
  return invoke<number>("clear_download_history");
}

export function setDownloadBandwidthLimit(
  downloadId: string,
  bytesPerSecond: number | null,
): Promise<void> {
  return invoke<void>("set_download_bandwidth_limit", {
    download_id: downloadId,
    bytes_per_second: bytesPerSecond,
  });
}

export function provideDownloadCredentials(
  downloadId: string,
  username: string,
  password: string,
): Promise<Download> {
  return invoke<Download>("provide_download_credentials", {
    download_id: downloadId,
    username,
    password,
  });
}

export function getPostQueueAction(): Promise<string> {
  return invoke<string>("get_post_queue_action");
}
