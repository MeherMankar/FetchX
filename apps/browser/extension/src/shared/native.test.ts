import { afterEach, describe, expect, it, vi } from "vitest";

describe("sendToNativeHost", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it("resolves a response even if disconnect fires during cleanup", async () => {
    const messageListeners: Array<(message: unknown) => void> = [];
    const disconnectListeners: Array<() => void> = [];
    const response = { version: 1, ok: true, pong: true, app_version: "test" };
    const port = {
      onMessage: { addListener: (listener: (message: unknown) => void) => messageListeners.push(listener) },
      onDisconnect: { addListener: (listener: () => void) => disconnectListeners.push(listener) },
      postMessage: () => messageListeners.forEach((listener) => listener(response)),
      disconnect: () => disconnectListeners.forEach((listener) => listener()),
    };
    vi.stubGlobal("chrome", {
      runtime: {
        connectNative: () => port,
        lastError: undefined,
      },
    });

    const { sendToNativeHost } = await import("./native");
    await expect(sendToNativeHost({ version: 1, type: "ping" })).resolves.toEqual(response);
  });
});
