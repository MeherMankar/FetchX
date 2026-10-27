# Headless Remote Server

The CLI can host the shared FetchX `CoreServices` graph without the desktop UI:

```powershell
fetchx server init-auth
fetchx server rotate-auth
fetchx server
fetchx server --bind 127.0.0.1 --port 39800
fetchx server --bind 127.0.0.1 --port 39800 --database C:\FetchX\fetchx.db --downloads C:\FetchX\Downloads
```

Initialize authentication once before starting the server. `init-auth` creates a
cryptographically random 256-bit bearer token, displays it once, and stores only its
SHA-256 verifier in the existing SQLite `app_config` table. The token is not displayed
again at startup. Save the token in a password manager or another secure local secret
store. The default database is in the FetchX application-data directory; its access
inherits the current user's operating-system file permissions. No additional plaintext
credential file is created.

Use the token in an HTTP Authorization header:

```powershell
curl.exe -H "Authorization: Bearer <TOKEN>" -H "Content-Type: application/json" `
  --data-binary '{"jsonrpc":"2.0","id":1,"method":"system.discover"}' `
  http://127.0.0.1:39800/rpc
```

`rotate-auth` replaces the verifier and immediately invalidates the old token for new
HTTP requests and active WebSocket sessions. It displays the replacement once. Do not
put real tokens in shell history or scripts.

The default listener is `127.0.0.1:39800`. The default database and runtime paths
match the CLI's standard FetchX application-data directory; downloads default to the
user Downloads folder. `--bind` accepts an IP address (not a hostname), and the CLI
also accepts `--port`, `--database`, and `--downloads`.

## Endpoints

- **PUBLIC:** `GET /health` — process health and protocol version only.
- **AUTHENTICATED:** `POST /rpc` — all JSON-RPC 2.0 methods, including
  `system.discover`, using the `fetchx.rpc.v1` contract. Supply `Authorization: Bearer
  <TOKEN>`.
- **AUTHENTICATED:** `GET /events` — WebSocket JSON-RPC event notifications. The
  handshake requires the same `Authorization` header and is rejected before event
  subscription if authentication fails. Query-string tokens are not accepted.

All current JSON-RPC calls, including protocol discovery, require authentication; only
the health endpoint is public. Missing, malformed, and invalid bearer tokens receive a
generic HTTP 401 response and JSON-RPC `Unauthorized` error (`-32001`). Credential
material is never included in the error body or logs. Requests are verified centrally
before JSON-RPC dispatch, not separately inside each operation.

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

## Threat model and remaining limitations

| Threat | Current mitigation / status |
|---|---|
| Token guessing | 256 bits from the operating-system cryptographic random source; SHA-256 verifier comparison is constant-time. |
| Token leakage | Only the verifier is stored; token is shown only by explicit `init-auth` or `rotate-auth`. Request headers are not logged. |
| Replay | Bearer tokens are reusable; HTTPS/TLS is not configured. Do not send tokens over untrusted networks. |
| Brute force | Existing connection, body-size, and time limits apply. Dedicated rate limiting is deferred to 21E. |
| Credential theft | SQLite access is protected by the host user's existing filesystem permissions; the verifier alone cannot authenticate. |
| Unauthorized RPC | A centralized bearer-token check guards all RPC before parsing/dispatch. |
| Unauthorized WebSocket | The handshake is authenticated before event subscription; sessions are rechecked when sending events so token rotation closes old sessions. |
| Restart bypass | The verifier remains in SQLite and is loaded at startup; startup fails closed if auth is uninitialized or corrupt. |
| Credential reset abuse | Only an explicit local CLI `rotate-auth` operation changes the token; no hidden recovery token exists. |

Phase 21C provides token authentication only. It does **not** implement authorization
(21D), TLS, WebSocket Origin validation, SSRF protections, or broader remote hardening
(21E). The default bind remains loopback. Changing it does not make LAN/public exposure
safe; do not expose the server to a LAN, reverse proxy, or public internet.
