export const protocolVersion = "fetchx.rpc.v1";

export type RpcId = string | number;

export interface RpcRequest {
  jsonrpc: "2.0";
  id: RpcId;
  method: string;
  params?: Record<string, unknown>;
}

export interface RpcErrorObject {
  code: number;
  message: string;
  data?: unknown;
}

export interface RpcSuccess<T> {
  jsonrpc: "2.0";
  id: RpcId;
  result: T;
}

export interface RpcFailure {
  jsonrpc: "2.0";
  id: RpcId | null;
  error: RpcErrorObject;
}

export type DownloadStatus =
  | "queued"
  | "preparing"
  | "downloading"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled"
  | "metadata_acquiring"
  | "seeding"
  | "recovering"
  | "stopped";

export type DownloadPriority = "low" | "normal" | "high";
export type EngineKind =
  | "direct_http"
  | "aria2"
  | "yt_dlp"
  | "hls"
  | "native_torrent"
  | "ftp"
  | "ftps"
  | "sftp"
  | "metalink";

export interface RemoteDownload {
  id: string;
  filename: string;
  status: DownloadStatus;
  priority: DownloadPriority;
  engine: EngineKind;
  downloaded_bytes: number;
  total_bytes: number | null;
  speed_bytes_per_second: number | null;
  estimated_remaining_seconds: number | null;
  progress_basis_points: number | null;
  error_code: "source" | "destination" | "engine" | "internal" | null;
  created_at: number;
  updated_at: number;
}

export interface RemoteDuplicateDownloadMatch {
  id: string;
  filename: string;
  status: DownloadStatus;
  updated_at: number;
}

export interface RemoteDownloadProgress {
  downloaded_bytes: number;
  total_bytes: number | null;
  speed_bytes_per_second: number | null;
  estimated_remaining_seconds: number | null;
  percentage_basis_points: number | null;
}

export interface RemoteDownloadRuntimeStatus {
  id: string;
  status: DownloadStatus;
  progress: RemoteDownloadProgress;
  updated_at: number;
}

export interface ProtocolInfo {
  protocol_version: string;
  server_version: string;
  capabilities: string[];
}

export interface HealthInfo {
  status: string;
  protocol_version: string;
}

export interface RemoteEvent {
  jsonrpc: "2.0";
  method: string;
  params: { data: unknown };
}

export type RemoteMethod =
  | "system.discover"
  | "system.websocketTicket"
  | "download.create"
  | "download.get"
  | "download.list"
  | "download.runtimeStatus"
  | "download.findDuplicate"
  | "download.pause"
  | "download.resume"
  | "download.cancel"
  | "download.remove"
  | "download.setPriority"
  | "queue.moveUp"
  | "queue.moveDown";

export class RemoteError extends Error {
  readonly kind: "network" | "authentication" | "forbidden" | "rpc" | "protocol";
  readonly status?: number;
  readonly code?: number;

  constructor(
    message: string,
    kind: RemoteError["kind"],
    details: { status?: number; code?: number } = {},
  ) {
    super(message);
    this.name = "RemoteError";
    this.kind = kind;
    this.status = details.status;
    this.code = details.code;
  }
}

let nextId = 0;

export class RemoteClient {
  private readonly baseUrl: URL;
  private token: string;
  private readonly onAuthenticationFailure: () => void;

  constructor(serverUrl: string, token: string, onAuthenticationFailure: () => void) {
    this.baseUrl = validateServerUrl(serverUrl);
    if (!token.trim()) throw new RemoteError("Enter an authentication token.", "authentication");
    this.token = token;
    this.onAuthenticationFailure = onAuthenticationFailure;
  }

  get serverOrigin(): string {
    return this.baseUrl.origin;
  }

  async health(): Promise<HealthInfo> {
    let response: Response;
    try {
      response = await fetch(new URL("/health", this.baseUrl));
    } catch {
      throw new RemoteError("Could not reach the FetchX server.", "network");
    }
    if (!response.ok) {
      throw new RemoteError(`Health check failed (HTTP ${response.status}).`, "network", {
        status: response.status,
      });
    }
    return parseJson<HealthInfo>(response);
  }

  async call<T>(
    method: RemoteMethod,
    params?: Record<string, unknown>,
  ): Promise<T> {
    const request: RpcRequest = {
      jsonrpc: "2.0",
      id: `remote-ui-${Date.now()}-${++nextId}`,
      method,
      ...(params ? { params } : {}),
    };
    let response: Response;
    try {
      response = await fetch(new URL("/rpc", this.baseUrl), {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.token}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(request),
      });
    } catch {
      throw new RemoteError("Could not reach the FetchX server.", "network");
    }

