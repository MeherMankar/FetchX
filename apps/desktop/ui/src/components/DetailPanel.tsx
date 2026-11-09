import { useEffect, useState } from "react";

import {
  getTorrentDownloadMetadata,
  getTorrentTelemetry,
  selectTorrentFiles,
  setTorrentDirectorySelected,
  setTorrentFileSelected,
  setTorrentSelectionAll,
} from "../api/downloads";
import type { Download, DownloadSchedule } from "../api/types";
import { formatBytes, formatDatetime, formatEta, formatPercent, formatRelativeTime, formatSpeed } from "../lib/format";
import { canSchedule, canChangePriority, canPause, canResume, canCancel, canRetry } from "../lib/status";
import { ScheduleEditor } from "./ScheduleEditor";
import type {
  DownloadPriority,
  TorrentDownloadMetadata,
  TorrentTelemetry,
} from "../api/types";

const ENGINE_LABELS: Record<string, string> = {
  direct_http: "Direct HTTP",
  aria2: "aria2",
  native_torrent: "Native torrent",
  yt_dlp: "yt-dlp",
};

interface DetailPanelProps {
  download: Download;
  schedule: DownloadSchedule | null;
  busy: boolean;
  onClose: () => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  onRetry: (id: string) => void;
  onPriorityChange: (id: string, priority: DownloadPriority) => void;
  onSetSchedule: (id: string, ms: number) => Promise<void>;
  onClearSchedule: (id: string) => Promise<void>;
  onError: (message: string) => void;
  onStopSeeding: (id: string) => Promise<void>;
}

