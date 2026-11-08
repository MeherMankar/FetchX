import { useEffect, useRef, useState, type FormEvent } from "react";

import { getEngineStatus } from "../api/engine";
import {
  addTorrentDownload,
  addDownload,
  addMediaDownload,
  discoverYtDlpFormats,
  extractDownloadLinks,
  suggestDownloadFilename,
  suggestDownloadPackageName,
  inspectTorrentFile,
  scheduleDownload,
  setDownloadPriority,
} from "../api/downloads";
import type { DownloadPriority, EngineKind, EngineStatus, YtDlpFormat } from "../api/types";
import { errorMessage, formatBytes, msToDatetimeLocal } from "../lib/format";

interface AddDialogProps {
  initialUrl?: string;
  onAdded: () => void;
  onClose: () => void;
}

/** Display label and serde key for each engine option. */
const ENGINE_OPTIONS: { kind: EngineKind; label: string }[] = [
  { kind: "direct_http", label: "Direct HTTP" },
  { kind: "yt_dlp",      label: "yt-dlp" },
];

/**
 * Returns true when the given engine kind is runtime-available according to
 * the status object loaded from the backend.
 * Direct HTTP is always available; optional engines must be Available.
 */
function isEngineAvailable(kind: EngineKind, status: EngineStatus | null): boolean {
  if (!status) return kind === "direct_http"; // safe default while loading
  switch (kind) {
    case "direct_http": return status.direct_http.available;
    case "aria2":       return false;
    case "yt_dlp":      return status.yt_dlp.available;
    case "native_torrent": return true;
  }
}

function engineUnavailableReason(kind: EngineKind, status: EngineStatus | null): string | null {
  if (!status) return null;
  switch (kind) {
    case "direct_http": return null;
    case "aria2":
      if (!status.aria2.enabled) return "disabled in Settings";
      if (!status.aria2.available) return status.aria2.error ?? "unavailable";
      return null;
    case "yt_dlp":
      if (!status.yt_dlp.enabled) return "disabled in Settings";
      if (!status.yt_dlp.available) return status.yt_dlp.error ?? "unavailable";
      return null;
    case "native_torrent": return null;
  }
}

