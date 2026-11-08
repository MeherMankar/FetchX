/**
 * Media Interception — network-level HLS/video/stream detection.
 *
 * Ports the best patterns from:
 * - AB Download Manager: webRequest.onResponseStarted for m3u8 + video MIME,
 *   per-tab MediaRegistry, HLS encryption check
 * - Motrix: bounded request-header cache (512 entries, 30s TTL),
 *   isNoisySegment filter, CAPTURED_REQUEST_HEADERS
 *
 * This module is imported by the background service worker. It registers
 * webRequest listeners that detect media responses and tracks them per-tab.
 * The popup queries get_media_for_tab to show detected streams.
 */

// ── Types ────────────────────────────────────────────────────────

export type MediaKind = "hls" | "dash" | "video" | "audio";

export interface DetectedStream {
  kind: MediaKind;
  url: string;
  pageUrl: string;
  tabId: number;
  headers: Record<string, string>;
  suggestedFilename: string;
  contentLength: number | null;
  mimeType: string | null;
  detectedAt: number;
}

// ── Constants ─────────────────────────────────────────────────────

/** Headers to capture from webRequest for passing to FetchX download engine. */
const CAPTURED_REQUEST_HEADERS = [
  "Cookie",
  "Referer",
  "User-Agent",
  "Origin",
  "Accept",
  "Accept-Language",
  "X-Requested-With",
] as const;

/** Max entries in the request-header correlation cache. */
const MAX_PENDING_HEADERS = 512;

/** TTL for cached request headers (milliseconds). */
const HEADERS_TTL_MS = 30_000;

/** Max detected streams per tab (prevents runaway memory). */
const MAX_STREAMS_PER_TAB = 64;

// ── Segment flood prevention ─────────────────────────────────────

const SEGMENT_EXTENSIONS = new Set([".ts", ".m4s", ".cmfv", ".cmsv"]);
const SEGMENT_PATH_PATTERNS = [
  /\/seg-\d+/i,
  /\/chunk-\d+/i,
  /\/fragment\d+/i,
  /\/hls\/\d+(\.ts)?$/i,
];

function isNoisySegment(url: string): boolean {
  const path = url.split("?")[0].toLowerCase();
  for (const ext of SEGMENT_EXTENSIONS) {
    if (path.endsWith(ext)) return true;
  }
  return SEGMENT_PATH_PATTERNS.some((p) => p.test(url));
}

// ── URL/MIME classification ───────────────────────────────────────

const HLS_EXTENSIONS = [".m3u8", ".m3u"];
const DASH_EXTENSIONS = [".mpd"];
const VIDEO_MIME_PREFIXES = ["video/"];
const AUDIO_MIME_PREFIXES = ["audio/"];

function classifyByUrl(url: string): MediaKind | null {
  const path = url.split("?")[0].toLowerCase();
  if (HLS_EXTENSIONS.some((ext) => path.endsWith(ext))) return "hls";
  if (DASH_EXTENSIONS.some((ext) => path.endsWith(ext))) return "dash";
  // Common video file extensions (but not segments)
  const ext = path.substring(path.lastIndexOf(".") + 1);
  if (["mp4", "webm", "mkv", "avi", "mov", "flv", "m4v", "3gp", "wmv"].includes(ext)) {
    return "video";
  }
  if (["mp3", "aac", "ogg", "flac", "wav", "m4a", "opus"].includes(ext)) {
    return "audio";
  }
  return null;
}

function classifyByMime(mimeType: string | null | undefined): MediaKind | null {
  if (!mimeType) return null;
  const lower = mimeType.toLowerCase().split(";")[0].trim();
  if (lower === "application/x-mpegurl" || lower === "application/vnd.apple.mpegurl") return "hls";
  if (lower === "application/dash+xml") return "dash";
  if (VIDEO_MIME_PREFIXES.some((p) => lower.startsWith(p))) return "video";
  if (AUDIO_MIME_PREFIXES.some((p) => lower.startsWith(p))) return "audio";
  return null;
}

function classify(
  url: string,
  mimeType: string | null | undefined,
): MediaKind | null {
  return classifyByMime(mimeType) ?? classifyByUrl(url);
}

// ── Suggested filename ─────────────────────────────────────────────

