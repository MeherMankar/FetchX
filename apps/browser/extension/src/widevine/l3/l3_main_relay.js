/**
 * l3_main_relay.js — Widevine L3 key relay (MAIN world)
 *
 * Runs after content_key_decryption.js and eme_interception.js in the MAIN
 * world. Wraps console.log to intercept the key-found line produced by
 * WidevineCrypto.decryptContentKey and forwards the key/kid pair to the
 * ISOLATED world via window.postMessage so l3_bridge.js can relay it to the
 * background service worker.
 */
(function () {
  var _origLog = console.log.bind(console);
  console.log = function () {
    _origLog.apply(console, arguments);
    var msg = typeof arguments[0] === "string" ? arguments[0] : "";
    // Matches: "WidevineDecryptor: Found key: <hex> (KID=<hex>)"
    var m = msg.match(
      /WidevineDecryptor: Found key: ([0-9a-fA-F]+) \(KID=([0-9a-fA-F]+)\)/
    );
    if (m) {
      window.postMessage(
        {
          type: "__l3_key_found",
          key: m[1],
          kid: m[2],
          url: window.location.href,
        },
        "*"
      );
    }
  };
})();
