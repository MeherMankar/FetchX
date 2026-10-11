/**
 * Content script — runs on every page.
 *
 * Responsibilities:
 * 1. Respond to "scan_resources" messages from the popup.
 *    Scans the DOM for video, audio, and downloadable links.
 * 2. Never modifies the page DOM or intercepts user events.
 */

interface ResourceItem {
  url: string;
  type: "video" | "audio" | "image" | "file";
  label: string;
}

function getExtension(url: string): string {
  try {
    const path = new URL(url).pathname;
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    return ext;
  } catch {
    return "";
  }
}

const VIDEO_EXTS = new Set(["mp4", "webm", "mkv", "avi", "mov", "flv", "m4v", "ts", "m3u8"]);
const AUDIO_EXTS = new Set(["mp3", "aac", "ogg", "flac", "wav", "m4a", "opus", "wma"]);
const IMAGE_EXTS = new Set(["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "avif"]);
const FILE_EXTS  = new Set(["zip", "rar", "7z", "tar", "gz", "iso", "exe", "msi", "dmg",
                             "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
                             "apk", "deb", "rpm", "pkg"]);

function classifyUrl(url: string): ResourceItem["type"] | null {
  const ext = getExtension(url);
  if (VIDEO_EXTS.has(ext)) return "video";
  if (AUDIO_EXTS.has(ext)) return "audio";
  if (IMAGE_EXTS.has(ext)) return "image";
  if (FILE_EXTS.has(ext))  return "file";
  return null;
}

function labelFor(url: string): string {
  try {
    const u = new URL(url);
    const parts = u.pathname.split("/").filter(Boolean);
    return decodeURIComponent(parts[parts.length - 1] || u.hostname);
  } catch {
    return url.slice(0, 60);
  }
}

function isHttpUrl(url: string): boolean {
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

function scanResources(): ResourceItem[] {
  const seen = new Set<string>();
  const results: ResourceItem[] = [];

  function add(url: string, forcedType?: ResourceItem["type"]) {
    if (!url || !isHttpUrl(url) || seen.has(url)) return;
    seen.add(url);
    const type = forcedType ?? classifyUrl(url);
    if (!type) return;
    results.push({ url, type, label: labelFor(url) });
  }

  // <video src> and <video><source src>
  document.querySelectorAll<HTMLVideoElement>("video[src]").forEach(v => add(v.src, "video"));
  document.querySelectorAll<HTMLSourceElement>("video source[src]").forEach(s => add(s.src, "video"));

  // <audio>
  document.querySelectorAll<HTMLAudioElement>("audio[src]").forEach(a => add(a.src, "audio"));
  document.querySelectorAll<HTMLSourceElement>("audio source[src]").forEach(s => add(s.src, "audio"));

  // <img> — only external, sized images (skip tiny icons)
  document.querySelectorAll<HTMLImageElement>("img[src]").forEach(img => {
    if (img.naturalWidth >= 100 && img.naturalHeight >= 100) add(img.src, "image");
  });

  // <a href> — only links with downloadable extensions
  document.querySelectorAll<HTMLAnchorElement>("a[href]").forEach(a => {
    const href = a.href;
    if (href && classifyUrl(href)) add(href);
  });

  // Sort: files and video first
  const order: Record<ResourceItem["type"], number> = { file: 0, video: 1, audio: 2, image: 3 };
  results.sort((a, b) => order[a.type] - order[b.type]);

  return results;
}

// Listen for messages from the popup / background
chrome.runtime.onMessage.addListener(
  (message: unknown, _sender, sendResponse): boolean => {
    if (
      typeof message === "object" &&
      message !== null &&
      (message as { type?: string }).type === "scan_resources"
    ) {
      sendResponse(scanResources());
    }
    return false;
  },
);