function suggestFilename(url: string, kind: MediaKind): string {
  const raw = url.split("?")[0].split("/").pop()?.trim();
  if (raw && raw.length > 0) {
    const ext = raw.substring(raw.lastIndexOf(".") + 1).toLowerCase();
    if (ext.length > 0 && ext.length <= 5) return raw;
  }
  const fallbackExt =
    kind === "hls" ? "m3u8" : kind === "dash" ? "mpd" : kind === "audio" ? "mp3" : "mp4";
  return `media.${fallbackExt}`;
}

// ── Bounded request-header cache ──────────────────────────────────

interface PendingHeaderEntry {
  tabId: number;
  url: string;
  capturedAt: number;
  headers: Record<string, string>;
}

const pendingHeaders = new Map<string, PendingHeaderEntry>();

function cacheRequestHeaders(
  requestId: string,
  tabId: number,
  url: string,
  requestHeaders: { name: string; value?: string }[] | undefined,
): void {
  // Evict expired entries if at capacity
  if (pendingHeaders.size >= MAX_PENDING_HEADERS) {
    evictExpiredHeaders();
    if (pendingHeaders.size >= MAX_PENDING_HEADERS) {
      // Drop oldest entry
      const oldest = pendingHeaders.values().next().value;
      if (oldest) pendingHeaders.delete(oldest.url);
    }
  }

  const headers: Record<string, string> = {};
  if (requestHeaders) {
    for (const h of requestHeaders) {
      if (h.value && CAPTURED_REQUEST_HEADERS.includes(h.name as typeof CAPTURED_REQUEST_HEADERS[number])) {
        headers[h.name] = h.value;
      }
    }
  }

  pendingHeaders.set(requestId, {
    tabId,
    url,
    capturedAt: Date.now(),
    headers,
  });
}

function evictExpiredHeaders(): void {
  const now = Date.now();
  for (const [key, entry] of pendingHeaders) {
    if (now - entry.capturedAt > HEADERS_TTL_MS) {
      pendingHeaders.delete(key);
    }
  }
}

function popHeaders(requestId: string): PendingHeaderEntry | undefined {
  return pendingHeaders.get(requestId);
}

// ── Per-tab stream registry ────────────────────────────────────────

const mediaByTab = new Map<number, DetectedStream[]>();

function addStreamToTab(tabId: number, stream: DetectedStream): boolean {
  let list = mediaByTab.get(tabId);
  if (!list) {
    list = [];
    mediaByTab.set(tabId, list);
  }
  // Skip if already detected (same URL)
  if (list.some((s) => s.url === stream.url)) return false;
  if (list.length >= MAX_STREAMS_PER_TAB) {
    list.shift(); // drop oldest
  }
  list.push(stream);
  return true;
}

export function getMediaForTab(tabId: number): DetectedStream[] {
  return mediaByTab.get(tabId) ?? [];
}

export function clearMediaForTab(tabId: number): void {
  mediaByTab.delete(tabId);
}

export function clearAllMedia(): void {
  mediaByTab.clear();
}

// ── Known ad/analytics CDN domains to skip ──────────────────────────

const SKIP_DOMAINS = new Set([
  "googlesyndication.com",
  "doubleclick.net",
  "googletagmanager.com",
  "google-analytics.com",
  "adservice.google.com",
  "facebook.com/tr",
  "analytics.twitter.com",
  "scorecardresearch.com",
]);

function shouldSkipDomain(url: string): boolean {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return SKIP_DOMAINS.has(host) || [...SKIP_DOMAINS].some((d) => host.endsWith(`.${d}`));
  } catch {
    return false;
  }
}

// ── webRequest listener registration ───────────────────────────────

interface WebRequestApi {
  onSendHeaders: {
    addListener: (
      callback: (details: {
        requestId: string;
        tabId: number;
        url: string;
        requestHeaders?: { name: string; value?: string }[];
      }) => void,
      filter: { urls: string[]; types: string[] },
      extraInfoSpec: string[],
    ) => void;
  };
  onResponseStarted: {
    addListener: (
      callback: (details: {
        requestId: string;
        tabId: number;
        url: string;
        statusCode: number;
        responseHeaders?: { name: string; value?: string }[];
      }) => void,
      filter: { urls: string[]; types: string[] },
      extraInfoSpec: string[],
    ) => void;
  };
}

/**
 * Registers all webRequest listeners for media interception.
 * Call this from the service worker on startup.
 *
 * @param onStreamDetected Callback fired when a new media stream is detected.
 *   Receives the DetectedStream. The caller is responsible for notifying the
 *   popup or forwarding to FetchX.
 */
