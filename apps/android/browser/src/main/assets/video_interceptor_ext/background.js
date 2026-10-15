// Video URL patterns matching VideoInterceptor.kt
const VIDEO_EXTENSIONS = new Set([
  "mp4", "m4v", "mkv", "webm", "avi", "mov", "flv",
  "ts", "m3u8", "mpd", "3gp", "wmv"
]);

const VIDEO_PATH_HINTS = [
  "/videoplayback", "/manifest", ".m3u8", ".mpd",
  "googlevideo.com", "fbcdn.net", "vimeocdn.com"
];

function isVideoUrl(url) {
  const lower = url.toLowerCase();
  // Strip query string before checking extension
  const pathPart = lower.split("?")[0];
  const lastDot = pathPart.lastIndexOf(".");
  const ext = lastDot >= 0 ? pathPart.slice(lastDot + 1) : "";
  if (VIDEO_EXTENSIONS.has(ext)) return true;
  return VIDEO_PATH_HINTS.some(hint => lower.includes(hint));
}

browser.webRequest.onBeforeRequest.addListener(
  (details) => {
    const url = details.url;
    if (isVideoUrl(url)) {
      // Fire-and-forget — ignore result and errors
      browser.runtime.sendNativeMessage("browser", { type: "video", url: url })
        .catch(() => {});
    }
    // Always return undefined — never block requests
  },
  { urls: ["<all_urls>"], types: ["xmlhttprequest", "media"] }
);
