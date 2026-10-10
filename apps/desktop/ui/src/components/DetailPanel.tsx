import type { Download, DownloadSchedule } from "../api/types";
import { formatBytes, formatDatetime, formatEta, formatPercent, formatSpeed } from "../lib/format";
import { canSchedule, canChangePriority, canPause, canResume, canCancel } from "../lib/status";
import { ScheduleEditor } from "./ScheduleEditor";
import type { DownloadPriority } from "../api/types";

const ENGINE_LABELS: Record<string, string> = {
  direct_http: "Direct HTTP",
  aria2: "aria2",
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
  onPriorityChange: (id: string, priority: DownloadPriority) => void;
  onSetSchedule: (id: string, ms: number) => Promise<void>;
  onClearSchedule: (id: string) => Promise<void>;
  onError: (message: string) => void;
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
  onPriorityChange,
  onSetSchedule,
  onClearSchedule,
  onError,
}: DetailPanelProps) {
  const { progress } = download;
  const pct = progress.percentage_basis_points;

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
            <dd>{formatDatetime(download.created_at)}</dd>
            <dt>Updated</dt>
            <dd>{formatDatetime(download.updated_at)}</dd>
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
