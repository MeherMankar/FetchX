FROM rust:1.88-bookworm AS builder

WORKDIR /build

RUN apt-get update \
    && apt-get install --no-install-recommends -y build-essential cmake pkg-config perl \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY crates ./crates
COPY apps/desktop/src-tauri ./apps/desktop/src-tauri
COPY apps/browser/native-host ./apps/browser/native-host

RUN cargo build --release --locked --package fetchx --bin fetchx

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data/db /data/downloads /data/config/FetchX/runtime /certs \
    && chown -R 10001:10001 /data

COPY --from=builder /build/target/release/fetchx /usr/local/bin/fetchx
COPY docker/entrypoint.sh /tmp/fetchx-entrypoint
COPY docker/healthcheck.sh /tmp/fetchx-healthcheck
RUN sed -i 's/\r$//' /tmp/fetchx-entrypoint /tmp/fetchx-healthcheck \
    && install -m 0555 /tmp/fetchx-entrypoint /usr/local/bin/fetchx-entrypoint \
    && install -m 0555 /tmp/fetchx-healthcheck /usr/local/bin/fetchx-healthcheck \
    && rm -f /tmp/fetchx-entrypoint /tmp/fetchx-healthcheck

ENV HOME=/data \
    XDG_DATA_HOME=/data/config \
    RUST_LOG=info

USER 10001:10001
WORKDIR /data

EXPOSE 39800
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/fetchx-entrypoint"]
CMD ["server"]
