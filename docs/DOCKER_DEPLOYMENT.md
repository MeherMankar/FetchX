# Docker and NAS Deployment

This deployment runs the existing FetchX headless server and its shared core
services. It does not add an API, authentication mechanism, or business logic.
The container listens on port `39800`; Compose publishes it on the host loopback
address by default. Keep that binding unless access is intentionally provided
through a trusted reverse proxy or other protected network boundary.

## Requirements

- Docker Engine with Docker Compose v2, including on NAS systems that support
  Compose projects.
- The repository checkout and a working Docker build environment.
- Persistent writable storage for FetchX data; the provided Compose file uses a
  named volume.

## Build and initialize authentication

From the repository root, build the image:

```sh
docker compose build
```

Initialize authentication once. This command prints a new random bearer token
once; capture it directly into a password manager or another secure secret
store. Do not put it in shell history, a Compose file, `.env`, or logs.

```sh
docker compose run --rm fetchx server init-auth
```

Start the service:

```sh
docker compose up -d
docker compose ps
```

The public health endpoint is available at `http://127.0.0.1:39800/health`.
The server applies its embedded SQLite migrations when it opens the database.
Use the token only over HTTPS when it traverses an untrusted network.

## Web UI, origins, and TLS

The host port is loopback-only by default. To use the Remote Web UI, configure
its exact origin in `FETCHX_ALLOWED_ORIGINS`, as a comma-separated list in the
environment used by Compose. For example:

```text
FETCHX_ALLOWED_ORIGINS=https://fetchx.example.com
```

The server validates each origin using the existing exact HTTP(S) origin policy.
Keep the UI on HTTPS when connecting over an untrusted network. A reverse proxy
can terminate public HTTPS and forward to the loopback-published server; configure
it to forward WebSocket upgrades for `/events` and preserve the `Origin` header.
If the reverse proxy is another container, attach it to the same Docker network
and forward to `http://fetchx:39800` instead of trying to reach the host's
loopback address.
Do not publish the server directly to the LAN or internet without an explicitly
reviewed network and TLS configuration.

To terminate TLS in FetchX, place the PEM certificate chain and private key in
the configured certificate directory (default `./certs`, mounted read-only at
`/certs`) and set both variables:

```text
FETCHX_TLS_CERTIFICATE=/certs/server-cert.pem
FETCHX_TLS_PRIVATE_KEY=/certs/server-key.pem
```

TLS is enabled only when both values are present; a partial configuration fails
startup. Never bake certificates or keys into the image. For a TLS healthcheck,
`FETCHX_HEALTHCHECK_HOST` must match a certificate DNS name or IP subject
alternative name. If the certificate uses a private CA, set
`FETCHX_HEALTHCHECK_CA` to a mounted CA PEM file. The healthcheck resolves that
name to container loopback and verifies the certificate normally; it does not
disable certificate validation.

The host-side published port can be changed with `FETCHX_HOST_PORT`. Change the
container port with `FETCHX_PORT`; Compose maps both the published and target
ports consistently. `FETCHX_DATABASE` and `FETCHX_DOWNLOADS` can select other
paths inside the persistent `/data` volume. Keep both paths on persistent
storage.

## NAS storage and permissions

The `fetchx-data` named volume persists the SQLite database, download payloads,
and runtime files under `/data`. The container runs as UID/GID `10001`, drops
all Linux capabilities, uses `no-new-privileges`, and has a read-only root
filesystem. Only `/data` and a small no-exec `/tmp` are writable. The optional
certificate mount is read-only.

Use the NAS container manager's persistent named-volume support or adapt the
Compose volume to a dedicated host directory. For a bind mount, ensure the
mounted directory and its subdirectories are writable by UID/GID `10001`; do
not make the storage world-writable. Keep the database and downloads on durable
storage, not a temporary container layer.

Compose sets conservative per-container defaults of 1 GiB memory, two CPUs,
and 256 processes. Adjust these limits for the NAS workload; large downloads
and multiple concurrent transfers may need more memory, storage, and network
capacity.

## Backup, restore, upgrades, and shutdown

Stop the server before making a filesystem-level backup so SQLite and downloads
are captured consistently:

```sh
mkdir -p ./fetchx-data-backup
docker compose stop fetchx
docker compose cp fetchx:/data/. ./fetchx-data-backup/
docker compose start fetchx
```

Keep the backup in protected storage because it contains download data and the
SQLite authentication verifier and authorization configuration. The bearer
token itself is not stored in the database.

To restore, stop the current service and create a separate Compose project so
the backup is restored into a new, empty volume instead of merged with live
data:

```sh
docker compose stop fetchx
docker compose -p fetchx-restore create fetchx
docker compose -p fetchx-restore cp ./fetchx-data-backup/. fetchx:/data/
docker compose -p fetchx-restore run --rm --no-deps --user 0:0 \
  --entrypoint /bin/chown fetchx -R 10001:10001 /data
docker compose -p fetchx-restore up -d
docker compose -p fetchx-restore ps
```

The restore project uses its own named volume and keeps the original volume
intact. Verify `/health` and authenticated access before using the restored
instance; the original service must remain stopped to avoid a host-port
collision or concurrent database access. Do not merge a backup into a live data
directory or run two server instances against the same SQLite database. Test
backup restoration periodically on a separate volume.

For upgrades, back up first and use one server instance per database. Embedded
forward migrations run automatically at startup; there is no automatic database
downgrade. Retain the prior image and a verified pre-upgrade backup for recovery.
Compose sends `SIGTERM` and allows 30 seconds for graceful shutdown before it
forces container termination.

## Security behavior

The container uses the existing authentication, authorization, Origin validation,
WebSocket ticket, request bounds, and downloader URL safety policies. Docker
networking does not make loopback, private, or internal HTTP targets safe; the
existing SSRF protections remain enabled. Do not add network exceptions or pass
bearer credentials in URLs. Host port publication is loopback-only by default.

The image packages the FetchX CLI/server, not optional helper executables such as
`aria2c` or `yt-dlp`. Workflows that require one of those tools need it to be
provisioned and configured separately; no helper is downloaded or installed by
container startup. Native media processing does not require FFmpeg.
