import { invoke } from "@tauri-apps/api/core";

import type {
  DiagnosticResult,
  EngineConfigDto,
  EngineStatus,
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

export function testAria2Config(): Promise<DiagnosticResult> {
  return invoke<DiagnosticResult>("test_aria2_config");
}

export function testYtDlpConfig(): Promise<DiagnosticResult> {
  return invoke<DiagnosticResult>("test_ytdlp_config");
}

export function getEngineStatus(): Promise<EngineStatus> {
  return invoke<EngineStatus>("get_engine_status");
}
