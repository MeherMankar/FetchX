import { invoke } from "@tauri-apps/api/core";

import type {
  DiagnosticResult,
  EngineConfigDto,
  EngineStatus,
  HelperInstallProgress,
  HelperStatus,
  ManagedHelperId,
  SaveEngineConfigRequest,
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

export function getHelperStatus(): Promise<HelperStatus[]> {
  return invoke<HelperStatus[]>("get_helper_status");
}

export function installHelper(helper: ManagedHelperId): Promise<void> {
  return invoke<void>("install_helper", { helper });
}

export function cancelHelperInstall(helper: ManagedHelperId): Promise<boolean> {
  return invoke<boolean>("cancel_helper_install", { helper });
}

export type { HelperInstallProgress };
