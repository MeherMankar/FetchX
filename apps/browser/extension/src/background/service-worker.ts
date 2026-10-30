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
import {
  claimDownloadEvent,
  filenameFromBrowserPath,
  handoffBrowserDownload,
  isBlockedDomain,
  type DownloadSubmission,
} from "./download-interception";

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
const CLAIMED_DOWNLOAD_IDS_KEY = "interceptedBrowserDownloadIds";
const MAX_CLAIMED_DOWNLOAD_IDS = 512;
const claimingDownloadIds = new Set<number>();

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
  void interceptBrowserDownload(item).catch((error: unknown) => {
    console.error(
      "Could not process browser download event",
      error instanceof Error ? error.message : "unknown error",
    );
  });
});

async function interceptBrowserDownload(item: chrome.downloads.DownloadItem): Promise<void> {
    const settings = await getSettings();
    if (!settings.autoIntercept) return;

    if (!isHttpUrl(item.url)) return;
    if (hasEmbeddedCredentials(item.url)) return;

    const hostname = new URL(item.url).hostname;
    if (isBlockedDomain(hostname, settings.blockedDomains)) return;

    // Check size threshold
    const fileSizeBytes = item.fileSize ?? 0;
    const minBytes = settings.minSizeMb * 1024 * 1024;
    if (minBytes > 0 && fileSizeBytes > 0 && fileSizeBytes < minBytes) return;

    if (!await claimBrowserDownloadEvent(item.id)) return;

    const result = await handoffBrowserDownload(item.id, {
      pause: (downloadId) => chrome.downloads.pause(downloadId),
      resume: (downloadId) => chrome.downloads.resume(downloadId),
      cancel: (downloadId) => chrome.downloads.cancel(downloadId),
      submit: async () => {
        const browserPath = await waitForBrowserDownloadFilename(item.id, item.filename);
        const filename = filenameFromBrowserPath(browserPath);
        if (!filename) {
          return {
            kind: "rejected",
            message: "The browser did not provide a download filename.",
          };
        }
        return submitBrowserDownload(item.url, filename);
      },
    });
    const displayName = filenameFromBrowserPath(item.filename) ?? hostname;

    switch (result.kind) {
      case "not_paused":
        if (settings.showNotifications) {
          notify("FetchX", `"${displayName}" was not intercepted; the browser download continues.`);
        }
        break;
      case "accepted":
        if (result.browserDownloadCancelled) {
          if (settings.showNotifications) {
            notify("Intercepted", `"${displayName}" handed to FetchX.`);
          }
        } else {
          notify(
            "FetchX accepted the download",
            `"${displayName}" may also remain in the browser. Check its Downloads list.`,
          );
        }
        break;
      case "rejected":
        if (result.browserDownloadResumed) {
          if (settings.showNotifications) notify("FetchX", result.message);
        } else {
          notify(
            "FetchX handoff failed",
            `The browser download could not be resumed. Check the browser Downloads list for "${displayName}".`,
          );
        }
        break;
      case "unknown":
        notify(
          "FetchX handoff unconfirmed",
          `"${displayName}" remains paused. Check FetchX before resuming the browser download.`,
        );
        break;
    }
}

async function claimBrowserDownloadEvent(downloadId: number): Promise<boolean> {
  return claimDownloadEvent(
    downloadId,
    claimingDownloadIds,
    {
      load: async () => {
        const stored = await chrome.storage.local.get(CLAIMED_DOWNLOAD_IDS_KEY);
        return stored[CLAIMED_DOWNLOAD_IDS_KEY];
      },
      save: async (ids) => {
        await chrome.storage.local.set({ [CLAIMED_DOWNLOAD_IDS_KEY]: ids });
      },
    },
    MAX_CLAIMED_DOWNLOAD_IDS,
  );
}

async function waitForBrowserDownloadFilename(
  downloadId: number,
  initialFilename: string,
): Promise<string | undefined> {
  if (initialFilename) return initialFilename;
  const current = await chrome.downloads.search({ id: downloadId });
  if (current[0]?.filename) return current[0].filename;

  return new Promise((resolve, reject) => {
    let settled = false;
    let timeoutId: ReturnType<typeof setTimeout>;
    const cleanup = () => {
      clearTimeout(timeoutId);
      chrome.downloads.onChanged.removeListener(onChanged);
    };
    const finish = (filename?: string) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(filename);
    };
    const onChanged = (delta: chrome.downloads.DownloadDelta) => {
      if (delta.id === downloadId && delta.filename?.current) {
        finish(delta.filename.current);
      }
    };

    chrome.downloads.onChanged.addListener(onChanged);
    timeoutId = setTimeout(() => finish(), 5_000);
    void chrome.downloads.search({ id: downloadId }).then(
      (items) => {
        if (items[0]?.filename) finish(items[0].filename);
      },
      (error: unknown) => {
        if (settled) return;
        settled = true;
        cleanup();
        reject(error);
      },
    );
  });
}

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
  if (!isHttpUrl(url)) {
    return err("unsupported_scheme", "Only HTTP and HTTPS URLs are accepted.");
  }
  if (hasEmbeddedCredentials(url)) {
    return err("credentials_in_url", "URLs containing credentials are not accepted.");
  }

  const submission = await submitBrowserDownload(url);
  if (submission.kind === "accepted") {
    void refreshTasks();
    return {
      version: PROTOCOL_VERSION,
      ok: true,
      download_id: submission.downloadId,
    };
  }
  if (submission.kind === "unknown") {
    return err("request_outcome_unknown", submission.message);
  }
  return err("download_rejected", submission.message);
}

