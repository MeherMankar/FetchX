import { invoke } from "@tauri-apps/api/core";

import type {
  DiagnosticResult,
  EngineConfigDto,
  EngineStatus,
  FolderCategoriesConfigDto,
  PostDownloadActionDto,
  PostQueueActionDto,
  SaveEngineConfigRequest,
  WebhookConfigDto,
} from "./types";

export function getEngineConfig(): Promise<EngineConfigDto> {
  return invoke<EngineConfigDto>("get_engine_config");
}

export function saveEngineConfig(
  request: SaveEngineConfigRequest,
): Promise<EngineConfigDto> {
  return invoke<EngineConfigDto>("save_engine_config", { request });
}

export function resetEngineConfig(): Promise<EngineConfigDto> {
  return invoke<EngineConfigDto>("reset_engine_config");
}

export function testYtDlpConfig(): Promise<DiagnosticResult> {
  return invoke<DiagnosticResult>("test_ytdlp_config");
}

export function getEngineStatus(): Promise<EngineStatus> {
  return invoke<EngineStatus>("get_engine_status");
}

export function setNotificationConfig(
  osNotificationsEnabled: boolean,
): Promise<void> {
  return invoke<void>("set_notification_config", {
    osNotificationsEnabled,
  });
}

export function getPostDownloadAction(): Promise<PostDownloadActionDto> {
  return invoke<PostDownloadActionDto>("get_post_download_action");
}

export function setPostDownloadAction(
  action: PostDownloadActionDto,
): Promise<void> {
  return invoke<void>("set_post_download_action", { action });
}

export function getFolderCategoriesConfig(): Promise<FolderCategoriesConfigDto> {
  return invoke<FolderCategoriesConfigDto>("get_folder_categories_config");
}

export function setFolderCategoriesConfig(
  config: FolderCategoriesConfigDto,
): Promise<void> {
  return invoke<void>("set_folder_categories_config", { config });
}

export function exportSettings(): Promise<void> {
  return invoke<void>("export_settings");
}

export function importSettings(): Promise<EngineConfigDto> {
  return invoke<EngineConfigDto>("import_settings");
}

export function getWebhookConfig(): Promise<WebhookConfigDto> {
  return invoke<WebhookConfigDto>("get_webhook_config");
}

export function setWebhookConfig(config: WebhookConfigDto): Promise<void> {
  return invoke<void>("set_webhook_config", { config });
}

export function setPostQueueAction(action: PostQueueActionDto): Promise<void> {
  return invoke<void>("set_post_queue_action", { action });
}
