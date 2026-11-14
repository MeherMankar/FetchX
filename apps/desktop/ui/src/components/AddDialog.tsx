import { useEffect, useRef, useState, type FormEvent } from "react";

import {
  addDownload,
  addDownloadBatch,
  addTorrentDownload,
  addMediaDownload,
  discoverYtDlpFormats,
  extractDownloadLinks,
  suggestDownloadFilename,
  suggestDownloadPackageName,
  inspectTorrentFile,
  scheduleDownload,
  setDownloadPriority,
} from "../api/downloads";
import type { DownloadPriority, YtDlpFormat } from "../api/types";
import { errorMessage, formatBytes, msToDatetimeLocal } from "../lib/format";

interface AddDialogProps {
  initialUrl?: string;
  onAdded: () => void;
  onClose: () => void;
}

function isLikelyYtDlpUrl(value: string): boolean {
  try {
    const parsed = new URL(value.trim());
    if (!["http:", "https:"].includes(parsed.protocol)) return false;
    const filename = parsed.pathname.split("/").pop() ?? "";
    const extension = filename.includes(".")
      ? filename.split(".").pop()?.toLowerCase()
      : undefined;
    if (extension) return ["mpd", "html", "htm", "php", "asp", "aspx"].includes(extension);
    const host = parsed.hostname.toLowerCase();
    const knownMediaHost = [
      "youtube.com",
      "youtu.be",
      "vimeo.com",
      "twitch.tv",
      "dailymotion.com",
      "tiktok.com",
      "facebook.com",
      "instagram.com",
      "soundcloud.com",
    ].some((domain) => host === domain || host.endsWith(`.${domain}`));
    const mediaPagePath = /\/(watch|video|videos|stream|player|embed)(\/|$)/i.test(parsed.pathname);
    return knownMediaHost || mediaPagePath;
  } catch {
    return false;
  }
}

function isHttpUrl(value: string): boolean {
  try {
    return ["http:", "https:"].includes(new URL(value.trim()).protocol);
  } catch {
    return false;
  }
}

export function AddDialog({ initialUrl = "", onAdded, onClose }: AddDialogProps) {
  const [url, setUrl] = useState(initialUrl);
  const [packageName, setPackageName] = useState("");
  const [packageNameEdited, setPackageNameEdited] = useState(false);
  const [priority, setPriority] = useState<DownloadPriority>("normal");
  const [scheduleEnabled, setScheduleEnabled] = useState(false);
  const [scheduledAt, setScheduledAt] = useState(
    msToDatetimeLocal(Math.ceil(Date.now() / 60000) * 60000 + 3_600_000),
  );
  const [busy, setBusy] = useState(false);
  const [formats, setFormats] = useState<YtDlpFormat[] | null>(null);
  const [selectedFormatId, setSelectedFormatId] = useState("");
  const [torrentInspection, setTorrentInspection] = useState<Awaited<ReturnType<typeof inspectTorrentFile>> | null>(null);
  const [selectedTorrentFiles, setSelectedTorrentFiles] = useState<number[]>([]);
  const [formatsLoading, setFormatsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const urlInputRef = useRef<HTMLInputElement>(null);
  const isMagnet = url.trimStart().toLowerCase().startsWith("magnet:");
  const isTorrentFile = !url.includes("://") && /\.torrent$/i.test(url.trim());
  const isTorrent = isMagnet || isTorrentFile;
  const isYtDlpUrl = !isTorrent && isLikelyYtDlpUrl(url);

  useEffect(() => {
    if (packageNameEdited || packageName.trim() || !url.trim()) return;
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
  }, [url, packageName, packageNameEdited]);

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
      const bulkLinks = await extractDownloadLinks(url);
      const urlsToAdd = bulkLinks.length > 1 ? bulkLinks : [url];
      if (
        urlsToAdd.length > 1
        && !isTorrent
        && urlsToAdd.every(isHttpUrl)
      ) {
        const results = await addDownloadBatch({
          urls: urlsToAdd,
          packageName: packageNameEdited ? packageName.trim() || undefined : undefined,
          engine: selectedFormatId ? "yt_dlp" : "direct_http",
          priority,
          formatId: selectedFormatId || undefined,
          extension: formats?.find((format) => format.format_id === selectedFormatId)?.extension,
        });
        const failures = results
          .filter((result) => result.error)
          .map((result) => `${result.input_url}: ${result.error}`);
        const failedUrls = results
          .filter((result) => !result.download_id)
          .map((result) => result.input_url);

        if (scheduleEnabled) {
          const ms = new Date(scheduledAt).getTime();
          for (const result of results) {
            if (!result.download_id) continue;
            try {
              await scheduleDownload(result.download_id, ms);
            } catch (cause) {
              failures.push(`${result.input_url}: scheduling failed (${errorMessage(cause)})`);
            }
          }
        }

        if (failures.length > 0) {
          setUrl(failedUrls.join("\n"));
          setSelectedFormatId("");
          const queuedCount = results.filter((result) => result.download_id).length;
          setError(
            `${queuedCount} of ${results.length} downloads were queued. ` +
              failures.slice(0, 5).join(" "),
          );
          setBusy(false);
          return;
        }

        onAdded();
        return;
      }

      let queuedCount = 0;
      const failures: string[] = [];
      const failedInputs: string[] = [];
      const packageNameForBatch = urlsToAdd.length > 1 && !packageNameEdited
        ? undefined
        : packageName.trim() || undefined;

      for (const targetUrl of urlsToAdd) {
        let admitted = false;
        try {
          const download = isTorrentFile && torrentInspection
            ? await addTorrentDownload(url.trim(), selectedTorrentFiles)
            : selectedFormatId && isYtDlpUrl
              ? await addMediaDownload(
                  targetUrl,
                  selectedFormatId,
                  formats?.find((format) => format.format_id === selectedFormatId)?.extension,
                  undefined,
                )
              : await addDownload({
                  url: targetUrl.trim(),
                  package_name: urlsToAdd.length > 1
                    ? packageNameForBatch
                    : packageName.trim() || undefined,
                });
          admitted = true;
          queuedCount += 1;

          if (priority !== "normal") {
            await setDownloadPriority(download.id, priority);
          }

          if (scheduleEnabled) {
            const ms = new Date(scheduledAt).getTime();
            await scheduleDownload(download.id, ms);
          }
        } catch (cause) {
          failures.push(`${targetUrl}: ${errorMessage(cause)}`);
          if (!admitted) failedInputs.push(targetUrl);
        }
      }

      if (failures.length > 0) {
        setUrl(failedInputs.join("\n"));
        setSelectedFormatId("");
        setError(
          `${queuedCount} of ${urlsToAdd.length} downloads were queued. ` +
            failures.slice(0, 5).join(" "),
        );
        setBusy(false);
        return;
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
              onChange={(event) => {
                setPackageName(event.target.value);
                setPackageNameEdited(true);
              }}
              placeholder="FetchX Downloads"
              disabled={busy}
            />
            <p className="field-hint">
              Related downloads are stored together in this package folder. For batches, leave the
              suggested name unchanged to group links by source.
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

          {isYtDlpUrl && (
            <div className="field">
              {formats === null ? (
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
              <p className="field-hint">
                {formats === null
                  ? "Discover formats before selecting a specific quality."
                  : formats.length === 0
                    ? "No selectable video formats were found; automatic selection remains available."
                    : "Choose a discovered format, or keep automatic best quality."}
              </p>
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
                disabled={busy || formatsLoading || !url}
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
