/**
 * FetchX Extension — Background Service Worker
 *
 * Features (matches Motrix extension):
 * 1. Context menu on links and page
 * 2. Auto-intercept browser downloads (configurable size threshold)
 * 3. Message handler for popup (add, get_recent, get_settings, set_settings)
 * 4. Download task cache for popup's task list
 */

import type { BrowserErrorCode, BrowserResponse, SendDrmKeysRequest } from "../types/protocol";
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
// WidevineProxy2 — content script registration
// ────────────────────────────────────────────────────────────

const WVP2_SCRIPT_CONFIGS = [
  {
    id: "WVP2_ISOLATED",
    matches: ["<all_urls>"],
    js: ["widevine/library/isolated/bundle.min.js"],
    runAt: "document_start" as const,
    world: "ISOLATED" as chrome.scripting.ExecutionWorld,
    allFrames: true,
    matchOriginAsFallback: true,
    persistAcrossSessions: true,
  },
  {
    id: "WVP2_MAIN",
    matches: ["<all_urls>"],
    js: ["widevine/library/main/bundle.min.js"],
    runAt: "document_start" as const,
    world: "MAIN" as chrome.scripting.ExecutionWorld,
    allFrames: true,
    matchOriginAsFallback: true,
    persistAcrossSessions: true,
  },
];

// ────────────────────────────────────────────────────────────
// Widevine L3 (helloir3) — content script registration
// ────────────────────────────────────────────────────────────

const WVP2_L3_SCRIPT_CONFIGS = [
  {
    id: "WVP2_L3_BRIDGE",
    matches: ["<all_urls>"],
    js: ["widevine/l3/content_script.js"],
    runAt: "document_start" as const,
    world: "ISOLATED" as chrome.scripting.ExecutionWorld,
    allFrames: true,
    matchOriginAsFallback: true,
    persistAcrossSessions: true,
  },
  {
    id: "WVP2_L3_MAIN",
    matches: ["<all_urls>"],
    js: [
      "widevine/l3/eme_interception.js",
      "widevine/l3/content_key_decryption.js",
      "widevine/l3/lib/cryptojs-aes_0.2.0.min.js",
      "widevine/l3/lib/pbf.3.0.5.min.js",
    ],
    runAt: "document_start" as const,
    world: "MAIN" as chrome.scripting.ExecutionWorld,
    allFrames: true,
    matchOriginAsFallback: true,
    persistAcrossSessions: true,
  },
];

let wvp2L3RegistrationPromise: Promise<void> | null = null;

async function wvp2L3RegisterScripts(): Promise<void> {
  const existing = await chrome.scripting.getRegisteredContentScripts();
  const existingIds = new Set(existing.map((s) => s.id));
  const toRegister = WVP2_L3_SCRIPT_CONFIGS.filter((cfg) => !existingIds.has(cfg.id));
  const toUpdate = WVP2_L3_SCRIPT_CONFIGS.filter((cfg) => existingIds.has(cfg.id));
  if (toRegister.length) await chrome.scripting.registerContentScripts(toRegister);
  if (toUpdate.length) await chrome.scripting.updateContentScripts(toUpdate);
}

function wvp2L3EnsureScriptsRegistered(): Promise<void> {
  if (wvp2L3RegistrationPromise) return wvp2L3RegistrationPromise;
  wvp2L3RegistrationPromise = (async () => {
    try {
      await wvp2L3RegisterScripts();
    } finally {
      wvp2L3RegistrationPromise = null;
    }
  })();
  return wvp2L3RegistrationPromise;
}

let wvp2RegistrationPromise: Promise<void> | null = null;

async function wvp2GetEnabledState(): Promise<boolean> {
  try {
    const result = await chrome.storage.sync.get(["enabled", "selected"]);
    return !!(result.enabled ?? true) && !!result.selected;
  } catch {
    return false;
  }
}

