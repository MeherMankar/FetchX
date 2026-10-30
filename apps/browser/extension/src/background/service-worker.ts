/**
 * FetchX Extension — Background Service Worker
 *
 * Features (matches Motrix extension):
 * 1. Context menu on links and page
 * 2. Auto-intercept browser downloads (configurable size threshold)
 * 3. Message handler for popup (add, get_recent, get_settings, set_settings)
 * 4. Download task cache for popup's task list
 */

import type { BrowserErrorCode, BrowserResponse } from "../types/protocol";
import {
  NativeHostError,
  callTargetRpc,
  hasEmbeddedCredentials,
  isHttpUrl,
} from "../shared/native";
import { PROTOCOL_VERSION } from "../types/protocol";
import type { RpcRequest, RpcTarget } from "../types/protocol";

// ────────────────────────────────────────────────────────────
// Settings (persisted in chrome.storage.local)
// ────────────────────────────────────────────────────────────

export interface ExtensionSettings {
  /** Intercept browser downloads automatically */
  autoIntercept: boolean;
  /** Minimum file size in MB to intercept (0 = all) */
  minSizeMb: number;
  /** Domains to never intercept (user-managed blocklist) */
  blockedDomains: string[];
  /** Show a notification on successful intercept */
  showNotifications: boolean;
}

const DEFAULT_SETTINGS: ExtensionSettings = {
  autoIntercept: true,
  minSizeMb: 0,
  blockedDomains: [],
  showNotifications: true,
};

export type BrowserTarget =
  | {
    id: string;
    name: string;
    kind: "local";
  }
  | {
    id: string;
    name: string;
    kind: "remote";
    endpoint: string;
    credential_id: string;
  };

const LOCAL_TARGET: BrowserTarget = {
  id: "local-windows",
  name: "This PC",
  kind: "local",
};

async function getSelectedTarget(): Promise<BrowserTarget> {
  const stored = await chrome.storage.local.get(["browserTargets", "selectedTargetId"]);
  const targets = Array.isArray(stored.browserTargets)
    ? stored.browserTargets.filter(isBrowserTarget)
    : [];
  const selected = targets.find((target) => target.id === stored.selectedTargetId);
  return selected ?? LOCAL_TARGET;
}

async function getSettings(): Promise<ExtensionSettings> {
  const stored = await chrome.storage.local.get("settings");
  return { ...DEFAULT_SETTINGS, ...(stored.settings ?? {}) } as ExtensionSettings;
}

async function saveSettings(settings: Partial<ExtensionSettings>): Promise<ExtensionSettings> {
  const current = await getSettings();
  const updated = { ...current, ...settings };
  await chrome.storage.local.set({ settings: updated });
  return updated;
}

// ────────────────────────────────────────────────────────────
// In-memory task cache (updated after every successful add)
// ────────────────────────────────────────────────────────────

interface CachedDownload {
  id: string;
  filename: string;
  url: string;
  status: string;
  downloaded_bytes: number;
  total_bytes: number | null;
  percentage_basis_points: number | null;
  speed_bytes_per_second: number | null;
}

let taskCache: CachedDownload[] = [];

// ────────────────────────────────────────────────────────────
// Context menus
// ────────────────────────────────────────────────────────────

const MENU_LINK = "md-link";
const MENU_PAGE = "md-page";

chrome.runtime.onInstalled.addListener(() => {
  void chrome.storage.local.get("browserTargets").then((stored) => {
    if (!Array.isArray(stored.browserTargets)) {
      return chrome.storage.local.set({ browserTargets: [] });
    }
  });
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_LINK,
      title: "Download link with FetchX",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: MENU_PAGE,
      title: "Download page URL with FetchX",
      contexts: ["page"],
    });
  });
});

chrome.contextMenus.onClicked.addListener((info, _tab) => {
  const url = info.linkUrl ?? info.pageUrl;
  if (!url) return;
  void handleAddDownload(url).then((result) => {
    const settings = getSettings();
    void settings.then((s) => {
      if (!s.showNotifications) return;
      if (result.ok) {
        notify("Download added", `"${hostnameOf(url)}" is being downloaded by FetchX.`);
      } else if (!result.ok) {
        notify("FetchX", result.error.message);
      }
    });
  });
});

// ────────────────────────────────────────────────────────────
// Auto-intercept browser downloads
// ────────────────────────────────────────────────────────────

chrome.downloads.onCreated.addListener((item) => {
  void (async () => {
    const settings = await getSettings();
    if (!settings.autoIntercept) return;

    // Skip if already handled by our own native host submission
    if (item.url.startsWith("blob:") || item.url.startsWith("data:")) return;
    if (!isHttpUrl(item.url)) return;
    if (hasEmbeddedCredentials(item.url)) return;

    // Check domain blocklist
    try {
      const hostname = new URL(item.url).hostname;
      if (settings.blockedDomains.some((d) => hostname.endsWith(d))) return;
    } catch { return; }

    // Check size threshold
    const fileSizeBytes = item.fileSize ?? 0;
    const minBytes = settings.minSizeMb * 1024 * 1024;
    if (minBytes > 0 && fileSizeBytes > 0 && fileSizeBytes < minBytes) return;

    // Cancel the browser's own download and hand off to FetchX
    chrome.downloads.cancel(item.id);
    chrome.downloads.erase({ id: item.id });

    const result = await handleAddDownload(item.url);
    if (result.ok && settings.showNotifications) {
      const name = item.filename?.split(/[/\\]/).pop() ?? hostnameOf(item.url);
      notify("Intercepted", `"${name}" handed to FetchX.`);
    }
  })();
});