export function DetailPanel({
  download,
  schedule,
  busy,
  onClose,
  onPause,
  onResume,
  onCancel,
  onDelete,
  onRetry,
  onPriorityChange,
  onSetSchedule,
  onClearSchedule,
  onError,
  onStopSeeding,
}: DetailPanelProps) {
  const { progress } = download;
  const pct = progress.percentage_basis_points;
  const isTorrent = (download.engine === "aria2" || download.engine === "native_torrent")
    && (download.url.startsWith("magnet:") || download.url.startsWith("torrent+file:"));
  const [torrentTelemetry, setTorrentTelemetry] = useState<TorrentTelemetry | null>(null);
  const [torrentTelemetryError, setTorrentTelemetryError] = useState<string | null>(null);
  const [torrentMetadata, setTorrentMetadata] = useState<TorrentDownloadMetadata | null>(null);
  const [torrentMetadataError, setTorrentMetadataError] = useState<string | null>(null);

  useEffect(() => {
    setTorrentTelemetry(null);
    setTorrentTelemetryError(null);
    setTorrentMetadata(null);
    setTorrentMetadataError(null);
    if (!isTorrent) return;

    let active = true;
    const refresh = async () => {
      const [telemetryResult, metadataResult] = await Promise.allSettled([
        getTorrentTelemetry(download.id),
        getTorrentDownloadMetadata(download.id),
      ]);
      if (!active) return;

      if (telemetryResult.status === "fulfilled") {
        setTorrentTelemetry(telemetryResult.value);
        setTorrentTelemetryError(null);
      } else {
        setTorrentTelemetryError(
          telemetryResult.reason instanceof Error
            ? telemetryResult.reason.message
            : String(telemetryResult.reason),
        );
      }
      if (metadataResult.status === "fulfilled") {
        setTorrentMetadata(metadataResult.value);
        setTorrentMetadataError(null);
      } else {
        setTorrentMetadataError(
          metadataResult.reason instanceof Error
            ? metadataResult.reason.message
            : String(metadataResult.reason),
        );
      }
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 2000);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, [download.id, isTorrent]);

  const updateMagnetSelection = async (update: () => Promise<void>) => {
    try {
      await update();
      setTorrentMetadata(await getTorrentDownloadMetadata(download.id));
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const magnetSelectionPending = isTorrent
    && download.url.startsWith("magnet:")
    && download.status === "metadata_acquiring"
    && torrentMetadata?.metadata !== null;

  return (
    <aside className="detail-panel" aria-label="Download details">
      <div className="detail-header">
        <h2 className="detail-title" title={download.filename}>{download.filename}</h2>
        <button className="icon-button" aria-label="Close details" onClick={onClose}>×</button>
      </div>

      <div className="detail-body">
        {/* Status + progress */}
        <section className="detail-section">
          <div className="detail-row detail-status-row">
            <span className={`status status-${download.status}`}>{download.status}</span>
            {pct !== null && (
              <span className="detail-pct">{formatPercent(pct)}</span>
            )}
          </div>
          <progress
            className="progress-track"
            aria-label={`Progress: ${formatPercent(pct)}`}
            value={pct === null ? 0 : pct / 100}
            max={100}
          />
          <dl className="detail-dl">
            <dt>Downloaded</dt>
            <dd>
              {formatBytes(progress.downloaded_bytes)}
              {progress.total_bytes !== null ? ` / ${formatBytes(progress.total_bytes)}` : ""}
            </dd>
            {progress.speed_bytes_per_second !== null && (
              <>
                <dt>Speed</dt>
                <dd>{formatSpeed(progress.speed_bytes_per_second)}</dd>
              </>
            )}
            {progress.estimated_remaining_seconds !== null && (
              <>
                <dt>ETA</dt>
                <dd>{formatEta(progress.estimated_remaining_seconds)}</dd>
              </>
            )}
          </dl>
        </section>

        {isTorrent && (
          <section className="detail-section">
            <p className="detail-section-title">Torrent</p>
            {torrentTelemetryError && (
              <p className="detail-error-message">{torrentTelemetryError}</p>
            )}
            {torrentTelemetry ? (
              <>
                <dl className="detail-dl">
                  <dt>Peers</dt>
                  <dd>{torrentTelemetry.peer_count ?? "Unavailable"}</dd>
                  <dt>Completed peers</dt>
                  <dd>{torrentTelemetry.completed_peer_count ?? "Unavailable"}</dd>
                  <dt>Seeds</dt>
                  <dd>{torrentTelemetry.seed_count ?? "Unavailable"}</dd>
                  <dt>Download</dt>
                  <dd>{torrentTelemetry.download_speed_bps === null ? "Unavailable" : formatSpeed(torrentTelemetry.download_speed_bps)}</dd>
                  <dt>Upload</dt>
                  <dd>{torrentTelemetry.upload_speed_bps === null ? "Unavailable" : formatSpeed(torrentTelemetry.upload_speed_bps)}</dd>
                  <dt>Uploaded</dt>
                  <dd>{torrentTelemetry.uploaded_bytes === null ? "Unavailable" : formatBytes(torrentTelemetry.uploaded_bytes)}</dd>
                  <dt>Share ratio</dt>
                  <dd>{torrentTelemetry.share_ratio === null ? "Unavailable" : torrentTelemetry.share_ratio.toFixed(3)}</dd>
                  <dt>Seeding</dt>
                  <dd>{torrentTelemetry.seeding === null ? "Unavailable" : torrentTelemetry.seeding ? "Yes" : "No"}</dd>
                  <dt>Trackers</dt>
                  <dd>{torrentTelemetry.tracker_uris === null ? "Unavailable" : torrentTelemetry.tracker_uris.join(", ") || "None reported"}</dd>
                </dl>
                {torrentTelemetry.seeding === true && (
                  <button
                    className="button button-secondary"
                    disabled={busy}
                    onClick={() => void onStopSeeding(download.id)}
                  >
                    Stop seeding
                  </button>
                )}
              </>
            ) : (
              <p className="field-hint">Live telemetry is unavailable until aria2 reports the task.</p>
            )}
            {magnetSelectionPending && torrentMetadata?.metadata && (
              <section className="detail-section">
                <p className="detail-section-title">
                  Magnet files · {formatBytes(torrentMetadata.selected_total_bytes)} selected
                </p>
                <p className="field-hint">
                  {torrentMetadata.metadata.name} · {formatBytes(torrentMetadata.metadata.total_size)}
                </p>
                {Array.from(new Set(torrentMetadata.metadata.files.flatMap((file) => {
                  const directories: string[][] = [];
                  for (let index = 1; index < file.path.length; index += 1) {
                    directories.push(file.path.slice(0, index));
                  }
                  return directories.map((directory) => directory.join("/"));
                }))).map((directory) => (
                  <div className="detail-actions" key={directory}>
                    <span>{directory}</span>
                    <button
                      className="button button-secondary"
                      disabled={busy}
                      onClick={() => void updateMagnetSelection(
                        () => setTorrentDirectorySelected(download.id, directory, true),
                      )}
                    >
                      Select folder
                    </button>
                    <button
                      className="button button-secondary"
                      disabled={busy}
                      onClick={() => void updateMagnetSelection(
                        () => setTorrentDirectorySelected(download.id, directory, false),
                      )}
                    >
                      Deselect folder
                    </button>
                  </div>
                ))}
                <div className="detail-actions">
                  <button
                    className="button button-secondary"
                    disabled={busy}
                    onClick={() => void updateMagnetSelection(
                      () => setTorrentSelectionAll(download.id, true),
                    )}
                  >
                    Select all
                  </button>
                  <button
                    className="button button-secondary"
                    disabled={busy}
                    onClick={() => void updateMagnetSelection(
                      () => setTorrentSelectionAll(download.id, false),
                    )}
                  >
                    Select none
                  </button>
                </div>
                {torrentMetadata.metadata.files.map((file, index) => {
                  const path = file.path.join("/");
                  const selected = torrentMetadata.selected_file_indexes.includes(index);
                  return (
                    <label className="detail-row" key={`${path}:${index}`}>
                      <input
                        type="checkbox"
                        checked={selected}
                        disabled={busy}
                        onChange={(event) => void updateMagnetSelection(
                          () => setTorrentFileSelected(
                            download.id,
                            index,
                            event.currentTarget.checked,
                          ),
                        )}
                      />
                      <span>{path}</span>
                      <span>{formatBytes(file.size)}</span>
                    </label>
                  );
                })}
                <button
                  className="button button-primary"
                  disabled={busy || torrentMetadata.selected_file_indexes.length === 0}
                  onClick={() => void updateMagnetSelection(
                    () => selectTorrentFiles(
                      download.id,
                      torrentMetadata.selected_file_indexes,
                    ),
                  )}
                >
                  Start selected files
                </button>
              </section>
            )}
            {torrentMetadataError && (
              <p className="detail-error-message">{torrentMetadataError}</p>
            )}
          </section>
        )}

        {/* Error */}
        {download.error && (
          <section className="detail-section">
            <p className="detail-error-label">Error</p>
            <p className="detail-error-message">{download.error.message}</p>
            <p className="detail-error-code">Code: {download.error.code}</p>
          </section>
        )}

        {/* Source info */}
        <section className="detail-section">
          <dl className="detail-dl">
            <dt>URL</dt>
            <dd className="detail-url" title={download.url}>{download.url}</dd>
            <dt>Destination</dt>
            <dd className="detail-path" title={download.destination}>{download.destination}</dd>
            <dt>Engine</dt>
            <dd>{ENGINE_LABELS[download.engine] ?? download.engine}</dd>
            <dt>Priority</dt>
            <dd>
              {canChangePriority(download.status) ? (
                <>
                  <label className="visually-hidden" htmlFor={`detail-priority-${download.id}`}>
                    Priority
                  </label>
                  <select
                    id={`detail-priority-${download.id}`}
                    className="priority-select priority-select-sm"
                    value={download.priority}
                    disabled={busy}
                    onChange={(e) =>
                      onPriorityChange(download.id, e.target.value as DownloadPriority)
                    }
                  >
                    <option value="low">Low</option>
                    <option value="normal">Normal</option>
                    <option value="high">High</option>
                  </select>
                </>
              ) : (
                <span style={{ textTransform: "capitalize" }}>{download.priority}</span>
              )}
            </dd>
            <dt>Added</dt>
            <dd title={formatDatetime(download.created_at)}>{formatRelativeTime(download.created_at)}</dd>
            <dt>Updated</dt>
            <dd title={formatDatetime(download.updated_at)}>{formatRelativeTime(download.updated_at)}</dd>
          </dl>
        </section>

        {/* Schedule */}
        {canSchedule(download.status) && (
          <section className="detail-section">
            <p className="detail-section-title">Schedule</p>
            <ScheduleEditor
              downloadId={download.id}
              schedule={schedule}
              busy={busy}
              onSet={(ms) => onSetSchedule(download.id, ms)}
              onClear={() => onClearSchedule(download.id)}
              onError={onError}
            />
          </section>
        )}

        {/* Actions */}
        <section className="detail-section detail-actions">
          {canPause(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onPause(download.id)}
            >
              Pause
            </button>
          )}
          {canResume(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onResume(download.id)}
            >
              Resume
            </button>
          )}
          {canRetry(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onRetry(download.id)}
            >
              Retry
            </button>
          )}
          {canCancel(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onCancel(download.id)}
            >
              Cancel
            </button>
          )}
          <button
            className="button button-danger"
            disabled={busy}
            onClick={() => onDelete(download.id)}
          >
            Delete
          </button>
        </section>
      </div>
    </aside>
  );
}