    const envelope = await parseJson<RpcSuccess<T> | RpcFailure>(response);
    if (response.status === 401 || isRpcError(envelope) && envelope.error.code === -32001) {
      this.token = "";
      this.onAuthenticationFailure();
      throw new RemoteError("Authentication expired or was rejected. Connect again.", "authentication", {
        status: 401,
        code: isRpcError(envelope) ? envelope.error.code : undefined,
      });
    }
    if (response.status === 403 || isRpcError(envelope) && envelope.error.code === -32003) {
      throw new RemoteError("This account is not permitted to perform that action.", "forbidden", {
        status: 403,
        code: isRpcError(envelope) ? envelope.error.code : undefined,
      });
    }
    if (!response.ok) {
      throw new RemoteError(`Request failed (HTTP ${response.status}).`, "network", {
        status: response.status,
      });
    }
    if (isRpcError(envelope)) {
      throw new RemoteError(envelope.error.message, "rpc", { code: envelope.error.code });
    }
    if (envelope.jsonrpc !== "2.0" || envelope.id !== request.id) {
      throw new RemoteError("The server returned an invalid JSON-RPC response.", "protocol");
    }
    return envelope.result;
  }

  findDuplicate(url: string): Promise<RemoteDuplicateDownloadMatch | null> {
    return this.call<RemoteDuplicateDownloadMatch | null>("download.findDuplicate", { url });
  }

  async listRuntimeStatuses(): Promise<RemoteDownloadRuntimeStatus[]> {
    const result = await this.call<{ statuses: RemoteDownloadRuntimeStatus[] }>(
      "download.runtimeStatus",
    );
    return result.statuses;
  }

  async openEvents(
    onEvent: (event: RemoteEvent) => void,
    onClose: () => void,
  ): Promise<WebSocket> {
    const result = await this.call<{ ticket: string; expires_in_seconds: number }>(
      "system.websocketTicket",
    );
    if (!result.ticket || result.expires_in_seconds !== 30) {
      throw new RemoteError("The server returned an invalid WebSocket ticket.", "protocol");
    }
    const socketUrl = new URL("/events", this.baseUrl);
    socketUrl.protocol = this.baseUrl.protocol === "https:" ? "wss:" : "ws:";
    socketUrl.searchParams.set("ticket", result.ticket);

    return new Promise((resolve, reject) => {
      const socket = new WebSocket(socketUrl);
      let opened = false;
      socket.addEventListener("open", () => {
        opened = true;
        resolve(socket);
      }, { once: true });
      socket.addEventListener("message", (message) => {
        if (typeof message.data !== "string") return;
        try {
          const event = JSON.parse(message.data) as RemoteEvent;
          if (event.jsonrpc === "2.0" && typeof event.method === "string" && event.params) {
            onEvent(event);
          }
        } catch {
          // Ignore malformed event frames without exposing transport payloads.
        }
      });
      socket.addEventListener("close", () => {
        onClose();
        if (!opened) reject(new RemoteError("The event stream could not be authorized.", "forbidden"));
      }, { once: true });
      socket.addEventListener("error", () => {
        if (!opened) reject(new RemoteError("The event stream could not be opened.", "network"));
      }, { once: true });
    });
  }

  clear(): void {
    this.token = "";
  }
}

function validateServerUrl(raw: string): URL {
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    throw new RemoteError("Enter a valid server URL, including http:// or https://.", "protocol");
  }
  if (
    !["http:", "https:"].includes(url.protocol)
    || url.username
    || url.password
    || url.pathname !== "/"
    || url.search
    || url.hash
  ) {
    throw new RemoteError("Use an HTTP(S) server origin without credentials or a path.", "protocol");
  }
  if (url.protocol === "http:" && !isLoopbackHost(url.hostname)) {
    throw new RemoteError("Use HTTPS for non-loopback servers; plaintext is allowed only on loopback.", "protocol");
  }
  if (globalThis.location?.protocol === "https:" && url.protocol !== "https:") {
    throw new RemoteError("An HTTPS web page can connect only to an HTTPS FetchX server.", "protocol");
  }
  return url;
}

function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  return host === "localhost" || host === "::1" || /^127(?:\.\d{1,3}){3}$/.test(host);
}

async function parseJson<T>(response: Response): Promise<T> {
  try {
    return await response.json() as T;
  } catch {
    throw new RemoteError("The server returned an invalid JSON response.", "protocol", {
      status: response.status,
    });
  }
}

function isRpcError<T>(value: RpcSuccess<T> | RpcFailure): value is RpcFailure {
  return "error" in value;
}
