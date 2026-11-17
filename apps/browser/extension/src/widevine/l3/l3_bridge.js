/**
 * l3_bridge.js — Widevine L3 ISOLATED-world bridge
 *
 * Listens for window.postMessage events dispatched by l3_main_relay.js (which
 * runs in the MAIN world) and relays the extracted key/kid pair to the
 * background service worker via chrome.runtime.sendMessage.
 *
 * The MAIN world cannot call chrome.runtime.sendMessage directly in MV3; this
 * bridge provides the standard MAIN→postMessage→ISOLATED→sendMessage relay.
 */
window.addEventListener("message", function (event) {
  // Only accept messages from the same frame; ignore cross-origin noise.
  if (event.source !== window) return;
  var data = event.data;
  if (!data || data.type !== "__l3_key_found") return;

  var key = data.key;
  var kid = data.kid;
  var url = data.url;
  if (!key || !kid) return;

  chrome.runtime.sendMessage({
    type: "WVP2_L3_KEYS",
    keys: [{ kid: kid, key: key }],
    url: url,
  });
});
