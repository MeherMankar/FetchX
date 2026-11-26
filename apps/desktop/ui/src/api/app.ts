import { invoke } from "@tauri-apps/api/core";

import type { AppInfo, UpdateCheckResult } from "./types";

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

export function checkForUpdate(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_for_update");
}

export function cancelPostQueueAction(): Promise<void> {
  return invoke<void>("cancel_post_queue_action");
}
