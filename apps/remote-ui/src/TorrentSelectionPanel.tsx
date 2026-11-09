import { useEffect, useReducer, useState } from "react";

import type { RemoteClient, RemoteDownload, RemoteNativeMagnetMetadata } from "./protocol";
import { initialTorrentSelectionState, reduceTorrentSelection } from "./torrentSelectionState";

interface TorrentSelectionPanelProps {
  client: RemoteClient;
  download: RemoteDownload;
  capabilities: Set<string>;
  busy: boolean;
  onConfirmed: () => void;
}

export function TorrentSelectionPanel({
  client,
  download,
  capabilities,
  busy,
  onConfirmed,
}: TorrentSelectionPanelProps) {
  const [state, dispatch] = useReducer(reduceTorrentSelection, initialTorrentSelectionState);
  const [confirming, setConfirming] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const canRead = capabilities.has("download.torrentMetadata");
  const canSelect = capabilities.has("download.selectTorrentFiles");

  useEffect(() => {
    if (!canRead) {
      dispatch({
        type: "unavailable",
        message: "This remote server does not advertise native torrent metadata support. Update the server to inspect magnet files.",
      });
      return;
    }

    let active = true;
    let timer: number | undefined;
    dispatch({ type: "loading" });

    const loadMetadata = async () => {
      try {
        const metadata = await client.call<RemoteNativeMagnetMetadata>(
          "download.torrentMetadata",
          { id: download.id },
        );
        if (!active) return;
        dispatch({ type: "metadata", metadata });
        if (
          metadata.metadata_state !== "ready"
          && download.status === "metadata_acquiring"
        ) {
          timer = window.setTimeout(() => void loadMetadata(), 1_500);
        }
      } catch (cause) {
        if (!active) return;
        dispatch({
          type: "failed",
          message: cause instanceof Error ? cause.message : "Could not load torrent metadata.",
        });
      }
    };

    void loadMetadata();
    return () => {
      active = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [canRead, client, download.id, download.status, reloadKey]);

  const confirmSelection = async () => {
    if (state.kind !== "ready" || !canSelect || state.selectedFileIndexes.length === 0 || confirming) return;
    setConfirming(true);
    dispatch({ type: "confirming" });
    try {
      await client.call("download.selectTorrentFiles", {
        id: download.id,
        info_hash: state.metadata.info_hash,
        selected_file_indexes: state.selectedFileIndexes,
      });
      dispatch({ type: "confirmed" });
      onConfirmed();
    } catch (cause) {
      const stale = cause instanceof Error
        && "code" in cause
        && cause.code === -32009;
      dispatch({
        type: "failed",
        message: stale
          ? "Torrent metadata changed. Refresh the file list before confirming."
          : cause instanceof Error ? cause.message : "Could not confirm torrent file selection.",
        stale,
      });
    } finally {
      setConfirming(false);
    }
  };

  return (
    <section className="torrent-selection-panel" aria-label="Torrent file selection">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">NATIVE MAGNET</p>
          <h3>{state.kind === "ready" || state.kind === "confirming" || state.kind === "confirmed"
            ? state.metadata.name ?? "Torrent files"
            : "Torrent files"}</h3>
        </div>
        {state.kind === "ready" && <span className="capability-chip">{state.metadata.files.length} files</span>}
      </div>
      {state.kind === "loading" && <p className="muted" role="status">Loading torrent metadata…</p>}
      {state.kind === "unavailable" && <p className="muted" role="status">{state.message}</p>}
      {state.kind === "acquiring" && (
        download.status === "failed"
          ? <p className="torrent-selection-error" role="alert">Magnet metadata acquisition stopped because this download failed.</p>
          : <p className="muted" role="status">Acquiring magnet metadata. This download will not transfer files until you confirm a selection.</p>
      )}
      {state.kind === "failed" && (
        <div className="torrent-selection-error" role="alert">
          <span>{state.message}</span>
          <button className="text-button" onClick={() => setReloadKey((key) => key + 1)}>
            {state.stale ? "Refresh files" : "Retry"}
          </button>
        </div>
      )}
      {(state.kind === "ready" || state.kind === "confirming") && (
        <>
          {download.status === "failed" && (
            <p className="torrent-selection-error" role="alert">
              Metadata acquisition stopped because this download failed. Retry is unavailable in the current remote protocol.
            </p>
          )}
          <div className="torrent-file-list">
            {state.metadata.files.map((file) => {
              const checked = state.selectedFileIndexes.includes(file.index);
              return (
                <label className="torrent-file-option" key={file.index}>
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={state.kind === "confirming" || busy || download.status !== "metadata_acquiring"}
                    onChange={(event) => dispatch({
                      type: "selection",
                      indexes: event.target.checked
                        ? [...state.selectedFileIndexes, file.index]
                        : state.selectedFileIndexes.filter((index) => index !== file.index),
                    })}
                  />
                  <span className="torrent-file-path">{file.path.join("/")}</span>
                  <span className="muted">{formatBytes(file.size)}</span>
                </label>
              );
            })}
          </div>
          {!canSelect && (
            <p className="muted small-note">
              This server can show torrent files but does not advertise file-selection confirmation.
            </p>
          )}
          <div className="torrent-selection-footer">
            <span className="muted">
              {state.selectedFileIndexes.length} selected · {formatBytes(selectedSize(state.metadata, state.selectedFileIndexes))}
            </span>
            <button
              className="primary-button"
              disabled={!canSelect || state.kind === "confirming" || confirming || busy
                || download.status !== "metadata_acquiring" || state.selectedFileIndexes.length === 0}
              onClick={() => void confirmSelection()}
            >
              {state.kind === "confirming" ? "Confirming…" : "Confirm files"}
            </button>
          </div>
          {state.kind === "ready" && state.error && <p className="torrent-selection-error">{state.error}</p>}
        </>
      )}
      {state.kind === "confirmed" && (
        <p className="muted" role="status">
          File selection confirmed. The shared queue will continue this download with the selected files only.
        </p>
      )}
    </section>
  );
}

function selectedSize(metadata: RemoteNativeMagnetMetadata, indexes: number[]): number {
  const selected = new Set(indexes);
  return metadata.files.reduce((total, file) => total + (selected.has(file.index) ? file.size : 0), 0);
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = -1;
  do {
    value /= 1024;
    unitIndex += 1;
  } while (value >= 1024 && unitIndex < units.length - 1);
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}