async function wvp2RegisterScripts(): Promise<void> {
  const existing = await chrome.scripting.getRegisteredContentScripts();
  const existingIds = new Set(existing.map((s) => s.id));
  const toRegister = WVP2_SCRIPT_CONFIGS.filter((cfg) => !existingIds.has(cfg.id));
  const toUpdate = WVP2_SCRIPT_CONFIGS.filter((cfg) => existingIds.has(cfg.id));
  if (toRegister.length) await chrome.scripting.registerContentScripts(toRegister);
  if (toUpdate.length) await chrome.scripting.updateContentScripts(toUpdate);
}

async function wvp2UnregisterScripts(): Promise<void> {
  const existing = await chrome.scripting.getRegisteredContentScripts();
  const existingIds = new Set(existing.map((s) => s.id));
  const ids = WVP2_SCRIPT_CONFIGS.map((cfg) => cfg.id).filter((id) => existingIds.has(id));
  if (ids.length) await chrome.scripting.unregisterContentScripts({ ids });
}

function wvp2EnsureScriptsRegistered(): Promise<void> {
  if (wvp2RegistrationPromise) return wvp2RegistrationPromise;
  wvp2RegistrationPromise = (async () => {
    try {
      const enabled = await wvp2GetEnabledState();
      if (enabled) {
        await wvp2RegisterScripts();
      } else {
        await wvp2UnregisterScripts();
      }
    } finally {
      wvp2RegistrationPromise = null;
    }
  })();
  return wvp2RegistrationPromise;
}

// ────────────────────────────────────────────────────────────
// WidevineProxy2 — remote CDM session management
// ────────────────────────────────────────────────────────────

interface RemoteCdmDevice {
  host: string;
  secret: string;
  device_name: string;
}

interface RemoteKey {
  k: string;
  kid: string;
}

class RemoteCdm {
  sessionId: string;
  host: string;
  secret: string;
  deviceName: string;

  constructor(sessionId: string, host: string, secret: string, deviceName: string) {
    this.sessionId = sessionId;
    this.host = host;
    this.secret = secret;
    this.deviceName = deviceName;
  }

  static async open(host: string, secret: string, deviceName: string): Promise<RemoteCdm> {
    const trimmedHost = host.replace(/\/+$/, "");
    const resp = await fetch(`${trimmedHost}/${deviceName}/open`, {
      method: "GET",
      headers: { "X-Secret-Key": secret },
    });
    if (resp.status !== 200) throw new Error("Remote CDM API (open) response code does not indicate success");
    const json = await resp.json() as { data: { session_id: string } };
    return new RemoteCdm(json.data.session_id, trimmedHost, secret, deviceName);
  }

  async setServiceCertificate(certificateB64: string): Promise<void> {
    const resp = await fetch(`${this.host}/${this.deviceName}/set_service_certificate`, {
      method: "POST",
      headers: { "content-type": "application/json", "X-Secret-Key": this.secret },
      body: JSON.stringify({ session_id: this.sessionId, certificate: certificateB64 }),
    });
    if (resp.status !== 200) throw new Error("Remote CDM API (set_service_certificate) response code does not indicate success");
  }

  async getLicenseChallenge(initDataB64: string, privacyMode: boolean): Promise<string> {
    const resp = await fetch(`${this.host}/${this.deviceName}/get_license_challenge/STREAMING`, {
      method: "POST",
      headers: { "content-type": "application/json", "X-Secret-Key": this.secret },
      body: JSON.stringify({ session_id: this.sessionId, init_data: initDataB64, privacy_mode: privacyMode }),
    });
    if (resp.status !== 200) throw new Error("Remote CDM API (get_license_challenge) response code does not indicate success");
    const json = await resp.json() as { data: { challenge_b64: string } };
    return json.data.challenge_b64;
  }