export function AddDialog({ initialUrl = "", onAdded, onClose }: AddDialogProps) {
  const [url, setUrl] = useState(initialUrl);
  const [packageName, setPackageName] = useState("");
  const [priority, setPriority] = useState<DownloadPriority>("normal");
  const [engineKind, setEngineKind] = useState<EngineKind>("direct_http");
  const [engineStatus, setEngineStatus] = useState<EngineStatus | null>(null);
  const [engineStatusLoading, setEngineStatusLoading] = useState(true);
  const [scheduleEnabled, setScheduleEnabled] = useState(false);
  const [scheduledAt, setScheduledAt] = useState(
    msToDatetimeLocal(Math.ceil(Date.now() / 60000) * 60000 + 3_600_000),
  );
  const [busy, setBusy] = useState(false);
  const [formats, setFormats] = useState<YtDlpFormat[] | null>(null);
  const [selectedFormatId, setSelectedFormatId] = useState("");
  const [audioFormat, setAudioFormat] = useState<"" | "mp3" | "m4a">("");
  const [torrentInspection, setTorrentInspection] = useState<Awaited<ReturnType<typeof inspectTorrentFile>> | null>(null);
  const [selectedTorrentFiles, setSelectedTorrentFiles] = useState<number[]>([]);
  const [formatsLoading, setFormatsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const urlInputRef = useRef<HTMLInputElement>(null);
  const isMagnet = url.trimStart().toLowerCase().startsWith("magnet:");
  const isTorrentFile = !url.includes("://") && /\.torrent$/i.test(url.trim());
  const isTorrent = isMagnet || isTorrentFile;

  useEffect(() => {
    if (packageName.trim() || !url.trim()) return;
    let cancelled = false;
    const firstUrl = extractDownloadLinks(url)
      .then((links) => links[0] ?? url.trim())
      .catch(() => url.trim().split(/\s+/)[0] ?? "");
    void Promise.all([firstUrl, suggestDownloadFilename(url)])
      .then(async ([targetUrl, filename]) => {
        if (!cancelled && targetUrl) {
          setPackageName(await suggestDownloadPackageName(targetUrl, filename));
        }
      })
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [url, packageName]);

  // Load engine availability once when the dialog opens.
  // No polling — one fetch on mount.
  useEffect(() => {
    let cancelled = false;
    void getEngineStatus()
      .then((status) => {
        if (!cancelled) {
          setEngineStatus(status);
          setEngineStatusLoading(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          // If the status fetch fails, fall back to Direct HTTP only.
          setEngineStatusLoading(false);
        }
      });
    return () => { cancelled = true; };
  }, []);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setError(null);

    if (scheduleEnabled) {
      const ms = new Date(scheduledAt).getTime();
      if (!Number.isFinite(ms) || ms <= Date.now()) {
        setError("Please choose a schedule time in the future.");
        return;
      }
    }
    if (isTorrentFile && torrentInspection && selectedTorrentFiles.length === 0) {
      setError("Select at least one torrent file.");
      return;
    }

    setBusy(true);
    try {
      // Pass the selected engine to the backend.
      // The backend re-validates availability — the frontend cannot bypass this check.
      // Omit engine_kind when Direct HTTP is selected to use the backend default.
      const bulkLinks = await extractDownloadLinks(url);
      const urlsToAdd = bulkLinks.length > 1 ? bulkLinks : [url];
      for (const targetUrl of urlsToAdd) {
        const download = isTorrentFile && torrentInspection
          ? await addTorrentDownload(url.trim(), selectedTorrentFiles)
          : engineKind === "yt_dlp" && !isTorrent
          ? await addMediaDownload(
              targetUrl,
              audioFormat ? undefined : selectedFormatId || undefined,
              audioFormat || formats?.find((format) => format.format_id === selectedFormatId)?.extension,
              audioFormat || undefined,
            )
          : await addDownload({
              url: targetUrl.trim(),
              package_name: packageName.trim() || undefined,
              engine_kind: isTorrent ? "native_torrent" : engineKind === "direct_http" ? undefined : engineKind,
            });

        if (priority !== "normal") {
          await setDownloadPriority(download.id, priority);
        }

        if (scheduleEnabled) {
          const ms = new Date(scheduledAt).getTime();
          await scheduleDownload(download.id, ms);
        }
      }

      onAdded();
    } catch (cause) {
      setError(errorMessage(cause));
      setBusy(false);
    }
  };

  const loadFormats = async () => {
    setError(null);
    setFormatsLoading(true);
    try {
      setFormats(await discoverYtDlpFormats(url));
      setSelectedFormatId("");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setFormatsLoading(false);
    }
  };

  const inspectTorrent = async () => {
    setError(null);
    setBusy(true);
    try {
      const metadata = await inspectTorrentFile(url.trim());
      setTorrentInspection(metadata);
      setSelectedTorrentFiles(metadata.files.map((file) => file.index));
    } catch (cause) {
      setTorrentInspection(null);
      setSelectedTorrentFiles([]);
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
    <div
      className="dialog-backdrop"
      onClick={handleBackdropClick}
      aria-modal="true"
      role="dialog"
      aria-labelledby="add-dialog-title"
    >
      <div className="dialog">
        <header className="dialog-header">
          <h2 id="add-dialog-title">Add download</h2>
          <button className="icon-button" aria-label="Close dialog" onClick={onClose}>
            ×
          </button>
        </header>

        <form className="dialog-body" onSubmit={(e) => void submit(e)}>
          {error && (
            <div className="dialog-error" role="alert">
              {error}
            </div>
          )}

          <div className="field">
            <label htmlFor="add-package" className="field-label">
              Package
            </label>
            <input
              id="add-package"
              className="field-input"
              value={packageName}
              onChange={(event) => setPackageName(event.target.value)}
              placeholder="FetchX Downloads"
              disabled={busy}
            />
            <p className="field-hint">
              Related downloads are stored together in this package folder.
            </p>
          </div>

          <div className="field">
            <label htmlFor="add-url" className="field-label">
              URL <span className="required" aria-hidden="true">*</span>
            </label>
            <input
              id="add-url"
              ref={urlInputRef}
              type="text"
              inputMode="url"
              className="field-input"
              placeholder="https://example.com/file.zip, magnet URI, or local .torrent path"
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                setFormats(null);
                setSelectedFormatId("");
                setTorrentInspection(null);
                setSelectedTorrentFiles([]);
              }}
              required
              disabled={busy || formatsLoading}
              autoFocus
            />
            <p className="field-hint">
              {isMagnet
                ? "Magnet links use FetchX's native BitTorrent engine."
                : isTorrentFile
                  ? "Local .torrent files use FetchX's native BitTorrent engine."
                : "HTTP and HTTPS URLs only. Credentials in URLs are not accepted."}
            </p>
            {isTorrentFile && (
              <div className="field">
                <button
                  className="button button-secondary"
                  type="button"
                  onClick={() => void inspectTorrent()}
                  disabled={busy || !url.trim()}
                >
                  Inspect torrent files
                </button>
                {torrentInspection && (
                  <div className="torrent-file-list">
                    <p>
                      {torrentInspection.name} · {formatBytes(torrentInspection.total_size)}
                    </p>
                    <p>
                      Selected: {selectedTorrentFiles.length} file(s) ·{" "}
                      {formatBytes(
                        torrentInspection.files
                          .filter((file) => selectedTorrentFiles.includes(file.index))
                          .reduce((total, file) => total + file.size, 0),
                      )}
                    </p>
                    <div className="button-row">
                      <button
                        className="button button-secondary"
                        type="button"
                        onClick={() => setSelectedTorrentFiles(torrentInspection.files.map((file) => file.index))}
                      >
                        Select all
                      </button>
                      <button
                        className="button button-secondary"
                        type="button"
                        onClick={() => setSelectedTorrentFiles([])}
                      >
                        Deselect all
                      </button>
                    </div>
                    {Array.from(new Set(
                      torrentInspection.files.flatMap((file) => {
                        const segments = file.path.split("/");
                        return segments.slice(0, -1).map((_, index) =>
                          segments.slice(0, index + 1).join("/"),
                        );
                      }),
                    )).map((directory) => {
                      const indexes = torrentInspection.files
                        .filter((file) => file.path.startsWith(`${directory}/`))
                        .map((file) => file.index);
                      const allSelected = indexes.every((index) => selectedTorrentFiles.includes(index));
                      return (
                        <button
                          className="button button-secondary"
                          type="button"
                          key={directory}
                          onClick={() => setSelectedTorrentFiles((current) => allSelected
                            ? current.filter((index) => !indexes.includes(index))
                            : Array.from(new Set([...current, ...indexes])).sort((a, b) => a - b))}
                        >
                          {allSelected ? "Deselect" : "Select"} {directory}
                        </button>
                      );
                    })}
                    {torrentInspection.files.map((file) => (
                      <label className="torrent-file-option" key={file.index}>
                        <input
                          type="checkbox"
                          checked={selectedTorrentFiles.includes(file.index)}
                          onChange={(event) => {
                            setSelectedTorrentFiles((current) => event.target.checked
                              ? [...current, file.index].sort((a, b) => a - b)
                              : current.filter((index) => index !== file.index));
                          }}
                        />
                        <span>{file.path}</span>
                        <span>{formatBytes(file.size)}</span>
                      </label>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="field">
            <label htmlFor="add-engine" className="field-label">
              Engine
            </label>
            {engineStatusLoading ? (
              <p className="field-hint">Loading engine status…</p>
            ) : (
              <select
                id="add-engine"
                className="field-select"
                value={isTorrent ? "native_torrent" : engineKind}
                onChange={(e) => {
                  setEngineKind(e.target.value as EngineKind);
                  setFormats(null);
                  setSelectedFormatId("");
                  setAudioFormat("");
                }}
                disabled={busy || formatsLoading || isTorrent}
                aria-describedby="add-engine-hint"
              >
                {ENGINE_OPTIONS.map(({ kind, label }) => {
                  const available = isEngineAvailable(kind, engineStatus);
                  const reason = engineUnavailableReason(kind, engineStatus);
                  return (
                    <option key={kind} value={kind} disabled={!available}>
                      {available ? label : `${label} — ${reason ?? "unavailable"}`}
                    </option>
                  );
                })}
              </select>
            )}
            <p id="add-engine-hint" className="field-hint">
              {isTorrent
                ? "Magnet links and local torrent files use the native FetchX BitTorrent engine."
                : "Only available engines are selectable. Configure engines in Settings."}
            </p>
          </div>

          {engineKind === "yt_dlp" && (
            <div className="field">
              <label htmlFor="add-audio-format" className="field-label">
                Media output
              </label>
              <select
                id="add-audio-format"
                className="field-select"
                value={audioFormat || "video"}
                onChange={(event) => {
                  const value = event.target.value;
                  setAudioFormat(value === "mp3" || value === "m4a" ? value : "");
                }}
                disabled={busy}
              >
                <option value="video">Video (select quality)</option>
                <option value="mp3">Extract audio as MP3</option>
                <option value="m4a">Extract audio as M4A</option>
              </select>
              {audioFormat ? (
                <p className="field-hint">
                  FetchX selects the best available audio stream and converts it to {audioFormat.toUpperCase()}.
                  FFmpeg must be available to yt-dlp.
                </p>
              ) : formats === null ? (
                <button
                  type="button"
                  className="button button-secondary"
                  onClick={() => void loadFormats()}
                  disabled={!url || busy || formatsLoading}
                >
                  {formatsLoading ? "Loading formats…" : "Load available formats"}
                </button>
              ) : (
                <>
                  <label htmlFor="add-format" className="field-label">
                    Video format
                  </label>
                <select
                  id="add-format"
                  className="field-select"
                  value={selectedFormatId}
                  onChange={(e) => setSelectedFormatId(e.target.value)}
                  disabled={busy}
                >
                  <option value="">Automatic best quality</option>
                  {formats.map((format) => (
                    <option key={format.format_id} value={format.format_id}>
                      {format.label}
                    </option>
                  ))}
                </select>
                </>
              )}
              {!audioFormat && (
                <p className="field-hint">
                  {formats === null
                    ? "Discover formats before selecting a specific quality."
                    : formats.length === 0
                      ? "No selectable video formats were found; automatic selection remains available."
                      : "Choose a discovered format, or keep automatic best quality."}
                </p>
              )}
            </div>
          )}

          <div className="field">
            <label htmlFor="add-priority" className="field-label">
              Priority
            </label>
            <select
              id="add-priority"
              className="field-select"
              value={priority}
              onChange={(e) => setPriority(e.target.value as DownloadPriority)}
              disabled={busy}
            >
              <option value="low">Low</option>
              <option value="normal">Normal</option>
              <option value="high">High</option>
            </select>
          </div>

          <div className="field field-checkbox">
            <input
              id="add-schedule-toggle"
              type="checkbox"
              checked={scheduleEnabled}
              onChange={(e) => setScheduleEnabled(e.target.checked)}
              disabled={busy}
            />
            <label htmlFor="add-schedule-toggle">Schedule start time</label>
          </div>

          {scheduleEnabled && (
            <div className="field">
              <label htmlFor="add-scheduled-at" className="field-label">
                Start at
              </label>
              <input
                id="add-scheduled-at"
                type="datetime-local"
                className="field-input"
                value={scheduledAt}
                onChange={(e) => setScheduledAt(e.target.value)}
                disabled={busy}
              />
            </div>
          )}

          <div className="dialog-footer">
            <p className="field-hint dialog-notice">
              Filename and destination are determined by the backend.
            </p>
            <div className="dialog-actions">
              <button
                type="button"
                className="button button-secondary"
                onClick={onClose}
                disabled={busy}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="button button-primary"
                disabled={busy || formatsLoading || !url || engineStatusLoading}
              >
                {busy ? "Adding…" : "Add download"}
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}