async function submitBrowserDownload(
  url: string,
  filename?: string,
): Promise<DownloadSubmission> {
  if (!isHttpUrl(url)) {
    return { kind: "rejected", message: "Only HTTP and HTTPS URLs are accepted." };
  }
  if (hasEmbeddedCredentials(url)) {
    return { kind: "rejected", message: "URLs containing credentials are not accepted." };
  }

  let target: BrowserTarget;
  try {
    target = await getSelectedTarget();
  } catch {
    return { kind: "rejected", message: "The selected FetchX target could not be loaded." };
  }

  const rpcRequest: RpcRequest = {
    jsonrpc: "2.0",
    id: crypto.randomUUID(),
    method: "download.create",
    params: filename ? { url, filename } : { url },
  };

  try {
    const response = await callTargetRpc(toRpcTarget(target), rpcRequest);
    if (!response.ok) {
      return response.error.code === "request_outcome_unknown"
        ? {
          kind: "unknown",
          message: "FetchX may have accepted the download, but confirmation was not received.",
        }
        : { kind: "rejected", message: response.error.message };
    }
    if (!("rpc_response" in response)) {
      return {
        kind: "unknown",
        message: "FetchX may have accepted the download, but returned no confirmation.",
      };
    }

    let rpcResponse = response.rpc_response;
    if (!isMatchingRpcResponse(rpcResponse, rpcRequest.id)) {
      return {
        kind: "unknown",
        message: "FetchX may have accepted the download, but returned an invalid confirmation.",
      };
    }
    if (
      filename
      && isRpcError(rpcResponse)
      && (rpcResponse.error.code === -32600 || rpcResponse.error.code === -32602)
    ) {
      const legacyRequest = { ...rpcRequest, params: { url } };
      const legacyResponse = await callTargetRpc(toRpcTarget(target), legacyRequest);
      if (!legacyResponse.ok) {
        return legacyResponse.error.code === "request_outcome_unknown"
          ? {
            kind: "unknown",
            message: "FetchX may have accepted the download, but confirmation was not received.",
          }
          : { kind: "rejected", message: legacyResponse.error.message };
      }
      if (!("rpc_response" in legacyResponse)) {
        return {
          kind: "unknown",
          message: "FetchX may have accepted the download, but returned no confirmation.",
        };
      }
      rpcResponse = legacyResponse.rpc_response;
      if (!isMatchingRpcResponse(rpcResponse, rpcRequest.id)) {
        return {
          kind: "unknown",
          message: "FetchX may have accepted the download, but returned an invalid confirmation.",
        };
      }
    }

    if (isRpcError(rpcResponse)) {
      if (rpcResponse.error.code === -32011 || rpcResponse.error.code === -32603) {
        return {
          kind: "unknown",
          message: "FetchX may have accepted the download, but confirmation was not received.",
        };
      }
      return { kind: "rejected", message: "FetchX rejected the download request." };
    }

    const downloadId = extractRpcDownloadId(rpcResponse.result);
    if (
      rpcResponse.id !== rpcRequest.id
      || downloadId === undefined
    ) {
      return {
        kind: "unknown",
        message: "FetchX may have accepted the download, but returned an invalid confirmation.",
      };
    }
    return {
      kind: "accepted",
      downloadId,
    };
  } catch (error) {
    if (error instanceof NativeHostError && error.outcomeUnknown) {
      return {
        kind: "unknown",
        message: "FetchX may have accepted the download, but confirmation was not received.",
      };
    }
    return {
      kind: "rejected",
      message: "FetchX could not be reached; the browser download will continue.",
    };
  }
}

function isRpcError(
  response: Record<string, unknown>,
): response is Record<string, unknown> & { error: { code: number } } {
  if (typeof response.error !== "object" || response.error === null) return false;
  return typeof (response.error as Record<string, unknown>).code === "number";
}

function isMatchingRpcResponse(
  response: Record<string, unknown>,
  requestId: string | number,
): boolean {
  return response.jsonrpc === "2.0" && response.id === requestId;
}

function extractRpcDownloadId(result: unknown): string | undefined {
  if (typeof result !== "object" || result === null) return undefined;
  const id = (result as Record<string, unknown>).id;
  return typeof id === "string" && id.length > 0 ? id : undefined;
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
