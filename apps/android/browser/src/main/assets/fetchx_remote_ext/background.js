"use strict";

browser.runtime.onMessage.addListener((message) => {
  if (message?.type !== "fetchx-rpc" || typeof message.request !== "object" || message.request === null) {
    return false;
  }
  return browser.runtime.sendNativeMessage("browser", message.request).catch(
    () => JSON.stringify({ error: { message: "FetchX RPC is unavailable." } }),
  );
});
