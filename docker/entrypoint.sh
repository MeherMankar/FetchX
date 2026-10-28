#!/bin/sh
set -eu

fetchx=/usr/local/bin/fetchx
database=${FETCHX_DATABASE:-/data/db/fetchx.db}
downloads=${FETCHX_DOWNLOADS:-/data/downloads}

if [ "$#" -gt 0 ]; then
    if [ "$1" != "server" ]; then
        exec "$fetchx" "$@"
    fi
    shift
fi

if [ "$#" -gt 0 ]; then
    case "$1" in
        init-auth|rotate-auth|show-role|set-role)
            action=$1
            shift
            exec "$fetchx" server --database "$database" "$action" "$@"
            ;;
    esac
fi

set -- \
    --bind "${FETCHX_BIND:-0.0.0.0}" \
    --port "${FETCHX_PORT:-39800}" \
    --database "$database" \
    --downloads "$downloads" \
    "$@"

if [ -n "${FETCHX_TLS_CERTIFICATE:-}" ] || [ -n "${FETCHX_TLS_PRIVATE_KEY:-}" ]; then
    if [ -z "${FETCHX_TLS_CERTIFICATE:-}" ] || [ -z "${FETCHX_TLS_PRIVATE_KEY:-}" ]; then
        echo "FETCHX_TLS_CERTIFICATE and FETCHX_TLS_PRIVATE_KEY must be configured together" >&2
        exit 2
    fi
    set -- "$@" --tls-certificate "$FETCHX_TLS_CERTIFICATE" --tls-private-key "$FETCHX_TLS_PRIVATE_KEY"
fi

if [ -n "${FETCHX_ALLOWED_ORIGINS:-}" ]; then
    set -f
    previous_ifs=$IFS
    IFS=,
    for origin in $FETCHX_ALLOWED_ORIGINS; do
        if [ -n "$origin" ]; then
            set -- "$@" --allowed-origin "$origin"
        fi
    done
    IFS=$previous_ifs
fi

exec "$fetchx" server "$@"