// ────────────────────────────────────────────────────────────
// Message handler (popup ↔ background)
// ────────────────────────────────────────────────────────────

interface Msg { type: string; [k: string]: unknown }

chrome.runtime.onMessage.addListener(
  (message: unknown, _sender, sendResponse): boolean => {
    if (!isMsg(message)) { sendResponse(null); return false; }

    switch (message.type) {
      case "add_download":
        if (typeof message.url === "string") {
          void handleAddDownload(message.url).then(sendResponse);
          return true;
        }
        break;

      case "add_batch": {
        const urls = Array.isArray(message.urls) ? message.urls as string[] : [];
        void handleBatchDownload(urls).then(sendResponse);
        return true;
      }

      case "get_tasks":
        sendResponse(taskCache);
        return false;

      case "get_settings":
        void getSettings().then(sendResponse);
        return true;

      case "save_settings":
        void saveSettings(message.settings as Partial<ExtensionSettings>).then(sendResponse);
        return true;

      case "refresh_tasks":
        void refreshTasks().then(() => sendResponse(taskCache));
        return true;

      case "target_changed":
        void refreshTasks().then(() => sendResponse({ ok: true }));
        return true;
    }

    sendResponse(null);
    return false;
  },
);

// ────────────────────────────────────────────────────────────
// Core download submission
// ────────────────────────────────────────────────────────────

async function handleAddDownload(url: string): Promise<BrowserResponse> {
  if (!isHttpUrl(url)) return err("unsupported_scheme", "Only HTTP and HTTPS URLs are supported.");
  if (hasEmbeddedCredentials(url)) return err("credentials_in_url", "URLs with credentials are not accepted.");

  try {
    const target = await getSelectedTarget();
    const rpcRequest: RpcRequest = {
      jsonrpc: "2.0",
      id: crypto.randomUUID(),
      method: "download.create",
      params: { url },
    };
    const response = await callTargetRpc(toRpcTarget(target), rpcRequest);
    if (!response.ok || !("rpc_response" in response)) return response;
    const rpcResponse = response.rpc_response as {
      result?: { id?: unknown };
      error?: { code?: unknown };
    };
    if (typeof rpcResponse.error === "object" && rpcResponse.error !== null) {
      return err("download_rejected", "FetchX rejected the download.");
    }
    const downloadId = rpcResponse.result?.id;
    if (typeof downloadId !== "string") {
      return err("internal_error", "FetchX returned an invalid RPC response.");
    }
    void refreshTasks();
    return {
      version: PROTOCOL_VERSION,
      ok: true,
      download_id: downloadId,
    };
  } catch (e) {
    const msg = e instanceof NativeHostError ? e.message : "Could not reach FetchX. Is it running?";
    return err("desktop_unavailable", msg);
  }
}

async function handleBatchDownload(urls: string[]): Promise<{ added: number; failed: number }> {
  let added = 0;
  let failed = 0;
  for (const url of urls) {
    const result = await handleAddDownload(url);
    result.ok ? added++ : failed++;
  }
  return { added, failed };
}

// ────────────────────────────────────────────────────────────
// Task cache refresh (asks desktop for latest download list)
// ────────────────────────────────────────────────────────────

async function refreshTasks(): Promise<void> {
  try {
    const target = await getSelectedTarget();
    const response = await callTargetRpc(toRpcTarget(target), {
      jsonrpc: "2.0",
      id: crypto.randomUUID(),
      method: "download.list",
    });
    if (!response.ok || !("rpc_response" in response)) return;
    const rpcResponse = response.rpc_response as {
      result?: { downloads?: CachedDownload[] };
    };
    const downloads = rpcResponse.result?.downloads;
    if (Array.isArray(downloads)) {
      taskCache = downloads.slice(0, 20);
    }
  } catch { /* optional */ }
}

function toRpcTarget(target: BrowserTarget): RpcTarget {
  if (target.kind === "local") return { kind: "local" };
  return {
    kind: "remote",
    endpoint: target.endpoint,
    credential_id: target.credential_id,
  };
}

function isBrowserTarget(value: unknown): value is BrowserTarget {
  if (typeof value !== "object" || value === null) return false;
  const target = value as Record<string, unknown>;
  return typeof target.id === "string"
    && typeof target.name === "string"
    && (target.kind === "local"
      || (target.kind === "remote"
        && typeof target.endpoint === "string"
        && typeof target.credential_id === "string"));
}

// ────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────

function isMsg(v: unknown): v is Msg {
  return typeof v === "object" && v !== null && typeof (v as Msg).type === "string";
}

function err(code: BrowserErrorCode, message: string): BrowserResponse {
  return { version: PROTOCOL_VERSION, ok: false, error: { code, message } };
}

function hostnameOf(url: string): string {
  try { return new URL(url).hostname; } catch { return url.slice(0, 40); }
}

function notify(title: string, message: string): void {
  try {
    chrome.notifications?.create({
      type: "basic",
      iconUrl: "icons/icon48.png",
      title,
      message,
    });
  } catch { /* cosmetic */ }
}
