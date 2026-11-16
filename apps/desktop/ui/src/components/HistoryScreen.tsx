import { useCallback, useEffect, useState } from "react";
import { clearDownloadHistory, listCompletedDownloads } from "../api/history";
import type { Download } from "../api/types";
import { formatBytes, formatRelativeTime, formatSpeed } from "../lib/format";
import { errorMessage } from "../lib/format";

export function HistoryScreen() {
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);
  const [filter, setFilter] = useState("");
  const [confirmClear, setConfirmClear] = useState(false);

  const totalBytes = downloads.reduce((sum, d) => sum + d.progress.downloaded_bytes, 0);
  const filtered = filter
    ? downloads.filter(
        (d) =>
          d.filename.toLowerCase().includes(filter.toLowerCase()) ||
          d.url.toLowerCase().includes(filter.toLowerCase()),
      )
    : downloads;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const items = await listCompletedDownloads(500);
      setDownloads(items);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const handleClear = async () => {
    if (!confirmClear) { setConfirmClear(true); return; }
    setClearing(true);
    try {
      await clearDownloadHistory();
      setDownloads([]);
      setConfirmClear(false);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setClearing(false);
    }
  };

  if (loading) return <div className="settings-loading" role="status">Loading history...</div>;

  return (
    <div className="settings-page">
      <div className="settings-header">
        <h2 className="settings-title">History</h2>
        <div style={{ display: "flex", gap: "0.5rem" }}>
          <button className="button button-secondary" onClick={() => void load()}>Refresh</button>
          {downloads.length > 0 && (
            <button
              className={confirmClear ? "button button-danger" : "button button-secondary"}
              onClick={() => void handleClear()}
              disabled={clearing}
            >
              {clearing ? "Clearing..." : confirmClear ? "Confirm clear" : "Clear history"}
            </button>
          )}
          {confirmClear && (
            <button className="button button-secondary" onClick={() => setConfirmClear(false)}>Cancel</button>
          )}
        </div>
      </div>

      {error && (
        <div className="error-banner" role="alert">
          <span>{error}</span>
          <button className="icon-button" aria-label="Dismiss" onClick={() => setError(null)}>x</button>
        </div>
      )}

      <div className="snapshot-bar" style={{ marginBottom: "1rem" }}>
        <span className="snapshot-stat">
          <span className="snapshot-label">Completed</span>
          <span className="snapshot-value">{downloads.length}</span>
        </span>
        <span className="snapshot-stat">
          <span className="snapshot-label">Total downloaded</span>
          <span className="snapshot-value">{formatBytes(totalBytes)}</span>
        </span>
      </div>

      {downloads.length > 0 && (
        <div className="settings-row" style={{ marginBottom: "0.75rem" }}>
          <input
            type="search"
            className="settings-input"
            placeholder="Filter by filename or URL..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            aria-label="Filter history"
          />
        </div>
      )}

      {filtered.length === 0 && (
        <div className="empty-state">
          <p>{downloads.length === 0 ? "No completed downloads yet." : "No results match your filter."}</p>
        </div>
      )}

      <div className="download-list" role="list">
        {filtered.map((download) => (
          <article key={download.id} className="download-card" role="listitem">
            <div className="download-main">
              <div className="download-title">
                <strong title={download.filename}>{download.filename}</strong>
                <span className="status status-completed">completed</span>
              </div>
              <p className="download-url" title={download.url}>{download.url}</p>
              <div className="download-meta">
                <span>{formatBytes(download.progress.downloaded_bytes)}</span>
                {download.progress.speed_bytes_per_second !== null && (
                  <span> avg {formatSpeed(download.progress.speed_bytes_per_second)}</span>
                )}
                <span className="download-time">{formatRelativeTime(download.updated_at)}</span>
              </div>
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}
