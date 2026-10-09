import { useCallback, useEffect, useState, type FormEvent } from "react";

import { getAppInfo } from "./api/app";
import {
  addDownload,
  cancelDownload,
  deleteDownload,
  listenForDownloadChanges,
  listDownloads,
  pauseDownload,
  resumeDownload,
} from "./api/downloads";
import type { AppInfo, Download, DownloadStatus } from "./api/types";

function errorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "The request could not be completed.";
}

function formatBytes(value: number | null): string {
  if (value === null) {
    return "Unknown";
  }
  if (value < 1024) {
    return `${value} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let size = value;
  let unit = -1;
  do {
    size /= 1024;
    unit += 1;
  } while (size >= 1024 && unit < units.length - 1);
  return `${size.toFixed(1)} ${units[unit]}`;
}

function canPause(status: DownloadStatus): boolean {
  return status === "queued" || status === "preparing" || status === "downloading";
}

function canCancel(status: DownloadStatus): boolean {
  return canPause(status) || status === "paused";
}

export function App() {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [url, setUrl] = useState("");
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const items = await listDownloads();
      setDownloads(items);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const initialize = async () => {
      try {
        const info = await getAppInfo();
        if (!disposed) {
          setAppInfo(info);
        }
        const stopListening = await listenForDownloadChanges(
          (download) => {
            setDownloads((current) => {
              const withoutItem = current.filter((item) => item.id !== download.id);
              return [download, ...withoutItem];
            });
          },
          (downloadId) => {
            setDownloads((current) => current.filter((item) => item.id !== downloadId));
          },
        );
        if (disposed) {
          stopListening();
        } else {
          unlisten = stopListening;
        }
      } catch (cause) {
        if (!disposed) {
          setError(errorMessage(cause));
        }
      }
      if (!disposed) {
        await refresh();
      }
    };

    void initialize();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refresh]);

  const perform = async (id: string, action: () => Promise<unknown>) => {
    setBusyId(id);
    setError(null);
    try {
      await action();
      await refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    try {
      const download = await addDownload({ url });
      setDownloads((current) => [download, ...current.filter((item) => item.id !== download.id)]);
      setUrl("");
      await refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand-mark" aria-hidden="true">↓</div>
        <div>
          <h1>{appInfo?.name ?? "MyDownloader"}</h1>
          <p>Desktop foundation{appInfo ? ` · v${appInfo.version}` : ""}</p>
        </div>
        <button className="button button-secondary refresh-button" onClick={() => void refresh()}>
          Refresh
        </button>
      </header>

      <section className="add-panel" aria-labelledby="add-heading">
        <h2 id="add-heading">Add a download</h2>
        <form className="add-form" onSubmit={(event) => void submit(event)}>
          <label className="visually-hidden" htmlFor="download-url">Download URL</label>
          <input
            id="download-url"
            type="url"
            inputMode="url"
            placeholder="https://example.com/file.zip"
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            required
          />
          <button className="button button-primary" type="submit">Add Download</button>
        </form>
      </section>

      {error && (
        <div className="error-banner" role="alert">
          <span>{error}</span>
          <button className="icon-button" aria-label="Dismiss error" onClick={() => setError(null)}>
            ×
          </button>
        </div>
      )}

      <section className="downloads-section" aria-labelledby="downloads-heading">
        <div className="section-heading">
          <div>
            <h2 id="downloads-heading">Downloads</h2>
            <p>{downloads.length} {downloads.length === 1 ? "item" : "items"}</p>
          </div>
        </div>

        {loading ? (
          <div className="empty-state">Loading downloads…</div>
        ) : downloads.length === 0 ? (
          <div className="empty-state">
            <span className="empty-icon" aria-hidden="true">↓</span>
            <strong>No downloads yet</strong>
            <span>Paste a link above to get started.</span>
          </div>
        ) : (
          <div className="download-list">
            {downloads.map((download) => {
              const percentage = download.progress.percentage_basis_points;
              return (
                <article className="download-card" key={download.id}>
                  <div className="download-main">
                    <div className="download-title">
                      <strong title={download.filename}>{download.filename}</strong>
                      <span className={`status status-${download.status}`}>
                        {download.status}
                      </span>
                    </div>
                    <p className="download-url" title={download.url}>{download.url}</p>
                    <progress
                      className="progress-track"
                      aria-label={`Progress for ${download.filename}`}
                      value={percentage === null ? 0 : percentage / 100}
                      max={100}
                    />
                    <div className="download-meta">
                      <span>{formatBytes(download.progress.downloaded_bytes)} of {formatBytes(download.progress.total_bytes)}</span>
                      {percentage !== null && <span>{(percentage / 100).toFixed(1)}%</span>}
                    </div>
                    {download.error && (
                      <p className="download-error">{download.error.message}</p>
                    )}
                  </div>
                  <div className="download-actions">
                    {canPause(download.status) && (
                      <button
                        className="button button-secondary"
                        disabled={busyId === download.id}
                        onClick={() => void perform(download.id, () => pauseDownload(download.id))}
                      >
                        Pause
                      </button>
                    )}
                    {download.status === "paused" && (
                      <button
                        className="button button-secondary"
                        disabled={busyId === download.id}
                        onClick={() => void perform(download.id, () => resumeDownload(download.id))}
                      >
                        Resume
                      </button>
                    )}
                    {canCancel(download.status) && (
                      <button
                        className="button button-secondary"
                        disabled={busyId === download.id}
                        onClick={() => void perform(download.id, () => cancelDownload(download.id))}
                      >
                        Cancel
                      </button>
                    )}
                    <button
                      className="button button-danger"
                      disabled={busyId === download.id}
                      onClick={() => void perform(download.id, () => deleteDownload(download.id))}
                    >
                      Delete
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>

      <footer className="app-footer">
        Downloads are stored in your system Downloads folder. Removing an item preserves its files.
      </footer>
    </main>
  );
}
