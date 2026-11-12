import { useEffect, useState } from "react";
import type { FormEvent } from "react";

import {
  cancelMediaProcess,
  getMediaProcessForDownload,
  getTorrentDownloadMetadata,
  getTorrentTelemetry,
  listenForMediaProcessUpdates,
  restartAria2HttpWithNative,
  restartAria2TorrentFileWithNative,
  restartAria2TorrentWithNative,
  inspectTorrentFile,
  selectTorrentFiles,
  setTorrentDirectorySelected,
  setTorrentFileSelected,
  setTorrentSelectionAll,
  startMediaRemux,
} from "../api/downloads";
import type { Download, DownloadSchedule, MediaProcessJob } from "../api/types";
import { errorMessage, formatBytes, formatDatetime, formatEta, formatPercent, formatRelativeTime, formatSpeed } from "../lib/format";
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

const MEDIA_EXTENSIONS = new Set([
  "aac", "avi", "flac", "m3u8", "m4a", "m4v", "mkv", "mov", "mp3", "mp4",
  "mpd", "mpeg", "mpg", "ogg", "ts", "wav", "webm", "wma", "wmv",
]);

const REMUXABLE_EXTENSIONS = new Set([
  "aac", "avi", "flac", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "mpeg",
  "mpg", "ogg", "ts", "wav", "webm", "wma", "wmv",
]);

function isHttpSource(url: string): boolean {
  try {
    return ["http:", "https:"].includes(new URL(url).protocol);
  } catch {
    return false;
  }
}

function isMediaHttpSource(url: string): boolean {
  try {
    const path = new URL(url).pathname;
    return MEDIA_EXTENSIONS.has(path.split("/").pop()?.split(".").pop()?.toLowerCase() ?? "");
  } catch {
    return true;
  }
}

