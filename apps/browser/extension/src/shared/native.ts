/**
 * Typed Native Messaging client.
 *
 * Connects to the FetchX native host, sends one request, and returns
 * the response. Each call opens a fresh port because the native host handles
 * one request and exits; this works in both Chromium service workers and
 * Firefox background scripts.
 *
 * Security:
 * - The host is identified by the registered native messaging host name.
 * - The extension only sends structured, validated requests.
 * - Responses are validated for shape; no data is eval()'d.
 * - The host name must match the registered manifest exactly.
 */

import type {
  BrowserRequest,
  BrowserResponse,
  RpcRequest,
  RpcTarget,
} from "../types/protocol";
import { PROTOCOL_VERSION } from "../types/protocol";
import { connectNative, type NativePort } from "./browser-api";
import { isNativeHostUnavailableError } from "./native-host-error";

export { hasEmbeddedCredentials, isHttpUrl } from "./url-validation";

/** The registered native messaging host name.
 *  Must match the `name` field in the native host manifest JSON file. */
export const NATIVE_HOST_NAME = "com.fetchx.native_host";

/** Maximum time to wait for a response from the native host (milliseconds). */
const RESPONSE_TIMEOUT_MS = 70_000;

export class NativeHostError extends Error {
  constructor(
    message: string,
    public readonly cause?: unknown,
    public readonly outcomeUnknown = false,
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
    let port: NativePort;
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    let settled = false;

    const cleanup = (disconnectPort = true) => {
      if (timeoutId !== undefined) clearTimeout(timeoutId);
      if (disconnectPort) {
        try { port.disconnect(); } catch { /* ignore */ }
      }
    };

    try {
      port = connectNative(NATIVE_HOST_NAME);
    } catch (err) {
      reject(new NativeHostError("Could not connect to FetchX native host.", err));
      return;
    }

    timeoutId = setTimeout(() => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(new NativeHostError("Native host did not respond in time.", undefined, true));
    }, RESPONSE_TIMEOUT_MS);

    port.onDisconnect.addListener(() => {
      const firefoxError = "error" in port ? port.error : undefined;
      const runtimeError = typeof browser === "undefined" ? chrome.runtime.lastError : undefined;
      if (settled) return;
      settled = true;
      cleanup(false);
      const errorMessage = firefoxError?.message ?? runtimeError?.message;
      const hostUnavailable = isNativeHostUnavailableError(errorMessage);
      reject(
        new NativeHostError(
          errorMessage
            ?? "FetchX's native host stopped before replying. Make sure FetchX is running, repair the native host in Settings → Browser extensions, then restart your browser.",
          undefined,
          !hostUnavailable,
        ),
      );
    });

    port.onMessage.addListener((message: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      if (!isValidResponse(message)) {
        reject(
          new NativeHostError(
            "Native host returned an unexpected response shape.",
            undefined,
            true,
          ),
        );
        return;
      }
      resolve(message);
    });

    try {
      port.postMessage(request);
    } catch (error) {
      if (settled) return;
      settled = true;
      cleanup();
      reject(new NativeHostError("Could not send the request to the native host.", error, true));
    }
  });
}

export async function callTargetRpc(
  target: RpcTarget,
  request: RpcRequest,
): Promise<BrowserResponse> {
  return sendToNativeHost({
    version: PROTOCOL_VERSION,
    type: "rpc",
    target,
    request,
  });
}

export async function storeTargetCredential(
  credentialId: string,
  endpoint: string,
  token: string,
): Promise<BrowserResponse> {
  return sendToNativeHost({
    version: PROTOCOL_VERSION,
    type: "store_credential",
    credential_id: credentialId,
    endpoint,
    token,
  });
}

export async function deleteTargetCredential(
  credentialId: string,
  endpoint: string,
): Promise<BrowserResponse> {
  return sendToNativeHost({
    version: PROTOCOL_VERSION,
    type: "delete_credential",
    credential_id: credentialId,
    endpoint,
  });
}

/** Runtime shape guard — never trust data from the native host blindly. */
function isValidResponse(value: unknown): value is BrowserResponse {
  if (typeof value !== "object" || value === null) return false;
  const obj = value as Record<string, unknown>;
  return (
    obj.version === PROTOCOL_VERSION &&
    typeof obj.ok === "boolean"
  );
}
