// Video URL patterns matching VideoInterceptor.kt
const VIDEO_EXTENSIONS = new Set([
  "mp4", "m4v", "mkv", "webm", "avi", "mov", "flv",
  "ts", "m3u8", "mpd", "3gp", "wmv"
]);

// Path/domain hints that indicate a video stream URL
const VIDEO_PATH_HINTS = [
  "/videoplayback",   // YouTube CDN
  ".m3u8",            // HLS manifest/segment
  ".mpd",             // DASH manifest
  "/hls/",            // HLS stream path
  "/dash/",           // DASH stream path
  "/stream/",         // generic stream
  "/manifest",        // manifest endpoint
  "googlevideo.com",  // YouTube CDN
  "fbcdn.net",        // Facebook CDN
  "vimeocdn.com",     // Vimeo CDN
  "xhamster",         // XHamster CDN (catches cdn-?.xhamster.com/videos/...)
  "/videos/mp4/",     // XHamster direct MP4 path
  "cdn-",             // common CDN subdomain prefix with media files
];

function isVideoUrl(url) {
  const lower = url.toLowerCase();
  const pathPart = lower.split("?")[0];
  const lastDot = pathPart.lastIndexOf(".");
  const ext = lastDot >= 0 ? pathPart.slice(lastDot + 1) : "";
  if (VIDEO_EXTENSIONS.has(ext)) return true;
  return VIDEO_PATH_HINTS.some(hint => lower.includes(hint));
}

// Already-reported URLs — avoid flooding native side with duplicates
const reported = new Set();

browser.webRequest.onBeforeRequest.addListener(
  (details) => {
    const url = details.url;
    if (!reported.has(url) && isVideoUrl(url)) {
      reported.add(url);
      browser.runtime.sendNativeMessage("browser", { type: "video", url: url })
        .catch(() => {});
    }
    // Never block — always return undefined
  },
  // Listen to ALL request types so we catch XHR, fetch, media elements, etc.
  { urls: ["<all_urls>"] }
);