  async parseLicense(licenseB64: string): Promise<RemoteKey[]> {
    const parseResp = await fetch(`${this.host}/${this.deviceName}/parse_license`, {
      method: "POST",
      headers: { "content-type": "application/json", "X-Secret-Key": this.secret },
      body: JSON.stringify({ session_id: this.sessionId, license_message: licenseB64 }),
    });
    if (parseResp.status !== 200) throw new Error("Remote CDM API (parse_license) response code does not indicate success");
    const keysResp = await fetch(`${this.host}/${this.deviceName}/get_keys/CONTENT`, {
      method: "POST",
      headers: { "content-type": "application/json", "X-Secret-Key": this.secret },
      body: JSON.stringify({ session_id: this.sessionId }),
    });
    if (keysResp.status !== 200) throw new Error("Remote CDM API (get_keys) response code does not indicate success");
    const keysJson = await keysResp.json() as { data: { keys: Array<{ key: string; key_id: string }> } };
    await fetch(`${this.host}/${this.deviceName}/close/${this.sessionId}`, {
      method: "GET",
      headers: { "X-Secret-Key": this.secret },
    });
    return keysJson.data.keys.map(({ key, key_id }) => ({ k: key, kid: key_id }));
  }
}

const remoteCdmSessions = new Map<string, RemoteCdm>();

async function getRemoteDevice(): Promise<RemoteCdmDevice> {
  const result = await chrome.storage.sync.get(["selected_remote_cdm"]);
  const selectedRemoteCdm = result.selected_remote_cdm as string | undefined;
  if (!selectedRemoteCdm) throw new Error("No remote CDM selected");
  const deviceObj = await chrome.storage.sync.get([selectedRemoteCdm]);
  const device = deviceObj[selectedRemoteCdm] as RemoteCdmDevice | undefined;
  if (!device) throw new Error(`Selected remote CDM "${selectedRemoteCdm}" not found in storage`);
  return device;
}

function getRemoteCdmSession(sessionId: string): RemoteCdm {
  const cdm = remoteCdmSessions.get(sessionId);
  if (!cdm) throw new Error(`Unknown remote CDM session "${sessionId}"`);
  return cdm;
}

type RemotePayload = Record<string, unknown>;

async function handleRemoteMessage(type: string, payload: RemotePayload): Promise<unknown> {
  switch (type) {
    case "REMOTE_OPEN": {
      const device = await getRemoteDevice();
      const cdm = await RemoteCdm.open(device.host, device.secret, device.device_name);
      remoteCdmSessions.set(cdm.sessionId, cdm);
      return cdm.sessionId;
    }
    case "REMOTE_SET_SERVICE_CERTIFICATE": {
      const cdm = getRemoteCdmSession(payload.sessionId as string);
      await cdm.setServiceCertificate(payload.certificate as string);
      return true;
    }
    case "REMOTE_GET_CHALLENGE": {
      const cdm = getRemoteCdmSession(payload.sessionId as string);
      return cdm.getLicenseChallenge(payload.initData as string, payload.privacyMode as boolean);
    }
    case "REMOTE_PARSE": {
      const cdm = getRemoteCdmSession(payload.sessionId as string);
      try {
        return await cdm.parseLicense(payload.license as string);
      } finally {
        remoteCdmSessions.delete(payload.sessionId as string);
      }
    }
    default:
      throw new Error(`Unknown remote message type "${type}"`);
  }
}

// ────────────────────────────────────────────────────────────
// WidevineProxy2 — icon state helpers
// ────────────────────────────────────────────────────────────

let wvp2IconTimeoutId: ReturnType<typeof setTimeout> | null = null;

async function wvp2SetIcon(type: "red" | "normal"): Promise<void> {
  try {
    const p = type === "red" ? "-red" : "";
    await chrome.action.setIcon({
      path: {
        16: `widevine/images/icon-16${p}.png`,
        32: `widevine/images/icon-32${p}.png`,
        64: `widevine/images/icon-64${p}.png`,
        128: `widevine/images/icon-128${p}.png`,
      },
    });
  } catch { /* cosmetic */ }
}

