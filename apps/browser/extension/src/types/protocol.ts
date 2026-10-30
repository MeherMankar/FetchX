/**
 * Versioned browser-to-desktop protocol types.
 * Must stay in sync with crates/browser-protocol/src/lib.rs.
 */

export const PROTOCOL_VERSION = 1 as const;

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

export interface AddDownloadRequest {
  version: typeof PROTOCOL_VERSION;
  type: "add_download";
  url: string;
  request_id?: string;
}

export interface PingRequest {
  version: typeof PROTOCOL_VERSION;
  type: "ping";
}

export type RpcTarget =
  | { kind: "local" }
  | { kind: "remote"; endpoint: string; credential_id: string };

export interface RpcRequest {
  jsonrpc: "2.0";
  id: string | number;
  method: string;
  params?: Record<string, unknown>;
}

export interface BrowserRpcRequest {
  version: typeof PROTOCOL_VERSION;
  type: "rpc";
  target: RpcTarget;
  request: RpcRequest;
}

export interface StoreCredentialRequest {
  version: typeof PROTOCOL_VERSION;
  type: "store_credential";
  credential_id: string;
  endpoint: string;
  token: string;
}

export interface DeleteCredentialRequest {
  version: typeof PROTOCOL_VERSION;
  type: "delete_credential";
  credential_id: string;
  endpoint: string;
}

export type BrowserRequest =
  | AddDownloadRequest
  | PingRequest
  | BrowserRpcRequest
  | StoreCredentialRequest
  | DeleteCredentialRequest;

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

export type BrowserErrorCode =
  | "invalid_request"
  | "unsupported_version"
  | "unsupported_message"
  | "invalid_url"
  | "credentials_in_url"
  | "unsupported_scheme"
  | "desktop_unavailable"
  | "authentication_failed"
  | "authorization_failed"
  | "incompatible_server"
  | "invalid_target"
  | "credential_store_unavailable"
  | "download_rejected"
  | "internal_error";

export interface BrowserError {
  code: BrowserErrorCode;
  message: string;
}

export interface AddDownloadOkResponse {
  ok: true;
  download_id: string;
  request_id?: string;
  version: number;
}

export interface PongOkResponse {
  ok: true;
  pong: true;
  app_version: string;
  request_id?: string;
  version: number;
}

export interface RpcOkResponse {
  ok: true;
  rpc_response: Record<string, unknown>;
  request_id?: string;
  version: number;
}

export interface CredentialStoredOkResponse {
  ok: true;
  stored: true;
  request_id?: string;
  version: number;
}

export interface ErrResponse {
  ok: false;
  error: BrowserError;
  request_id?: string;
  version: number;
}

export type BrowserResponse =
  | AddDownloadOkResponse
  | PongOkResponse
  | RpcOkResponse
  | CredentialStoredOkResponse
  | ErrResponse;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Returns true when the response is an error. */
export function isError(r: BrowserResponse): r is ErrResponse {
  return !r.ok;
}

/** Returns a user-readable message for a browser error code. */
export function userMessageForCode(code: BrowserErrorCode): string {
  switch (code) {
    case "invalid_url":
      return "The URL is not valid.";
    case "credentials_in_url":
      return "URLs containing passwords or usernames are not accepted.";
    case "unsupported_scheme":
      return "Only HTTP and HTTPS URLs are supported.";
    case "desktop_unavailable":
      return "FetchX is not running. Please start the application first.";
    case "download_rejected":
      return "FetchX rejected the download. Check the application for details.";
    case "unsupported_version":
      return "Please update FetchX to use this extension.";
    case "authentication_failed":
      return "The selected FetchX target rejected its authentication token.";
    case "authorization_failed":
      return "The selected FetchX account is not allowed to perform this operation.";
    case "incompatible_server":
      return "The selected target does not support a compatible FetchX protocol.";
    case "invalid_target":
      return "The selected FetchX target is invalid.";
    case "credential_store_unavailable":
      return "The operating system credential store is unavailable.";
    default:
      return "An unexpected error occurred. Check that FetchX is running.";
  }
}
