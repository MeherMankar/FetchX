import { memo } from "react";

import type { Download, DownloadPriority, DownloadSchedule } from "../api/types";
import {
  formatBytes,
  formatDatetime,
  formatEta,
  formatPercent,
  formatRelativeTime,
  formatSpeed,
} from "../lib/format";
import {
  canCancel,
  canChangePriority,
  canPause,
  canResume,
  canRetry,
  canSchedule,
  isActive,
} from "../lib/status";
import { ScheduleEditor } from "./ScheduleEditor";

interface DownloadRowProps {
  download: Download;
  schedule: DownloadSchedule | null | undefined;
  busy: boolean;
  selected: boolean;
  checked: boolean;
  onSelect: () => void;
  onCheckToggle: (e: React.ChangeEvent<HTMLInputElement>) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  onRetry: (id: string) => void;
  onPriorityChange: (id: string, priority: DownloadPriority) => void;
  onSetSchedule: (id: string, ms: number) => Promise<void>;
  onClearSchedule: (id: string) => Promise<void>;
  onError: (message: string) => void;
  onEnsureSchedule: (download: Download) => void;
}

export const DownloadRow = memo(function DownloadRow({
  download,
  schedule,
  busy,
  selected,
  checked,
  onSelect,
  onCheckToggle,
  onPause,
  onResume,
  onCancel,
  onDelete,
  onRetry,
  onPriorityChange,
  onSetSchedule,
  onClearSchedule,
  onError,
  onEnsureSchedule,
}: DownloadRowProps) {
  const { progress } = download;
  const pct = progress.percentage_basis_points;

  // Trigger lazy schedule fetch for schedulable downloads.
  onEnsureSchedule(download);

  return (
    <article
      className={`download-card${selected ? " download-card--selected" : ""}${checked ? " download-card--checked" : ""}`}
      data-status={download.status}
      aria-selected={selected}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onSelect();
        }
      }}
      tabIndex={0}
      role="button"
      aria-label={`${download.filename}, ${download.status}`}
    >
      {/* Checkbox — stops row selection propagation */}
      {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
      <div
        className="row-checkbox-wrap"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.stopPropagation()}
      >
        <input
          type="checkbox"
          className="row-checkbox"
          checked={checked}
          onChange={onCheckToggle}
          aria-label={`Select ${download.filename}`}
          tabIndex={-1}
        />
      </div>

      <div className="download-main">
        <div className="download-title">
          <strong title={download.filename}>{download.filename}</strong>
          <span className={`status status-${download.status}`}>{download.status}</span>
          {schedule?.enabled && (
            <span
              className="badge-scheduled"
              title={`Scheduled: ${formatDatetime(schedule.scheduled_at_ms)}`}
            >
              ⏱
            </span>
          )}
        </div>

        <p className="download-url" title={download.url}>{download.url}</p>

        <progress
          className={`progress-track${isActive(download.status) ? " progress-track--active" : ""}`}
          aria-label={`Progress: ${formatPercent(pct)}`}
          value={pct === null ? 0 : pct / 100}
          max={100}
        />

        <div className="download-meta">
          <span>
            {formatBytes(progress.downloaded_bytes)}
            {progress.total_bytes !== null ? ` / ${formatBytes(progress.total_bytes)}` : ""}
            {pct !== null ? ` · ${formatPercent(pct)}` : ""}
          </span>
          {isActive(download.status) ? (
            <span className="download-speed">
              <span>{formatSpeed(progress.speed_bytes_per_second)}</span>
              <span className="download-eta">
                ETA {formatEta(progress.estimated_remaining_seconds)}
              </span>
            </span>
          ) : (
            <span
              className="download-time"
              title={formatDatetime(download.updated_at)}
            >
              {formatRelativeTime(download.updated_at)}
            </span>
          )}
        </div>

        {download.error && (
          <p className="download-error">{download.error.message}</p>
        )}

        {canSchedule(download.status) && (
          // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
          <div onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
            <ScheduleEditor
              downloadId={download.id}
              schedule={schedule ?? null}
              busy={busy}
              onSet={(ms) => onSetSchedule(download.id, ms)}
              onClear={() => onClearSchedule(download.id)}
              onError={onError}
            />
          </div>
        )}
      </div>

      {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
      <div
        className="download-actions"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.stopPropagation()}
      >
        {canChangePriority(download.status) && (
          <>
            <label className="visually-hidden" htmlFor={`priority-${download.id}`}>
              Priority
            </label>
            <select
              id={`priority-${download.id}`}
              className="priority-select"
              value={download.priority}
              disabled={busy}
              onChange={(e) =>
                onPriorityChange(download.id, e.target.value as DownloadPriority)
              }
              aria-label={`Priority for ${download.filename}`}
            >
              <option value="low">Low</option>
              <option value="normal">Normal</option>
              <option value="high">High</option>
            </select>
          </>
        )}
        {canPause(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onPause(download.id)}
            aria-label={`Pause ${download.filename}`}
          >
            Pause
          </button>
        )}
        {canResume(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onResume(download.id)}
            aria-label={`Resume ${download.filename}`}
          >
            Resume
          </button>
        )}
        {canRetry(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onRetry(download.id)}
            aria-label={`Retry ${download.filename}`}
          >
            Retry
          </button>
        )}
        {canCancel(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onCancel(download.id)}
            aria-label={`Cancel ${download.filename}`}
          >
            Cancel
          </button>
        )}
        <button
          className="button button-danger"
          disabled={busy}
          onClick={() => onDelete(download.id)}
          aria-label={`Delete ${download.filename}`}
        >
          Delete
        </button>
      </div>
    </article>
  );
});
