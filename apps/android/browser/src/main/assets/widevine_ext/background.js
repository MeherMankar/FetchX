"use strict";

// WVP2 background logic adapted for GeckoView built-in WebExtension.
// When the isolated-world content script stores extracted DRM keys via
// chrome.storage.local, we forward them to FetchX through native messaging
// so the Android WidevineManager / bridge can store them for use by the
// download engine.

chrome.storage.onChanged.addListener(function (changes, areaName) {
  if (areaName !== "local") return;

  var entries = Object.entries(changes);
  for (var i = 0; i < entries.length; i++) {
    var key = entries[i][0];
    var change = entries[i][1];
    // Only react to new entries (no oldValue), and only objects with a "keys" field.
    if (change.oldValue !== undefined) continue;
    var entry = change.newValue;
    if (!entry || typeof entry !== "object" || !entry.keys) continue;

    // Build the native message payload matching the BrowserRequest schema
    // used by the fetchx-browser-protocol ReportDrmKeys variant.
    var keys = (entry.keys || []).map(function (k) {
      if (typeof k === "string") {
        var parts = k.split(":");
        return { key_id: parts[0] || "", key: parts[1] || "" };
      }
      return { key_id: k.kid || k.key_id || "", key: k.k || k.key || "" };
    });

    var message = {
      version: 1,
      type: "report_drm_keys",
      url: entry.url || key,
      pssh: entry.pssh_data || null,
      keys: keys,
    };

    // Forward to FetchX native host — best-effort, errors are silently ignored
    // so a failed delivery never breaks the page.
    browser.runtime.sendNativeMessage("browser", message).catch(function () {});
  }
});
