import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";

import { getAppInfo } from "./api/app";
import {
  addDownload,
  cancelDownload,
  deleteDownload,
  getDownloadSchedule,
  listenForDownloadChanges,
  listDownloads,
  pauseDownload,
  removeDownloadSchedule,
  resumeDownload,
  scheduleDownload,
  setDownloadPriority,
} from "./api/downloads";
import type { AppInfo, Download, DownloadPriority, DownloadSchedule, DownloadStatus } from "./api/types";

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

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
    return "—";
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

function formatSpeed(bytesPerSecond: number | null): string {
  if (bytesPerSecond === null) {
    return "—";
  }
  return `${formatBytes(bytesPerSecond)}/s`;
}

function formatEta(seconds: number | null): string {
  if (seconds === null) {
    return "—";
  }
  if (seconds < 60) {
    return `${seconds}s`;
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  }
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

/** Format a Unix-ms timestamp as a local datetime string, e.g. "Oct 10, 2026, 14:30". */
function formatScheduledAt(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * Convert a Unix-ms timestamp to the value expected by <input type="datetime-local">,
 * which is "YYYY-MM-DDTHH:MM" in local time.
 */
function msToDatetimeLocal(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

// ---------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------

function isActive(status: DownloadStatus): boolean {
  return status === "downloading";
}

function isTerminal(status: DownloadStatus): boolean {
  return status === "completed" || status === "failed" || status === "cancelled";
}

function canPause(status: DownloadStatus): boolean {
  return status === "queued" || status === "preparing" || status === "downloading";
}

function canCancel(status: DownloadStatus): boolean {
  return canPause(status) || status === "paused";
}

function canChangePriority(status: DownloadStatus): boolean {
  return !isTerminal(status);
}

function canSchedule(status: DownloadStatus): boolean {
  return status === "queued" || status === "paused";
}

// ---------------------------------------------------------------------------
// ScheduleEditor sub-component
// ---------------------------------------------------------------------------

interface ScheduleEditorProps {
  downloadId: string;
  schedule: DownloadSchedule | null;
  onScheduleChanged: (schedule: DownloadSchedule | null) => void;
  onError: (message: string) => void;
}

function ScheduleEditor({ downloadId, schedule, onScheduleChanged, onError }: ScheduleEditorProps) {
  const [editing, setEditing] = useState(false);
  // Default to 1 hour from now, rounded to the minute.
  const defaultValue = msToDatetimeLocal(Math.ceil(Date.now() / 60000) * 60000 + 3_600_000);
  const [inputValue, setInputValue] = useState(
    schedule ? msToDatetimeLocal(schedule.scheduled_at_ms) : defaultValue,
  );
  const [busy, setBusy] = useState(false);

  const apply = async () => {
    const ms = new Date(inputValue).getTime();
    if (!Number.isFinite(ms) || ms <= Date.now()) {
      onError("Please choose a time in the future.");
      return;
    }
    setBusy(true);
    try {
      const updated = await scheduleDownload(downloadId, ms);
      onScheduleChanged(updated);
      setEditing(false);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      await removeDownloadSchedule(downloadId);
      onScheduleChanged(null);
      setEditing(false);
      setInputValue(defaultValue);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  if (!editing) {
    return (
      <div className="schedule-row">
        {schedule?.enabled ? (
          <>
            <span className="schedule-label">
              Starts {formatScheduledAt(schedule.scheduled_at_ms)}
            </span>
            <button
              className="button button-secondary button-xs"
              disabled={busy}
              onClick={() => {
                setInputValue(msToDatetimeLocal(schedule.scheduled_at_ms));
                setEditing(true);
              }}
            >
              Edit
            </button>
            <button
              className="button button-secondary button-xs"
              disabled={busy}
              onClick={() => void clear()}
            >
              Clear
            </button>
          </>
        ) : (
          <button
            className="button button-secondary button-xs"
            disabled={busy}
            onClick={() => setEditing(true)}
          >
            Schedule start
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="schedule-row">
      <label className="visually-hidden" htmlFor={`sched-${downloadId}`}>
        Schedule start time
      </label>
      <input
        id={`sched-${downloadId}`}
        className="schedule-input"
        type="datetime-local"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        disabled={busy}
      />
      <button
        className="button button-primary button-xs"
        disabled={busy || !inputValue}
        onClick={() => void apply()}
      >
        Set
      </button>
      <button
        className="button button-secondary button-xs"
        disabled={busy}
        onClick={() => setEditing(false)}
      >
        Cancel
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export function App() {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [downloads, setDownloads] = useState<Download[]>([]);
  // Map from download id → its persisted schedule (null = no schedule).
  // Populated lazily when a schedulable download is first rendered.
  const [schedules, setSchedules] = useState<Record<string, DownloadSchedule | null>>({});
  const [url, setUrl] = useState("");
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Track which download ids we have already fetched schedules for.
  const fetchedSchedules = useRef<Set<string>>(new Set());

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
            // Clean up schedule state for removed downloads.
            setSchedules((current) => {
              const next = { ...current };
              delete next[downloadId];
              return next;
            });
            fetchedSchedules.current.delete(downloadId);
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

  // Lazily fetch the schedule for a download the first time it becomes
  // schedulable (queued or paused).  Avoids a burst of IPC calls on startup.
  const ensureScheduleFetched = useCallback((download: Download) => {
    if (!canSchedule(download.status)) return;
    if (fetchedSchedules.current.has(download.id)) return;
    fetchedSchedules.current.add(download.id);
    void getDownloadSchedule(download.id).then((schedule) => {
      setSchedules((current) => ({ ...current, [download.id]: schedule }));
    });
  }, []);

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

  const handlePriorityChange = async (download: Download, priority: DownloadPriority) => {
    setBusyId(download.id);
    setError(null);
    try {
      const updated = await setDownloadPriority(download.id, priority);
      setDownloads((current) =>
        current.map((item) => (item.id === updated.id ? updated : item)),
      );
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
          <p>v{appInfo?.version ?? "…"}</p>
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
          <button className="button button-primary" type="submit">Add</button>
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
              // Trigger lazy schedule fetch whenever this download is rendered.
              ensureScheduleFetched(download);
              const schedule = schedules[download.id] ?? null;

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
                      <span>
                        {formatBytes(download.progress.downloaded_bytes)}
                        {download.progress.total_bytes !== null
                          ? ` / ${formatBytes(download.progress.total_bytes)}`
                          : ""}
                        {percentage !== null ? ` · ${(percentage / 100).toFixed(1)}%` : ""}
                      </span>
                      {isActive(download.status) && (
                        <span className="download-speed">
                          <span>{formatSpeed(download.progress.speed_bytes_per_second)}</span>
                          <span className="download-eta">
                            ETA {formatEta(download.progress.estimated_remaining_seconds)}
                          </span>
                        </span>
                      )}
                    </div>
                    {download.error && (
                      <p className="download-error">{download.error.message}</p>
                    )}
                    {canSchedule(download.status) && (
                      <ScheduleEditor
                        downloadId={download.id}
                        schedule={schedule}
                        onScheduleChanged={(updated) => {
                          setSchedules((current) => ({ ...current, [download.id]: updated }));
                        }}
                        onError={setError}
                      />
                    )}
                  </div>
                  <div className="download-actions">
                    {canChangePriority(download.status) && (
                      <label className="visually-hidden" htmlFor={`priority-${download.id}`}>
                        Priority
                      </label>
                    )}
                    {canChangePriority(download.status) && (
                      <select
                        id={`priority-${download.id}`}
                        className="priority-select"
                        value={download.priority}
                        disabled={busyId === download.id}
                        onChange={(e) =>
                          void handlePriorityChange(
                            download,
                            e.target.value as DownloadPriority,
                          )
                        }
                        aria-label={`Priority for ${download.filename}`}
                      >
                        <option value="low">Low</option>
                        <option value="normal">Normal</option>
                        <option value="high">High</option>
                      </select>
                    )}
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
