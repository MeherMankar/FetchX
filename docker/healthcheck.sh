#!/bin/sh
set -eu

port=${FETCHX_PORT:-39800}
host=${FETCHX_HEALTHCHECK_HOST:-127.0.0.1}
scheme=http
set -- --fail --silent --show-error --max-time 4

if [ -n "${FETCHX_TLS_CERTIFICATE:-}" ]; then
    scheme=https
    set -- "$@" --resolve "${host}:${port}:127.0.0.1"
    if [ -n "${FETCHX_HEALTHCHECK_CA:-}" ]; then
        set -- "$@" --cacert "$FETCHX_HEALTHCHECK_CA"
    fi
fi

exec curl "$@" "${scheme}://${host}:${port}/health"
