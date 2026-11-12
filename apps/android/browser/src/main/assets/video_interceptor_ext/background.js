// ─── FetchX Video Interceptor — background.js ─────────────────────────────────
// Strategy A: intercept webRequest for known video patterns (catches default quality)
// Strategy B: inject content script to read all quality URLs from page JS data

// ── Patterns ──────────────────────────────────────────────────────────────────

const VIDEO_EXTENSIONS = new Set(["mp4", "m4v", "mkv", "webm", "mov", "flv", "avi", "3gp", "wmv"]);
const MANIFEST_EXTENSIONS = new Set(["m3u8", "mpd"]);

const VIDEO_PATH_PATTERNS = [
  /\/videoplayback/i,
  /googlevideo\.com/i,
  /vimeocdn\.com\/video\//i,
  /fbcdn\.net\/v\//i,
  /xhpingcdn\.com\/v\//i,   // XHamster video CDN
  /xhvid\.net\/v\//i,
];

const SKIP_PATTERNS = [
  /(?:^|[\/_-])previews?(?:[\/_.-]|$)/i,
  /\binit[-_v]/i,    // init-v1-a1.mp4 etc.
  /_TPL_/i,          // template placeholders
  /\/seg-\d+/i,      // individual HLS transport segments
  /\/chunk-\d+/i,    // DASH chunks
  /\/frag\(/i,
];

const SKIP_DOMAINS = [
  "doubleclick.net", "googlesyndication.com", "google-analytics.com",
  "googletagmanager.com", "amazon-adsystem.com", "criteo.com",
  "scorecardresearch.com", "static.xhpingcdn.com", "xhpingcdn.com/static",
];

function shouldSkip(url) {
  const lower = url.toLowerCase();
  if (SKIP_DOMAINS.some(d => lower.includes(d))) return true;
  if (SKIP_PATTERNS.some(re => re.test(url))) return true;
  return false;
}

function isVideoUrl(url) {
  if (shouldSkip(url)) return false;
  const lower = url.toLowerCase();
  const path = lower.split(/[?#]/)[0].replace(/\/+$/, "");
  const ext = path.slice(path.lastIndexOf(".") + 1);
  if (VIDEO_EXTENSIONS.has(ext)) return true;
  if (MANIFEST_EXTENSIONS.has(ext)) return true;
  return VIDEO_PATH_PATTERNS.some(re => re.test(url));
}

function extractQuality(url, label) {
  const labeled = (label || "").match(/(\d{3,4})p/i);
  if (labeled) return `${labeled[1]}p`;
  const explicit = url.match(/(\d{3,4})(?:p|m)\b/i);
  if (explicit) return `${explicit[1]}p`;
  const height = url.match(/[?&](?:quality|resolution|height|video_height)=["']?(\d{3,4})/i);
  return height ? `${height[1]}p` : null;
}

function suggestFilename(url) {
  const path = url.split(/[?#]/)[0].replace(/\/+$/, "");
  const name = path.split("/").pop() || "video";
  const ext = name.split(".").pop().toLowerCase();
  const ALL = new Set(["mp4","m4v","mkv","webm","mov","flv","avi","3gp","wmv","m3u8","mpd"]);
  if (ALL.has(ext)) {
    const quality = extractQuality(url);
    return quality ? `${quality}.${ext}` : name;
  }
  return name + ".mp4";
}

// ── Strategy A: webRequest interception ───────────────────────────────────────

const reported = new Set();

function videoIdentity(url) {
  try {
    const parsed = new URL(url);
    return parsed.origin + parsed.pathname.replace(/\/+$/, "");
  } catch (_) {
    return url.split(/[?#]/)[0].replace(/\/+$/, "");
  }
}

function sendVideo(url, tabId) {
  const identity = videoIdentity(url);
  if (reported.has(identity)) return;
  reported.add(identity);
  browser.runtime.sendNativeMessage("browser", {
    type: "video",
    url: url,
    filename: suggestFilename(url),
    quality: extractQuality(url),
  }).catch(() => {});
}

browser.webRequest.onBeforeRequest.addListener(
  (details) => {
    if (isVideoUrl(details.url)) sendVideo(details.url, details.tabId);
  },
  { urls: ["<all_urls>"], types: ["xmlhttprequest", "media", "other"] }
);

// ── Strategy B: inject content script to read ALL quality levels ───────────────
// XHamster stores all quality URLs in window.initObj.videoSources or similar.
// We inject a script after page load to extract them all at once.

const XHAMSTER_PATTERN = /xhamster\d*\.com/i;

browser.webNavigation.onCompleted.addListener((details) => {
  if (details.frameId !== 0) return; // top frame only
  const url = details.url;
  if (!XHAMSTER_PATTERN.test(url)) return;
  if (!url.includes("/videos/")) return;

  // Execute in page context to read XHamster's embedded video data
  browser.tabs.executeScript(details.tabId, {
    code: `
      (function() {
        try {
          // XHamster stores quality data in window.initObj
          var sources = [];
          var obj = window.initObj || {};
          var vs = obj.videoSources || obj.sources || obj.videoModel && obj.videoModel.sources || [];
          if (vs && vs.forEach) {
            vs.forEach(function(s) {
              if (s.url && s.url.indexOf('.m3u8') >= 0) {
                sources.push({ url: s.url, quality: s.quality || s.label || '' });
              }
            });
          }
          // Also check xhjs embed data
          var scripts = document.querySelectorAll('script:not([src])');
          for (var i = 0; i < scripts.length; i++) {
            var text = scripts[i].textContent;
            var matches = text.match(/"url"\\s*:\\s*"(https[^"]+\\.m3u8[^"]*)"/g);
            if (matches) {
              matches.forEach(function(m) {
                var u = m.match(/"(https[^"]+\\.m3u8[^"]*)"/);
                if (u) sources.push({ url: u[1].replace(/\\\\/g, ''), quality: '' });
              });
            }
          }
          return JSON.stringify(sources);
        } catch(e) { return '[]'; }
      })()
    `,
  }).then(function(results) {
    if (!results || !results[0]) return;
    try {
      var sources = JSON.parse(results[0]);
      sources.forEach(function(s) {
        var identity = s.url && videoIdentity(s.url);
        if (s.url && !reported.has(identity)) {
          reported.add(identity);
          browser.runtime.sendNativeMessage("browser", {
            type: "video",
            url: s.url,
            filename: s.quality ? s.quality + ".m3u8" : suggestFilename(s.url),
            quality: s.quality || extractQuality(s.url),
          }).catch(function() {});
        }
      });
    } catch(e) {}
  }).catch(function() {});
}, { url: [{ hostContains: "xhamster" }] });

browser.webNavigation.onCompleted.addListener((details) => {
  if (details.frameId !== 0 || !/^https?:/i.test(details.url)) return;

  browser.tabs.executeScript(details.tabId, {
    code: String.raw`
      (function() {
        try {
          var sources = [];
          var seen = new Set();
          var pageVideoId = location.pathname.match(/\/videos?\/(\d+)/i);
          pageVideoId = pageVideoId && pageVideoId[1];

          function qualityFor(value, label) {
            var labeled = (label || "").match(/(\d{3,4})p/i);
            if (labeled) return labeled[1] + "p";
            var explicit = value.match(/(\d{3,4})(?:p|m)\b/i);
            if (explicit) return explicit[1] + "p";
            var height = value.match(/[?&](?:quality|resolution|height|video_height)=["']?(\d{3,4})/i);
            return height ? height[1] + "p" : "";
          }

          function addSource(value, label) {
            if (typeof value !== "string" || !value) return;
            value = value.replace(/\\u002f/gi, "/").replace(/\\\//g, "/");
            var url;
            try { url = new URL(value, location.href); } catch (_) { return; }
            if (url.protocol !== "http:" && url.protocol !== "https:") return;
            if (!/\.(?:mp4|m4v|webm|mkv|mov|m3u8|mpd)\/?$/i.test(url.pathname)) return;
            if (/preview/i.test(url.pathname)) return;
            if (pageVideoId && /\/get_file\//i.test(url.pathname) &&
                !new RegExp("/" + pageVideoId + "(?:[_./-]|$)").test(url.pathname)) return;
            if (seen.has(url.href)) return;
            seen.add(url.href);
            var quality = qualityFor(url.href, label);
            var extension = url.pathname.replace(/\/+$/, "").split(".").pop();
            sources.push({
              url: url.href,
              quality: quality || "",
              filename: (quality || "video") + "." + extension
            });
          }

          document.querySelectorAll("video").forEach(function(video) {
            addSource(video.currentSrc, video.getAttribute("label"));
            addSource(video.src, video.getAttribute("label"));
            video.querySelectorAll("source[src]").forEach(function(source) {
              addSource(
                source.src,
                source.getAttribute("label") ||
                  source.getAttribute("data-quality") ||
                  source.getAttribute("title"),
              );
            });
          });

          var mediaPattern = /(?:https?:\/\/|\/\/|\/)[^"'\s<>]*?\.(?:mp4|m4v|webm|mkv|mov|m3u8|mpd)(?:[?#][^"'\s<>]*)?\/?/gi;
          document.querySelectorAll("script:not([src])").forEach(function(script) {
            var text = script.textContent.replace(/\\u002f/gi, "/").replace(/\\\//g, "/");
            var match;
            while ((match = mediaPattern.exec(text)) !== null && sources.length < 32) {
              addSource(match[0]);
            }
          });
          return JSON.stringify(sources.slice(0, 32));
        } catch (_) {
          return "[]";
        }
      })()
    `,
  }).then(function(results) {
    if (!results || !results[0]) return;
    try {
      JSON.parse(results[0]).forEach(function(source) {
        var identity = source.url && videoIdentity(source.url);
        if (!source.url || reported.has(identity)) return;
        reported.add(identity);
        browser.runtime.sendNativeMessage("browser", {
          type: "video",
          url: source.url,
          filename: source.filename || suggestFilename(source.url),
          quality: source.quality || extractQuality(source.url),
        }).catch(function() {});
      });
    } catch (_) {}
  }).catch(function() {});
}, { url: [{ schemes: ["http", "https"] }] });
