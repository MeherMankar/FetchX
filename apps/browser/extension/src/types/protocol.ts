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

export type BrowserRequest = AddDownloadRequest | PingRequest;

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

export interface ErrResponse {
  ok: false;
  error: BrowserError;
  request_id?: string;
  version: number;
}

export type BrowserResponse = AddDownloadOkResponse | PongOkResponse | ErrResponse;

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
    default:
      return "An unexpected error occurred. Check that FetchX is running.";
  }
}