function safeSourceSuggestion(url: string): string {
  try {
    const parsed = new URL(url);
    return ["http:", "https:"].includes(parsed.protocol)
      && !parsed.username
      && !parsed.password
      && !parsed.hash
      && !isMediaHttpSource(url)
      ? url
      : "";
  } catch {
    return "";
  }
}

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
  const isTorrent = download.engine === "native_torrent"
    && (download.url.startsWith("magnet:") || download.url.startsWith("torrent+file:"));
  const isAria2 = download.engine === "aria2";
  const [restartOpen, setRestartOpen] = useState(false);
  const [restartUrl, setRestartUrl] = useState("");
  const [restartDestination, setRestartDestination] = useState("");
  const [restartConfirmedDirectFile, setRestartConfirmedDirectFile] = useState(false);
  const [restartLoading, setRestartLoading] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const [restartedDownloadId, setRestartedDownloadId] = useState<string | null>(null);
  const [torrentRestartOpen, setTorrentRestartOpen] = useState(false);
  const [torrentRestartSourceType, setTorrentRestartSourceType] = useState<"magnet" | "file">("magnet");
  const [torrentRestartMagnet, setTorrentRestartMagnet] = useState("");
  const [torrentRestartFilePath, setTorrentRestartFilePath] = useState("");
  const [torrentRestartFileInspection, setTorrentRestartFileInspection] =
    useState<Awaited<ReturnType<typeof inspectTorrentFile>> | null>(null);
  const [torrentRestartSelectedFiles, setTorrentRestartSelectedFiles] = useState<number[]>([]);
  const [torrentRestartSelectionConfirmed, setTorrentRestartSelectionConfirmed] = useState(false);
  const [torrentRestartDestination, setTorrentRestartDestination] = useState("");
  const [torrentRestartConfirmed, setTorrentRestartConfirmed] = useState(false);
  const [torrentRestartLoading, setTorrentRestartLoading] = useState(false);
  const [mediaJob, setMediaJob] = useState<MediaProcessJob | null>(null);
  const [mediaContainer, setMediaContainer] = useState<"mp4" | "mkv" | "webm">("mp4");
  const [mediaLoading, setMediaLoading] = useState(false);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [torrentRestartError, setTorrentRestartError] = useState<string | null>(null);
  const [restartedTorrentDownloadId, setRestartedTorrentDownloadId] = useState<string | null>(null);
  const [torrentTelemetry, setTorrentTelemetry] = useState<TorrentTelemetry | null>(null);
  const [torrentTelemetryError, setTorrentTelemetryError] = useState<string | null>(null);
  const [torrentMetadata, setTorrentMetadata] = useState<TorrentDownloadMetadata | null>(null);
  const [torrentMetadataError, setTorrentMetadataError] = useState<string | null>(null);

  useEffect(() => {
    setRestartOpen(false);
    setRestartError(null);
    setRestartedDownloadId(null);
    setTorrentRestartOpen(false);
    setTorrentRestartError(null);
    setRestartedTorrentDownloadId(null);
  }, [download.id]);

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
  const canRestartAria2 = isAria2
    && isHttpSource(download.url)
    && !isMediaHttpSource(download.url)
    && download.status !== "completed"
    && download.status !== "stopped"
    && !MEDIA_EXTENSIONS.has(download.filename.split(".").pop()?.toLowerCase() ?? "");
  const canRestartAria2Torrent = isAria2
    && (download.url.startsWith("magnet:") || download.url.startsWith("torrent+file:"))
    && download.status !== "completed"
    && download.status !== "seeding"
    && download.status !== "stopped";

  const startNativeRestart = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setRestartLoading(true);
    setRestartError(null);
    setRestartedDownloadId(null);
    try {
      const created = await restartAria2HttpWithNative(
        download.id,
        restartUrl,
        restartDestination,
        restartConfirmedDirectFile,
      );
      setRestartedDownloadId(created.id);
    } catch (cause) {
      setRestartError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRestartLoading(false);
    }
  };

  const startNativeTorrentRestart = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setTorrentRestartLoading(true);
    setTorrentRestartError(null);
    setRestartedTorrentDownloadId(null);
    try {
      const created = torrentRestartSourceType === "magnet"
        ? await restartAria2TorrentWithNative(
            download.id,
            torrentRestartMagnet,
            torrentRestartDestination,
            torrentRestartConfirmed,
          )
        : await restartAria2TorrentFileWithNative(
            download.id,
            torrentRestartFilePath,
            torrentRestartDestination,
            torrentRestartSelectedFiles,
            torrentRestartConfirmed,
            torrentRestartSelectionConfirmed,
          );
      setRestartedTorrentDownloadId(created.id);
    } catch (cause) {
      setTorrentRestartError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setTorrentRestartLoading(false);
    }
  };

  const inspectTorrentRestartFile = async () => {
    setTorrentRestartError(null);
    setTorrentRestartFileInspection(null);
    setTorrentRestartSelectedFiles([]);
    setTorrentRestartSelectionConfirmed(false);
    try {
      const inspected = await inspectTorrentFile(torrentRestartFilePath.trim());
      setTorrentRestartFileInspection(inspected);
    } catch (cause) {
      setTorrentRestartError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    setMediaJob(null);
    setMediaError(null);
    void (async () => {
      try {
        const stop = await listenForMediaProcessUpdates((job) => {
          if (job.download_id === download.id) setMediaJob(job);
        });
        if (!active) {
          stop();
          return;
        }
        unlisten = stop;
        setMediaJob(await getMediaProcessForDownload(download.id));
      } catch (cause) {
        if (active) setMediaError(errorMessage(cause));
      }
    })();
    return () => {
      active = false;
      unlisten?.();
    };
  }, [download.id]);

  const remux = async () => {
    setMediaLoading(true);
    setMediaError(null);
    try {
      setMediaJob(await startMediaRemux(download.id, mediaContainer));
    } catch (cause) {
      setMediaError(errorMessage(cause));
    } finally {
      setMediaLoading(false);
    }
  };

  const cancelRemux = async () => {
    if (!mediaJob) return;
    setMediaError(null);
    try {
      const cancelled = await cancelMediaProcess(mediaJob.job_id);
      if (!cancelled) setMediaError("This remux job is no longer active.");
    } catch (cause) {
      setMediaError(errorMessage(cause));
    }
  };

  const fileExtension = download.filename.split(".").pop()?.toLowerCase() ?? "";
  const canRemux = download.status === "completed" && REMUXABLE_EXTENSIONS.has(fileExtension);
  const mediaJobActive = mediaJob?.status === "preparing" || mediaJob?.status === "processing";
  const mediaProgress = mediaJob?.status === "completed"
    ? 100
    : mediaJob?.duration_seconds && mediaJob.duration_seconds > 0
      ? Math.min(100, (mediaJob.output_time_seconds / mediaJob.duration_seconds) * 100)
      : undefined;

  return (
    <aside className="detail-panel" aria-label="Download details">
      <div className="detail-header">
        <h2 className="detail-title" title={download.filename}>{download.filename}</h2>
        <button className="icon-button" aria-label="Close details" onClick={onClose}>×</button>
      </div>

      <div className="detail-body">
        {isAria2 && download.status !== "completed" && (
          <section className="detail-section" aria-label="aria2 recovery">
            <p className="detail-section-title">Legacy aria2 job</p>
            <p className="field-hint">
              Legacy aria2 job cannot resume because the external engine was removed. Its saved record and files remain unchanged.
            </p>
            {canRestartAria2 && (
              <div className="aria2-native-restart">
                {!restartOpen ? (
                  <button
                    className="button button-secondary"
                    onClick={() => {
                      setRestartUrl(safeSourceSuggestion(download.url));
                      setRestartDestination("");
                      setRestartConfirmedDirectFile(false);
                      setRestartError(null);
                      setRestartedDownloadId(null);
                      setRestartOpen(true);
                    }}
                  >
                    Restart with native
                  </button>
                ) : (
                  <form onSubmit={(event) => void startNativeRestart(event)}>
                    <label className="field-label" htmlFor={`restart-url-${download.id}`}>
                      Current HTTP(S) URL
                    </label>
                    <input
                      id={`restart-url-${download.id}`}
                      className="field-input"
                      type="url"
                      required
                      value={restartUrl}
                      disabled={restartLoading || restartedDownloadId !== null}
                      onChange={(event) => setRestartUrl(event.target.value)}
                    />
                    <p className="field-hint">
                      The saved URL is only a suggestion. Confirm or enter its current full URL.
                    </p>
                    <label className="field-label" htmlFor={`restart-destination-${download.id}`}>
                      New absolute destination
                    </label>
                    <input
                      id={`restart-destination-${download.id}`}
                      className="field-input"
                      type="text"
                      required
                      value={restartDestination}
                      disabled={restartLoading || restartedDownloadId !== null}
                      onChange={(event) => setRestartDestination(event.target.value)}
                      placeholder="Enter a new full file path"
                    />
                    <p className="field-hint">
                      Must be a different, unused path. The old output, partial data, and aria2 sidecar are never reused.
                    </p>
                    <label className="field-checkbox">
                      <input
                        type="checkbox"
                        checked={restartConfirmedDirectFile}
                        disabled={restartLoading || restartedDownloadId !== null}
                        onChange={(event) => setRestartConfirmedDirectFile(event.target.checked)}
                      />
                      <span>
                        I confirm this is a direct file download, not a torrent, Metalink, or media/streaming job.
                      </span>
                    </label>
                    {restartError && <p className="detail-error-message">{restartError}</p>}
                    {restartedDownloadId && (
                      <p role="status">
                        Created a separate native download ({restartedDownloadId}). The aria2 job remains unchanged.
                      </p>
                    )}
                    <div className="detail-actions">
                      <button
                        className="button button-primary"
                        type="submit"
                        disabled={
                          restartLoading ||
                          restartedDownloadId !== null ||
                          !restartConfirmedDirectFile
                        }
                      >
                        {restartLoading ? "Creating…" : "Create separate native download"}
                      </button>
                      <button
                        className="button button-secondary"
                        type="button"
                        disabled={restartLoading}
                        onClick={() => setRestartOpen(false)}
                      >
                        Cancel
                      </button>
                    </div>
                  </form>
                )}
              </div>
            )}
            {canRestartAria2Torrent && (
              <div className="aria2-native-restart">
                {!torrentRestartOpen ? (
                  <button
                    className="button button-secondary"
                    onClick={() => {
                      setTorrentRestartMagnet("");
                      setTorrentRestartDestination("");
                      setTorrentRestartConfirmed(false);
                      setTorrentRestartError(null);
                      setRestartedTorrentDownloadId(null);
                      setTorrentRestartOpen(true);
                    }}
                  >
                    Restart torrent with native
                  </button>
                ) : (
                  <form onSubmit={(event) => void startNativeTorrentRestart(event)}>
                    <label className="field-label" htmlFor={`torrent-restart-source-type-${download.id}`}>
                      Current torrent source
                    </label>
                    <select
                      id={`torrent-restart-source-type-${download.id}`}
                      className="field-input"
                      value={torrentRestartSourceType}
                      disabled={torrentRestartLoading || restartedTorrentDownloadId !== null}
                      onChange={(event) => {
                        setTorrentRestartSourceType(event.target.value as "magnet" | "file");
                        setTorrentRestartFileInspection(null);
                        setTorrentRestartSelectedFiles([]);
                        setTorrentRestartSelectionConfirmed(false);
                      }}
                    >
                      <option value="magnet">Magnet URI</option>
                      <option value="file">Local .torrent file</option>
                    </select>
                    {torrentRestartSourceType === "magnet" ? (
                      <>
                        <label className="field-label" htmlFor={`torrent-restart-magnet-${download.id}`}>
                          Current magnet URI
                        </label>
                        <input
                          id={`torrent-restart-magnet-${download.id}`}
                          className="field-input"
                          type="text"
                          required
                          value={torrentRestartMagnet}
                          disabled={torrentRestartLoading || restartedTorrentDownloadId !== null}
                          onChange={(event) => setTorrentRestartMagnet(event.target.value)}
                        />
                        <p className="field-hint">
                          Enter a current v1 magnet URI. Hex and base32 btih values are accepted; FetchX verifies the info hash against the saved aria2 torrent.
                        </p>
                      </>
                    ) : (
                      <>
                        <label className="field-label" htmlFor={`torrent-restart-file-${download.id}`}>
                          Current .torrent file path
                        </label>
                        <input
                          id={`torrent-restart-file-${download.id}`}
                          className="field-input"
                          type="text"
                          required
                          value={torrentRestartFilePath}
                          disabled={torrentRestartLoading || restartedTorrentDownloadId !== null}
                          onChange={(event) => {
                            setTorrentRestartFilePath(event.target.value);
                            setTorrentRestartFileInspection(null);
                            setTorrentRestartSelectedFiles([]);
                            setTorrentRestartSelectionConfirmed(false);
                          }}
                          placeholder="Path to a current .torrent file"
                        />
                        <button
                          className="button button-secondary"
                          type="button"
                          disabled={!torrentRestartFilePath.trim() || torrentRestartLoading}
                          onClick={() => void inspectTorrentRestartFile()}
                        >
                          Inspect and review files
                        </button>
                        {torrentRestartFileInspection && (
                          <div className="torrent-file-list">
                            <p>
                              {torrentRestartFileInspection.name} · {formatBytes(torrentRestartFileInspection.total_size)}
                            </p>
                            {torrentRestartFileInspection.files.map((file) => (
                              <label className="field-checkbox" key={file.index}>
                                <input
                                  type="checkbox"
                                  checked={torrentRestartSelectedFiles.includes(file.index)}
                                  disabled={torrentRestartLoading || restartedTorrentDownloadId !== null}
                                  onChange={(event) => {
                                    setTorrentRestartSelectedFiles((current) =>
                                      event.target.checked
                                        ? [...current, file.index]
                                        : current.filter((index) => index !== file.index),
                                    );
                                    setTorrentRestartSelectionConfirmed(false);
                                  }}
                                />
                                <span>{file.path} · {formatBytes(file.size)}</span>
                              </label>
                            ))}
                            <label className="field-checkbox">
                              <input
                                type="checkbox"
                                checked={torrentRestartSelectionConfirmed}
                                disabled={
                                  torrentRestartLoading ||
                                  restartedTorrentDownloadId !== null ||
                                  torrentRestartSelectedFiles.length === 0
                                }
                                onChange={(event) => setTorrentRestartSelectionConfirmed(event.target.checked)}
                              />
                              <span>I reviewed this file list and selected the content for the new native job.</span>
                            </label>
                          </div>
                        )}
                      </>
                    )}
                    <label className="field-label" htmlFor={`torrent-restart-destination-${download.id}`}>
                      New destination directory
                    </label>
                    <input
                      id={`torrent-restart-destination-${download.id}`}
                      className="field-input"
                      type="text"
                      required
                      value={torrentRestartDestination}
                      disabled={torrentRestartLoading || restartedTorrentDownloadId !== null}
                      onChange={(event) => setTorrentRestartDestination(event.target.value)}
                      placeholder="Absolute directory inside FetchX downloads"
                    />
                    <p className="field-hint">
                      Choose a new, non-existing directory. The old torrent output, partial files, and aria2 sidecar are never reused.
                    </p>
                    <label className="field-checkbox">
                      <input
                        type="checkbox"
                        checked={torrentRestartConfirmed}
                        disabled={torrentRestartLoading || restartedTorrentDownloadId !== null}
                        onChange={(event) => setTorrentRestartConfirmed(event.target.checked)}
                      />
                      <span>I confirm this is a current source for the saved aria2 torrent. The old job and its files will remain unchanged.</span>
                    </label>
                    {torrentRestartError && <p className="detail-error-message">{torrentRestartError}</p>}
                    {restartedTorrentDownloadId && (
                      <p role="status">
                        Created separate native torrent {restartedTorrentDownloadId}. Open that job to review its files and confirm a fresh selection; the aria2 job remains unchanged.
                      </p>
                    )}
                    <div className="detail-actions">
                      <button
                        className="button button-primary"
                        type="submit"
                        disabled={
                          torrentRestartLoading ||
                          restartedTorrentDownloadId !== null ||
                          !torrentRestartConfirmed ||
                          (torrentRestartSourceType === "file" &&
                            (!torrentRestartFileInspection ||
                              torrentRestartSelectedFiles.length === 0 ||
                              !torrentRestartSelectionConfirmed))
                        }
                      >
                        {torrentRestartLoading ? "Creating…" : "Create separate native torrent"}
                      </button>
                      <button
                        className="button button-secondary"
                        type="button"
                        disabled={torrentRestartLoading}
                        onClick={() => setTorrentRestartOpen(false)}
                      >
                        Cancel
                      </button>
                    </div>
                  </form>
                )}
              </div>
            )}
          </section>
        )}

        {canRemux && (
          <section className="detail-section" aria-label="Media remux">
            <p className="detail-section-title">Remux media</p>
            <p className="field-hint">
              Copies the existing streams into another container without re-encoding.
              The source is preserved and the new file is saved beside it. Closing FetchX cancels the remux.
            </p>
            <label className="field-label" htmlFor={`remux-container-${download.id}`}>
              Output container
            </label>
            <select
              id={`remux-container-${download.id}`}
              className="field-input"
              value={mediaContainer}
              disabled={mediaLoading || mediaJobActive}
              onChange={(event) =>
                setMediaContainer(event.target.value as "mp4" | "mkv" | "webm")
              }
            >
              <option value="mp4">MP4</option>
              <option value="mkv">Matroska (MKV)</option>
              <option value="webm">WebM</option>
            </select>
            <div className="detail-actions">
              <button
                className="button button-primary"
                disabled={mediaLoading || mediaJobActive}
                onClick={() => void remux()}
              >
                {mediaLoading
                  ? "Starting…"
                  : mediaJob?.status === "completed"
                    ? "Remux again"
                    : "Remux"}
              </button>
              {mediaJobActive && (
                <button
                  className="button button-secondary"
                  onClick={() => void cancelRemux()}
                >
                  Cancel remux
                </button>
              )}
            </div>
            {mediaJob && (
              <div role="status" aria-live="polite">
                <p className="field-hint">
                  Remux {mediaJob.status}
                  {mediaJob.status === "processing" && (
                    <>
                      {" · "}
                      {formatEta(Math.floor(mediaJob.output_time_seconds))}
                      {mediaJob.duration_seconds !== null
                        ? ` / ${formatEta(Math.floor(mediaJob.duration_seconds))}`
                        : ""}
                    </>
                  )}
                  {mediaJob.output_size_bytes !== null
                    ? ` · ${formatBytes(mediaJob.output_size_bytes)}`
                    : ""}
                </p>
                {(mediaJobActive || mediaJob.status === "completed") && (
                  <progress
                    className="progress-track"
                    aria-label="Remux progress"
                    value={mediaProgress}
                    max={100}
                  />
                )}
                {mediaJob.output_path && <code>{mediaJob.output_path}</code>}
                {mediaJob.error && (
                  <p className="detail-error-message">{mediaJob.error}</p>
                )}
              </div>
            )}
            {mediaError && <p className="detail-error-message">{mediaError}</p>}
          </section>
        )}

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
              <p className="field-hint">Live telemetry is unavailable for this native torrent job.</p>
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
              {!isAria2 && canChangePriority(download.status) ? (
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
        {!isAria2 && canSchedule(download.status) && (
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
          {!isAria2 && canPause(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onPause(download.id)}
            >
              Pause
            </button>
          )}
          {!isAria2 && canResume(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onResume(download.id)}
            >
              Resume
            </button>
          )}
          {!isAria2 && canRetry(download.status) && (
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onRetry(download.id)}
            >
              Retry
            </button>
          )}
          {!isAria2 && canCancel(download.status) && (
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
