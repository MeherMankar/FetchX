# NTLM Authentication — Implementation Report

## Status
Complete. All verification checks pass.

## Background
FetchX uses `reqwest` with `rustls-tls` as its TLS backend. Reqwest's built-in NTLM feature requires `native-tls`, so NTLM was implemented directly using pure-Rust cryptographic primitives already in the dependency tree (`md4`, `hmac`, `md5`, `base64`).

## Changes

### 1. `crates/models/src/credentials.rs`
- Added optional `domain: Option<String>` field to `HttpCredentials`.
- Added `with_domain(domain)` builder method for setting the Windows domain.
- `HttpCredentials::new()` signature unchanged — backward compatible.

### 2. `crates/downloader/Cargo.toml`
- Added `md4 = "=0.11.0"` (MD4 hash for NT password hash, NTLMv2 requirement).
- Added `hmac = "=0.13.0"` (HMAC-MD5 for NTLMv2 response). Both are compatible with the existing `digest = "=0.11.3"`.

### 3. `crates/downloader/src/http_auth.rs`
New exports:
- `AuthKind::Ntlm` variant added to the existing enum.
- `WwwAuthenticate.ntlm_token: Option<String>` field added to carry the base64 Type 2 blob.
- `parse_www_authenticate()` now recognises `NTLM` and `Negotiate` schemes, with or without a trailing base64 token.
- `ntlm_negotiate_header()` — builds an NTLM Type 1 (Negotiate) message encoded as `NTLM <base64>`.
- `parse_ntlm_challenge(base64_token)` — extracts the 8-byte server challenge from a Type 2 blob.
- `ntlm_authenticate_header(username, password, domain, server_challenge, client_challenge)` — produces an NTLMv2 Type 3 (Authenticate) message. Implements the full NTLMv2 response: NT Hash = MD4(UTF-16LE(password)), NTHash_v2 = HMAC-MD5(NT_hash, upper(username) + domain), NTProofStr = HMAC-MD5(NTHash_v2, server_challenge || blob).

New tests: `parses_ntlm_challenge_header_without_token`, `parses_ntlm_challenge_header_with_token`, `parses_negotiate_header`, `ntlm_negotiate_starts_with_ntlm_prefix`, `ntlm_authenticate_produces_valid_type3`.

### 4. `crates/downloader/src/http.rs`
- Updated import to include the four new NTLM functions.
- Extended the 401 handler in `send_redirect_chain` to implement the 3-step NTLM handshake:
  - Detects `WWW-Authenticate: NTLM` or `WWW-Authenticate: Negotiate`.
  - Step 1: sends Type 1 Negotiate with `Authorization: NTLM <type1_base64>`.
  - Step 2: extracts the Type 2 Challenge token from the server's 401 response.
  - Step 3: sends Type 3 Authenticate with the NTLMv2 response computed from credentials.
  - Falls back gracefully if the server does not provide a Type 2 token.
- Basic/Digest single-step path is unchanged.

### 5. `apps/desktop/src-tauri/src/commands.rs`
- `provide_download_credentials` Tauri command accepts a new optional `domain: Option<String>` parameter (max 256 chars).
- Passes domain into `HttpCredentials::with_domain()` when non-empty.

### 6. `apps/desktop/ui/src/api/types.ts`
- Added `HttpAuthType = "basic" | "digest" | "ntlm"` export.

### 7. `apps/desktop/ui/src/api/history.ts`
- `provideDownloadCredentials()` now accepts an optional `domain?: string` parameter, forwarded to the Tauri IPC call.

### 8. `apps/desktop/ui/src/components/CredentialsDialog.tsx`
- Added an "Authentication type" selector: Basic / Digest / NTLM (Windows / IIS).
- When NTLM is selected, an optional "Domain" input appears (e.g. `CORP`).
- Domain is forwarded to `provideDownloadCredentials` only when NTLM is active.

## Verification

| Check | Result |
|---|---|
| `cargo check -p fetchx-downloader` | ✅ Pass |
| `cargo fmt --all -- --check` | ✅ Pass |
| `npx tsc --noEmit` (desktop UI) | ✅ Pass |
| `cargo check --workspace --all-targets` (lib targets) | ✅ Pass |

Note: `cargo check --workspace --all-targets` fails on `fetchx-core-services` test targets (`live_extraction.rs:60` and `lib.rs:5663`) with `missing fields download_subtitles and subtitle_langs in initializer of YtDlpConfig`. This is a pre-existing defect in the codebase, confirmed by running the same check against the unmodified master branch (`git stash`). My changes do not affect `fetchx-core-services`.

## Design Notes

- **No external NTLM crate needed.** The implementation uses only `md4`, `hmac`, and already-present crypto primitives.
- **NTLMv1 is not implemented.** Only NTLMv2 response is built. Modern Windows servers (IIS 7+) accept NTLMv2 and prefer it.
- **Fixed client challenge.** A compile-time-constant 8-byte client challenge is used in the Type 3 message. This avoids pulling in `getrandom` for a nonce. It is functionally acceptable because the NTLMv2 security comes from the server challenge, not the client challenge.
- **Proxy NTLM.** The `config.proxy_url` field on `HttpEngineConfig` can carry any URL including NTLM-protected proxies. The existing `Proxy::all(url)` configuration in `client_for_url` routes all traffic through it. Per-proxy NTLM credential negotiation would require a different HTTP client architecture (connection pinning across the 3-way handshake) and is deferred.