// ────────────────────────────────────────────────────────────
// WidevineProxy2 — picker helper
// ────────────────────────────────────────────────────────────

function wvp2OpenPicker(path: string, mobile: boolean): void {
  if (mobile) {
    void chrome.tabs.create({ url: chrome.runtime.getURL(path) });
  } else {
    void chrome.windows.create({ url: path, type: "popup", width: 320, height: 180 });
  }
}

// ────────────────────────────────────────────────────────────
// WidevineProxy2 — storage change watcher
// ────────────────────────────────────────────────────────────

chrome.storage.onChanged.addListener((changes, areaName) => {
  if (areaName === "local") {
    const added = Object.values(changes).some(
      ({ oldValue, newValue }) => oldValue === undefined && newValue !== undefined,
    );
    if (added) {
      void wvp2SetIcon("red");
      if (wvp2IconTimeoutId) clearTimeout(wvp2IconTimeoutId);
      wvp2IconTimeoutId = setTimeout(() => {
        wvp2IconTimeoutId = null;
        void wvp2SetIcon("normal");
      }, 60 * 1000);

      // Forward newly captured DRM keys to FetchX
      for (const [psshData, { oldValue, newValue }] of Object.entries(changes)) {
        if (oldValue === undefined && typeof newValue === "object" && newValue !== null
          && ("keys" in newValue || "type" in newValue)) {
          void submitDrmKeysToFetchX(psshData, newValue as WvpStoredEntry);
        }
      }
    }
  } else if (areaName === "sync") {
    void wvp2EnsureScriptsRegistered();
  }
});

// ────────────────────────────────────────────────────────────
// WidevineProxy2 — webRequest manifest header relay
// ────────────────────────────────────────────────────────────

if (webRequestApi) {
  const webRequestFull = webRequestApi as unknown as {
    onBeforeSendHeaders?: {
      addListener: (
        cb: (details: { tabId: number; method: string; url: string; requestHeaders: Array<{ name: string; value: string }> }) => void,
        filter: object,
        extra: string[],
      ) => void;
    };
  };
  webRequestFull.onBeforeSendHeaders?.addListener(
    async (details) => {
      if (details.tabId === -1) return;
      if (details.method !== "GET") return;
      const headers = (details.requestHeaders ?? [])
        .filter(
          (item) =>
            !item.name.startsWith("sec-ch-ua") &&
            !item.name.startsWith("Sec-Fetch") &&
            !item.name.startsWith("Accept-") &&
            !item.name.startsWith("Host") &&
            item.name !== "Connection",
        )
        .reduce<Record<string, string>>((acc, item) => {
          acc[item.name] = item.value;
          return acc;
        }, {});
      try {
        await chrome.tabs.sendMessage(details.tabId, {
          type: "MANIFEST_HEADERS",
          payload: { url: details.url, headers },
        });
      } catch {
        // tab may not have the content script injected; ignore
      }
    },
    { urls: ["<all_urls>"] },
    ["requestHeaders", "extraHeaders"],
  );
}

chrome.runtime.onSuspend.addListener(() => {
  void wvp2SetIcon("normal");
});

// Bootstrap WVP2 script registration after service worker becomes active
setTimeout(() => {
  void wvp2EnsureScriptsRegistered();
}, 1000);

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
  void wvp2EnsureScriptsRegistered();
  void wvp2L3EnsureScriptsRegistered();
});

