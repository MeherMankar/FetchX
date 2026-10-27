# Headless Remote Server

The CLI can host the shared FetchX `CoreServices` graph without the desktop UI:

```powershell
fetchx server
fetchx server --bind 127.0.0.1 --port 39800
fetchx server --bind 127.0.0.1 --port 39800 --database C:\FetchX\fetchx.db --downloads C:\FetchX\Downloads
```

The default listener is `127.0.0.1:39800`. The default database and runtime paths
match the CLI's standard FetchX application-data directory; downloads default to the
user Downloads folder. `--bind` accepts an IP address (not a hostname), and the CLI
also accepts `--port`, `--database`, and `--downloads`.

## Endpoints

- `GET /health` — process health and protocol version.
- `POST /rpc` — JSON-RPC 2.0 requests using the `fetchx.rpc.v1` contract.
- `GET /events` — WebSocket JSON-RPC event notifications.

Current RPC operations are `system.discover`, `download.create`, `download.get`,
`download.list`, `download.pause`, `download.resume`, `download.cancel`,
`download.remove`, and `download.setPriority`. Creation supports HTTP(S) downloads
through DirectHttp and magnet downloads through the engine selected by CoreServices.
Discovery reports only supported operations and event names. An operation accepted by
the shared Phase 21A parser but not implemented by CoreServices returns the stable
`UnsupportedOperation` error; it is not reported as a successful no-op.

Download responses and event payloads include the ID, filename, lifecycle state,
priority, engine, progress, timestamps, and a non-sensitive error code. They do not
include source URLs, destination paths, or raw engine error text. WebSocket subscribers
use a bounded event buffer; a slow subscriber is disconnected rather than blocking
downloads or silently continuing after an event gap.

## Resource bounds

The server accepts JSON-RPC request bodies up to the protocol's 1 MiB maximum. Default
limits are 32 concurrent RPC/WebSocket sessions, a 30-second request/body deadline,
and 256 queued notifications per WebSocket client. Configuration validation caps
these at 128 sessions, 300 seconds, and 2,048 queued notifications. WebSocket frames
use the configured request-size limit. The queue's existing broadcast event capacity
remains bounded independently.

## Security scope

This server currently has **no authentication, authorization, TLS, rate limiting,
WebSocket Origin validation, or SSRF protection**. The bind address is loopback by
default, but changing it does not add security. Do not bind to a LAN address, use a
reverse proxy, or expose the server to the public internet. Remote authentication,
authorization, and security hardening are later roadmap phases.
