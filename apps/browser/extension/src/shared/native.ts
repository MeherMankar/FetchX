/**
 * Typed Native Messaging client.
 *
 * Connects to the FetchX native host, sends one request, and returns
 * the response.  Each call opens a fresh port connection (Native Messaging
 * connections are stateless per-message in Manifest V3 service workers).
 *
 * Security:
 * - The host is identified by the registered native messaging host name.
 * - The extension only sends structured, validated requests.
 * - Responses are validated for shape; no data is eval()'d.
 * - The host name must match the registered manifest exactly.
 */

import type { BrowserRequest, BrowserResponse } from "../types/protocol";

/** The registered native messaging host name.
 *  Must match the `name` field in the native host manifest JSON file. */
export const NATIVE_HOST_NAME = "com.fetchx.native_host";

/** Maximum time to wait for a response from the native host (milliseconds). */
const RESPONSE_TIMEOUT_MS = 15_000;

export class NativeHostError extends Error {
  constructor(
    message: string,
    public readonly cause?: unknown,
  ) {
    super(message);
    this.name = "NativeHostError";
  }
}

/**
 * Sends a single request to the FetchX native host and returns the
 * response.  Throws {@link NativeHostError} on timeout, disconnection, or
 * if the response shape is unexpected.
 *
 * Never uses eval() or executes any data received from the host.
 */
export async function sendToNativeHost(request: BrowserRequest): Promise<BrowserResponse> {
  return new Promise<BrowserResponse>((resolve, reject) => {
    let port: chrome.runtime.Port;
    let timeoutId: ReturnType<typeof setTimeout> | undefined;

    const cleanup = () => {
      if (timeoutId !== undefined) clearTimeout(timeoutId);
      try { port.disconnect(); } catch { /* ignore */ }
    };

    try {
      port = chrome.runtime.connectNative(NATIVE_HOST_NAME);
    } catch (err) {
      reject(new NativeHostError("Could not connect to FetchX native host.", err));
      return;
    }

    timeoutId = setTimeout(() => {
      cleanup();
      reject(new NativeHostError("Native host did not respond in time."));
    }, RESPONSE_TIMEOUT_MS);

    port.onDisconnect.addListener(() => {
      cleanup();
      const runtimeError = chrome.runtime.lastError;
      reject(
        new NativeHostError(
          runtimeError?.message ?? "Native host disconnected unexpectedly.",
        ),
      );
    });

    port.onMessage.addListener((message: unknown) => {
      cleanup();
      if (!isValidResponse(message)) {
        reject(new NativeHostError("Native host returned an unexpected response shape."));
        return;
      }
      resolve(message);
    });

    port.postMessage(request);
  });
}

/** Runtime shape guard — never trust data from the native host blindly. */
function isValidResponse(value: unknown): value is BrowserResponse {
  if (typeof value !== "object" || value === null) return false;
  const obj = value as Record<string, unknown>;
  return (
    typeof obj.version === "number" &&
    typeof obj.ok === "boolean"
  );
}

/**
 * Validates a URL client-side for fast UX feedback.
 * The native host and desktop application perform the authoritative check.
 */
export function isHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * Returns true if the URL contains embedded credentials.
 * Mirrors the server-side check.
 */
export function hasEmbeddedCredentials(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.username !== "" || parsed.password !== "";
  } catch {
    return false;
  }
}