browserApi.runtime.onStartup.addListener(() => {
  void wvp2EnsureScriptsRegistered();
  void wvp2L3EnsureScriptsRegistered();
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

      // ── WidevineProxy2 messages ──────────────────────────────
      case "IS_OUTDATED":
        sendResponse(false); // version check handled separately
        return false;

      case "report_drm_keys": {
        // Content script / page reports extracted DRM keys; store and forward to FetchX.
        const psshData = typeof message.pssh_data === "string" ? message.pssh_data : "";
        if (psshData) {
          const entry: WvpStoredEntry = {
            url: typeof message.url === "string" ? message.url : undefined,
            keys: Array.isArray(message.keys) ? message.keys as WvpStoredEntry["keys"] : [],
            timestamp: typeof message.timestamp === "number" ? message.timestamp : Date.now(),
          };
          void browserApi.storage.local.set({ [psshData]: entry });
          void submitDrmKeysToFetchX(psshData, entry);
        }
        sendResponse({ ok: true });
        return false;
      }

      case "WVP2_L3_KEYS": {
        // Widevine L3 (helloir3) content script reports extracted key pairs.
        const l3Keys = Array.isArray(message.keys)
          ? (message.keys as Array<{ kid: string; key: string }>)
          : [];
        const l3Url = typeof message.url === "string" ? message.url : undefined;
        void (async () => {
          for (const { kid, key } of l3Keys) {
            if (!kid || !key) continue;
            const storageKey = `l3_keys_${kid}`;
            const existing = await chrome.storage.local.get(storageKey);
            if (existing[storageKey]) continue; // deduplicated by kid
            const entry = { kid, key, url: l3Url, timestamp: Date.now() };
            await chrome.storage.local.set({ [storageKey]: entry });
            void submitDrmKeysToFetchX(`l3:${kid}`, {
              url: l3Url,
              keys: [{ kid, k: key }],
              timestamp: entry.timestamp,
            });
          }
        })();
        sendResponse({ ok: true });
        return false;
      }

      case "OPEN_PICKER_WVD":
        wvp2OpenPicker("widevine/picker/wvd/filePicker.html", false);
        break;
      case "OPEN_PICKER_WVD_MOBILE":
        wvp2OpenPicker("widevine/picker/wvd/filePicker.html", true);
        break;
      case "OPEN_PICKER_REMOTE":
        wvp2OpenPicker("widevine/picker/remote/filePicker.html", false);
        break;
      case "OPEN_PICKER_REMOTE_MOBILE":
        wvp2OpenPicker("widevine/picker/remote/filePicker.html", true);
        break;
    }

    // Handle REMOTE_* messages for WidevineProxy2 Remote CDM
    if (typeof message.type === "string" && message.type.startsWith("REMOTE_")) {
      void handleRemoteMessage(message.type, (message.payload ?? {}) as RemotePayload)
        .then((data) => sendResponse({ data }))
        .catch((error: unknown) => sendResponse({ error: error instanceof Error ? error.message : String(error) }));
      return true;
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
// WidevineProxy2 — DRM key forwarding to FetchX
// ────────────────────────────────────────────────────────────

/** Shape of entries WVP2 stores in chrome.storage.local keyed by PSSH/URL identifier. */
interface WvpStoredEntry {
  url?: string;
  type?: string;
  keys?: Array<string | { kid: string; k: string }>;
  timestamp?: number;
}

async function submitDrmKeysToFetchX(psshData: string, entry: WvpStoredEntry): Promise<void> {
  let target: BrowserTarget;
  try {
    target = await getSelectedTarget();
  } catch {
    return;
  }

  const keys = (entry.keys ?? []).map((k) =>
    typeof k === "string"
      ? { kid: k.split(":")[0] ?? "", k: k.split(":")[1] ?? "" }
      : { kid: k.kid, k: k.k },
  );

  const request: SendDrmKeysRequest = {
    version: PROTOCOL_VERSION,
    type: "send_drm_keys",
    pssh_data: psshData,
    keys,
    url: entry.url,
    timestamp: entry.timestamp,
  };

  const rpcRequest: RpcRequest = {
    jsonrpc: "2.0",
    id: crypto.randomUUID(),
    method: "drm.storeKeys",
    params: request as unknown as Record<string, unknown>,
  };

  try {
    await callTargetRpc(toRpcTarget(target), rpcRequest);
  } catch {
    // Best-effort: key forwarding failures are non-fatal
  }
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
