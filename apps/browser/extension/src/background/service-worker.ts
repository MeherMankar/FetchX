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
import { browserApi } from "../shared/browser-api";
import { PROTOCOL_VERSION } from "../types/protocol";
import type { RpcRequest, RpcTarget } from "../types/protocol";
import {
  claimDownloadEvent,
  filenameFromBrowserPath,
  handoffBrowserDownload,
  isBlockedDomain,
  submitDownloadWithResolvedFilename,
  type DownloadSubmission,
} from "./download-interception";
import {
  registerMediaInterception,
  getMediaForTab,
  clearMediaForTab,
  type DetectedStream,
} from "./media-interception";

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
  const stored = await browserApi.storage.local.get(["browserTargets", "selectedTargetId"]);
  const targets = Array.isArray(stored.browserTargets)
    ? stored.browserTargets.filter(isBrowserTarget)
    : [];
  const selected = targets.find((target) => target.id === stored.selectedTargetId);
  return selected ?? LOCAL_TARGET;
}

async function getSettings(): Promise<ExtensionSettings> {
  const stored = await browserApi.storage.local.get("settings");
  return { ...DEFAULT_SETTINGS, ...(stored.settings ?? {}) } as ExtensionSettings;
}

async function saveSettings(settings: Partial<ExtensionSettings>): Promise<ExtensionSettings> {
  const current = await getSettings();
  const updated = { ...current, ...settings };
  await browserApi.storage.local.set({ settings: updated });
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
// Network-level media interception (HLS, DASH, video/audio)
// ────────────────────────────────────────────────────────────

// Register webRequest listeners for media detection.
// The onStreamDetected callback fires when a new stream is found on a tab.
const webRequestApi = (browserApi as unknown as {
  webRequest?: {
    onSendHeaders: { addListener: (cb: Function, filter: object, extra: string[]) => void };
    onResponseStarted: { addListener: (cb: Function, filter: object, extra: string[]) => void };
  };
}).webRequest;

if (webRequestApi) {
  registerMediaInterception(
    webRequestApi,
    (stream: DetectedStream) => {
      // Optionally notify — we keep it silent unless user has notifications on
      void getSettings().then((s) => {
        if (s.showNotifications) {
          const label =
            stream.kind === "hls" ? "HLS stream"
              : stream.kind === "dash" ? "DASH stream"
              : stream.kind === "audio" ? "Audio"
              : "Video";
          notify(`Media detected on tab ${stream.tabId}`, `${label}: ${stream.suggestedFilename}`);
        }
      });
    },
  );
}

// Clear per-tab media when the tab navigates or is closed.
browserApi.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.url) {
    clearMediaForTab(tabId);
  }
});

browserApi.tabs.onRemoved.addListener((tabId) => {
  clearMediaForTab(tabId);
});

// ────────────────────────────────────────────────────────────
// Context menus
// ────────────────────────────────────────────────────────────

const MENU_LINK = "md-link";
const MENU_PAGE = "md-page";

browserApi.runtime.onInstalled.addListener(() => {
  void browserApi.storage.local.get("browserTargets").then((stored) => {
    if (!Array.isArray(stored.browserTargets)) {
      return browserApi.storage.local.set({ browserTargets: [] });
    }
  });
  void browserApi.contextMenus.removeAll().then(() => {
    browserApi.contextMenus.create({
      id: MENU_LINK,
      title: "Download link with FetchX",
      contexts: ["link"],
    });
    browserApi.contextMenus.create({
      id: MENU_PAGE,
      title: "Download page URL with FetchX",
      contexts: ["page"],
    });
  });
});

