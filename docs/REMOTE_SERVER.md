# Headless Remote Server

The CLI can host the shared FetchX `CoreServices` graph without the desktop UI:

```powershell
fetchx server init-auth
fetchx server rotate-auth
fetchx server
fetchx server --bind 127.0.0.1 --port 39800
fetchx server --bind 127.0.0.1 --port 39800 --database C:\FetchX\fetchx.db --downloads C:\FetchX\Downloads
fetchx server --bind 127.0.0.1 --port 39800 --tls-certificate C:\FetchX\server-cert.pem --tls-private-key C:\FetchX\server-key.pem
fetchx server --allowed-origin https://fetchx.example.com
```

Initialize authentication once before starting the server. `init-auth` creates a
cryptographically random 256-bit bearer token, displays it once, and stores only its
SHA-256 verifier in the existing SQLite `app_config` table. The token is not displayed
again at startup. Save the token in a password manager or another secure local secret
store. The default database is in the FetchX application-data directory; its access
inherits the current user's operating-system file permissions. No additional plaintext
credential file is created.

TLS is enabled only when both `--tls-certificate` and `--tls-private-key` are
provided. Both files must be PEM encoded. Invalid, missing, or mismatched files
cause startup to fail; a requested TLS server never falls back to plaintext. The
default listener remains loopback-only. Without these flags, the server continues
to serve HTTP as before. Protect the private key with operating-system file
permissions and do not place it in source control.

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
also accepts `--port`, `--database`, and `--downloads`. Repeat `--allowed-origin`
to add exact HTTP(S) web-client origins. The server keeps its loopback origins and
rejects configured origins containing a path, credentials, query, or fragment.
Use HTTPS for a remotely hosted UI and configure the server's TLS certificate.

The Phase 21F web client is under `apps/remote-ui`. For local development, start
the server with `--allowed-origin http://127.0.0.1:5173`, then run
`npm --prefix apps/remote-ui run dev`. A production `dist` bundle can be served
from a static HTTPS origin; add that exact origin to `--allowed-origin`. The web UI
uses authenticated JSON-RPC requests and does not persist bearer tokens in browser
storage.

## Endpoints

- **PUBLIC:** `GET /health` — process health and protocol version only.
- **AUTHENTICATED:** `POST /rpc` — all JSON-RPC 2.0 methods, including
  `system.discover`, using the `fetchx.rpc.v1` contract. Supply `Authorization: Bearer
  <TOKEN>`.
- **AUTHENTICATED:** `GET /events` — WebSocket JSON-RPC event notifications. Native
  clients may authenticate with the bearer `Authorization` header. Browsers cannot
  set that header on a WebSocket handshake, so the web UI first calls
  `system.websocketTicket` over authenticated JSON-RPC and supplies the returned
  short-lived, single-use event ticket. Reusable bearer tokens are never accepted
  in the query string.

All current JSON-RPC calls, including protocol discovery, require authentication; only
the health endpoint is public. Missing, malformed, and invalid bearer tokens receive a
generic HTTP 401 response and JSON-RPC `Unauthorized` error (`-32001`). Credential
material is never included in the error body or logs. Requests are verified centrally
before JSON-RPC dispatch, not separately inside each operation.

Current RPC operations are `system.discover`, `download.create`, `download.get`,
`download.list`, `download.pause`, `download.resume`, `download.cancel`,
`download.remove`, `download.setPriority`, and `system.websocketTicket`. Creation
supports HTTP(S) downloads through DirectHttp and magnet downloads through the engine selected by CoreServices.
Discovery reports only supported operations and event names. An operation accepted by
the shared Phase 21A parser but not implemented by CoreServices returns the stable
`UnsupportedOperation` error; it is not reported as a successful no-op.

The browser UI exposes current download data and queue actions supported by these
methods. Queue metrics/order, engine management/status, scheduling, remote settings,
device pairing, and plugin management are not available in the current protocol and
are shown as unavailable rather than simulated.

Download responses and event payloads include the ID, filename, lifecycle state,
priority, engine, progress, timestamps, and a non-sensitive error code. They do not
include source URLs, destination paths, or raw engine error text. WebSocket subscribers
use a bounded event buffer; a slow subscriber is disconnected rather than blocking
downloads or silently continuing after an event gap.

## Resource bounds and security defaults

The server accepts JSON-RPC request bodies up to the protocol's 1 MiB maximum. Default
limits are 32 concurrent RPC/WebSocket sessions, a 30-second request/body deadline,
and 256 queued notifications per WebSocket client. Configuration validation caps
these at 128 sessions, 300 seconds, and 2,048 queued notifications. WebSocket frames
use the configured request-size limit. The queue's existing broadcast event capacity
remains bounded independently.
The WebSocket ticket table is in-memory, capped at 1,024 pending tickets, and tickets
expire after 30 seconds. Each ticket is scoped to event access, accepted once, stored
only as a digest, and tied to the current credential verifier; credential rotation
invalidates outstanding tickets.

The default bind remains loopback (`127.0.0.1`). The remote server intentionally does
not expose a public listener by default. Direct HTTP and redirect targets are rejected
when they resolve to localhost, loopback, private, link-local, multicast, unspecified,
or IPv4-mapped IPv6 local ranges. The downloader validates and pins each resolved
destination address for the matching connection, including each redirect hop, to
prevent a second DNS lookup from selecting a different address. Environment proxy settings are
bypassed for these protected direct requests. WebSocket connections require a
configured origin allow-list; forbidden origins are rejected before subscription
establishment. HTTP browser requests receive CORS headers only for exact configured
origins; preflight accepts only the remote RPC method and its authorization/content
type headers.
A bounded in-memory rate limiter rejects repeated abusive requests with HTTP 429 while
preserving the existing 21A/21D authentication and authorization checks.

## Threat model and remaining limitations

| Threat | Current mitigation / status |
|---|---|
| Token guessing | 256 bits from the operating-system cryptographic random source; SHA-256 verifier comparison is constant-time. |
| Token leakage | Only the verifier is stored; token is shown only by explicit `init-auth` or `rotate-auth`. Request headers are not logged. |
| Replay | Bearer tokens are reusable. TLS is optional; configure a certificate and private key before sending tokens over an untrusted network. |
| Brute force | Bounded in-memory HTTP/WebSocket rate limiting plus connection, body-size, and request-deadline limits. |
| Credential theft | SQLite access is protected by the host user's existing filesystem permissions; the verifier alone cannot authenticate. |
| Unauthorized RPC | A centralized bearer-token check guards all RPC before parsing/dispatch. |
| Unauthorized WebSocket | The handshake is authenticated before event subscription; sessions are rechecked when sending events so token rotation closes old sessions. |
| Restart bypass | The verifier remains in SQLite and is loaded at startup; startup fails closed if auth is uninitialized or corrupt. |
| Credential reset abuse | Only an explicit local CLI `rotate-auth` operation changes the token; no hidden recovery token exists. |

Browser event tickets are sent in the WebSocket URL because the browser WebSocket
API does not permit custom Authorization headers. Tickets are random, one-use, and
short-lived rather than reusable bearer credentials. A reverse proxy should avoid
logging the `ticket` query parameter. The default bind remains loopback. Changing it
does not make LAN/public exposure safe; use TLS and review the deployment threat
model before exposing the server to a LAN, reverse proxy, or public internet.