export function registerMediaInterception(
  webRequest: WebRequestApi,
  onStreamDetected: (stream: DetectedStream) => void,
): void {
  // Phase 1: Capture request headers (Cookie, Referer, etc.) for correlation.
  webRequest.onSendHeaders.addListener(
    (details) => {
      if (details.tabId < 0) return; // not a tab request
      if (shouldSkipDomain(details.url)) return;
      cacheRequestHeaders(
        details.requestId,
        details.tabId,
        details.url,
        details.requestHeaders,
      );
    },
    { urls: ["http://*/*", "https://*/*"], types: ["media", "xmlhttprequest", "other"] },
    ["requestHeaders"],
  );

  // Phase 2a: Watch for HLS/DASH playlist responses by URL pattern.
  webRequest.onResponseStarted.addListener(
    (details) => {
      if (details.tabId < 0 || details.statusCode >= 400) return;
      if (shouldSkipDomain(details.url)) return;
      if (isNoisySegment(details.url)) return;

      const kind = classifyByUrl(details.url);
      if (!kind || (kind !== "hls" && kind !== "dash")) return;

      const headerEntry = popHeaders(details.requestId);
      const responseMime = getHeader(details.responseHeaders, "content-type");
      const contentLength = parseContentLength(
        getHeader(details.responseHeaders, "content-length"),
      );

      const stream: DetectedStream = {
        kind,
        url: details.url,
        pageUrl: "", // filled by caller if needed
        tabId: details.tabId,
        headers: headerEntry?.headers ?? {},
        suggestedFilename: suggestFilename(details.url, kind),
        contentLength,
        mimeType: responseMime,
        detectedAt: Date.now(),
      };

      if (addStreamToTab(details.tabId, stream)) {
        onStreamDetected(stream);
      }
    },
    {
      urls: [
        "http://*/*.m3u8",
        "https://*/*.m3u8",
        "http://*/*.m3u8?*",
        "https://*/*.m3u8?*",
        "http://*/*.m3u",
        "https://*/*.m3u",
        "http://*/*.m3u?*",
        "https://*/*.m3u?*",
        "http://*/*.mpd",
        "https://*/*.mpd",
        "http://*/*.mpd?*",
        "https://*/*.mpd?*",
      ],
      types: ["media", "xmlhttprequest", "other"],
    },
    ["responseHeaders"],
  );

  // Phase 2b: Watch for direct video/audio responses by Content-Type.
  webRequest.onResponseStarted.addListener(
    (details) => {
      if (details.tabId < 0 || details.statusCode >= 400) return;
      if (shouldSkipDomain(details.url)) return;
      if (isNoisySegment(details.url)) return;

      const mimeType = getHeader(details.responseHeaders, "content-type");
      const kind = classifyByMime(mimeType);
      if (!kind) return;
      // HLS/DASH already caught by URL filter above; skip duplicates
      if (kind === "hls" || kind === "dash") return;

      const headerEntry = popHeaders(details.requestId);
      const contentLength = parseContentLength(
        getHeader(details.responseHeaders, "content-length"),
      );

      // Skip tiny responses (likely thumbnails, audio beacons, etc.)
      if (contentLength !== null && contentLength < 100_000) return;

      const stream: DetectedStream = {
        kind,
        url: details.url,
        pageUrl: "",
        tabId: details.tabId,
        headers: headerEntry?.headers ?? {},
        suggestedFilename: suggestFilename(details.url, kind),
        contentLength,
        mimeType,
        detectedAt: Date.now(),
      };

      if (addStreamToTab(details.tabId, stream)) {
        onStreamDetected(stream);
      }
    },
    { urls: ["http://*/*", "https://*/*"], types: ["media"] },
    ["responseHeaders"],
  );
}

// ── Helpers ────────────────────────────────────────────────────────

function getHeader(
  headers: { name: string; value?: string }[] | undefined,
  name: string,
): string | null {
  if (!headers) return null;
  const lower = name.toLowerCase();
  for (const h of headers) {
    if (h.name.toLowerCase() === lower && h.value) return h.value;
  }
  return null;
}

function parseContentLength(value: string | null): number | null {
  if (!value) return null;
  const n = parseInt(value, 10);
  return Number.isFinite(n) && n > 0 ? n : null;
}
