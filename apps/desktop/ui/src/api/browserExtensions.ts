import { invoke } from "@tauri-apps/api/core";

import type { BrowserExtensionStatus } from "./types";

export function getBrowserExtensionStatus(): Promise<BrowserExtensionStatus> {
  return invoke<BrowserExtensionStatus>("get_browser_extension_status");
}

export function openBrowserExtensionInstallPage(browser: string): Promise<void> {
  return invoke<void>("open_browser_extension_install_page", { browser });
}

export function launchBrowserWithExtension(browser: string): Promise<void> {
  return invoke<void>("launch_browser_with_extension", { browser });
}
