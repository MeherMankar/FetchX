import { afterEach, describe, expect, it, vi } from "vitest";

import { RemoteClient, RemoteError } from "./protocol";

const fakeToken = "fx_test_token_for_unit_tests";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("RemoteClient", () => {
  it("sends authenticated JSON-RPC requests with IDs and handles success envelopes", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const request = JSON.parse(String(init?.body)) as {
        jsonrpc: string;
        id: string;
        method: string;
      };
      return Response.json({
        jsonrpc: "2.0",
        id: request.id,
        result: { protocol_version: "fetchx.rpc.v1" },
      });
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new RemoteClient("http://127.0.0.1:39800", fakeToken, vi.fn());

    await expect(client.call("system.discover")).resolves.toEqual({
      protocol_version: "fetchx.rpc.v1",
    });
    const [, init] = fetchMock.mock.calls[0];
    expect(new Headers(init?.headers).get("Authorization")).toBe(`Bearer ${fakeToken}`);
    expect(JSON.parse(String(init?.body))).toMatchObject({
      jsonrpc: "2.0",
      method: "system.discover",
    });
  });

  it("invalidates the in-memory session on HTTP 401", async () => {
    const invalidate = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => Response.json({
      jsonrpc: "2.0",
      id: "request",
      error: { code: -32001, message: "authentication required" },
    }, { status: 401 })));
    const client = new RemoteClient("http://localhost:39800", fakeToken, invalidate);

    await expect(client.call("download.list")).rejects.toMatchObject({
      kind: "authentication",
      status: 401,
    });
    expect(invalidate).toHaveBeenCalledOnce();
  });

  it("surfaces forbidden responses without clearing the authenticated session", async () => {
    const invalidate = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => Response.json({
      jsonrpc: "2.0",
      id: "request",
      error: { code: -32003, message: "permission denied" },
    }, { status: 403 })));
    const client = new RemoteClient("http://localhost:39800", fakeToken, invalidate);

    await expect(client.call("download.remove", { id: "download-id" }))
      .rejects.toMatchObject({ kind: "forbidden", status: 403 });
    expect(invalidate).not.toHaveBeenCalled();
  });

  it("sends native torrent metadata and selection calls using the advertised RPC methods", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const request = JSON.parse(String(init?.body)) as { id: string; method: string; params: unknown };
      return Response.json({
        jsonrpc: "2.0",
        id: request.id,
        result: request.method === "download.torrentMetadata"
          ? { metadata_state: "ready", files: [] }
          : { selection_confirmed: true },
      });
    });
    vi.stubGlobal("fetch", fetchMock);
    const client = new RemoteClient("http://127.0.0.1:39800", fakeToken, vi.fn());

    await client.call("download.torrentMetadata", { id: "download-id" });
    await client.call("download.selectTorrentFiles", {
      id: "download-id",
      info_hash: "0123456789012345678901234567890123456789",
      selected_file_indexes: [1, 3],
    });
    expect(fetchMock.mock.calls.map(([, init]) => JSON.parse(String(init?.body))))
      .toMatchObject([
        { method: "download.torrentMetadata", params: { id: "download-id" } },
        {
          method: "download.selectTorrentFiles",
          params: {
            id: "download-id",
            info_hash: "0123456789012345678901234567890123456789",
            selected_file_indexes: [1, 3],
          },
        },
      ]);
  });

  it("uses a one-use ticket in the browser WebSocket URL", async () => {
    let connectedUrl = "";
    vi.stubGlobal("fetch", vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const request = JSON.parse(String(init?.body)) as { id: string };
      return Response.json({
        jsonrpc: "2.0",
        id: request.id,
        result: { ticket: `fxws_${"a".repeat(64)}`, expires_in_seconds: 30 },
      });
    }));
    class FakeWebSocket extends EventTarget {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readonly url: string;
      readyState = FakeWebSocket.CONNECTING;

      constructor(url: string | URL) {
        super();
        this.url = String(url);
        connectedUrl = this.url;
        queueMicrotask(() => {
          this.readyState = FakeWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        });
      }

      close(): void {
        this.readyState = FakeWebSocket.CLOSED;
        this.dispatchEvent(new Event("close"));
      }
    }
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = new RemoteClient("https://fetchx.example.test", fakeToken, vi.fn());
    await client.openEvents(vi.fn(), vi.fn());

    const websocketUrl = new URL(connectedUrl);
    expect(websocketUrl.protocol).toBe("wss:");
    expect(websocketUrl.pathname).toBe("/events");
    expect(websocketUrl.searchParams.get("ticket")).toBe(`fxws_${"a".repeat(64)}`);
  });

  it("rejects plaintext access to non-loopback hosts", () => {
    expect(() => new RemoteClient("http://remote.example.test", fakeToken, vi.fn()))
      .toThrowError(RemoteError);
  });
});