browserApi.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl ?? info.pageUrl;
  if (!url) return;
  const context = await captureDownloadContext(url, info.pageUrl ?? tab?.url);
  void handleAddDownload(url, context).then((result) => {
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

browserApi.downloads.onCreated.addListener((item) => {
  void interceptBrowserDownload(item).catch((error: unknown) => {
    console.error(
      "Could not process browser download event",
      error instanceof Error ? error.message : "unknown error",
    );
  });
});

interface DownloadContext {
  referer?: string;
  userAgent?: string;
  cookies?: string;
}

async function captureDownloadContext(url: string, referrer?: string): Promise<DownloadContext> {
  let cookies: string | undefined;
  try {
    const cookieApi = (browserApi as unknown as { cookies?: { getAll: (details: { url: string }) => Promise<Array<{ name: string; value: string }>> } }).cookies;
    if (cookieApi && typeof cookieApi.getAll === "function") {
      const found = await cookieApi.getAll({ url });
      if (Array.isArray(found) && found.length > 0) {
        cookies = found.map((c) => `${c.name}=${c.value}`).join("; ");
      }
    }
  } catch {
    // Ignore cookie retrieval failures
  }

  return {
    referer: referrer || undefined,
    userAgent: typeof navigator !== "undefined" ? navigator.userAgent : undefined,
    cookies,
  };
}

interface BrowserDownloadItem {
  id: number;
  url: string;
  filename: string;
  fileSize: number;
  referrer?: string;
}

async function interceptBrowserDownload(item: BrowserDownloadItem): Promise<void> {
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

    const context = await captureDownloadContext(item.url, item.referrer);

    const result = await handoffBrowserDownload(item.id, {
      pause: (downloadId) => browserApi.downloads.pause(downloadId),
      resume: (downloadId) => browserApi.downloads.resume(downloadId),
      cancel: (downloadId) => browserApi.downloads.cancel(downloadId),
      submit: () => submitDownloadWithResolvedFilename(item, {
          search: async (downloadId) => browserApi.downloads.search({ id: downloadId }),
          addChangedListener: (listener) => browserApi.downloads.onChanged.addListener(listener),
          removeChangedListener: (listener) => browserApi.downloads.onChanged.removeListener(listener),
        }, (url, filename) => submitBrowserDownload(url, filename, context)),
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
        const stored = await browserApi.storage.local.get(CLAIMED_DOWNLOAD_IDS_KEY);
        return stored[CLAIMED_DOWNLOAD_IDS_KEY];
      },
      save: async (ids) => {
        await browserApi.storage.local.set({ [CLAIMED_DOWNLOAD_IDS_KEY]: ids });
      },
    },
    MAX_CLAIMED_DOWNLOAD_IDS,
  );
}

// ────────────────────────────────────────────────────────────
// Message handler (popup ↔ background)
// ────────────────────────────────────────────────────────────

interface Msg { type: string; [k: string]: unknown }

browserApi.runtime.onMessage.addListener(
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

      case "get_media_for_tab":
        if (typeof message.tabId === "number") {
          sendResponse(getMediaForTab(message.tabId));
        } else {
          sendResponse([]);
        }
        return false;

      case "clear_media_for_tab":
        if (typeof message.tabId === "number") {
          clearMediaForTab(message.tabId);
        }
        sendResponse({ ok: true });
        return false;
    }

    sendResponse(null);
    return false;
  },
);

// ────────────────────────────────────────────────────────────
// Core download submission
// ────────────────────────────────────────────────────────────

async function handleAddDownload(url: string, context?: DownloadContext): Promise<BrowserResponse> {
  if (!isHttpUrl(url)) {
    return err("unsupported_scheme", "Only HTTP and HTTPS URLs are accepted.");
  }
  if (hasEmbeddedCredentials(url)) {
    return err("credentials_in_url", "URLs containing credentials are not accepted.");
  }

  const submission = await submitBrowserDownload(url, undefined, context);
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
  context?: DownloadContext,
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

  const params: Record<string, unknown> = { url };
  if (filename) params.filename = filename;
  if (context?.referer) params.referer = context.referer;
  if (context?.userAgent) params.user_agent = context.userAgent;
  if (context?.cookies) params.cookies = context.cookies;

  const rpcRequest: RpcRequest = {
    jsonrpc: "2.0",
    id: crypto.randomUUID(),
    method: "download.create",
    params,
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
      (filename || context?.referer || context?.cookies)
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
    void browserApi.notifications?.create({
      type: "basic",
      iconUrl: "icons/icon48.png",
      title,
      message,
    });
  } catch { /* cosmetic */ }
}
