import { memo } from "react";

import type { Download, DownloadSchedule } from "../api/types";
import { Sparkline } from "./Sparkline";

// Map raw status strings to readable labels for display only
const STATUS_LABELS: Record<string, string> = {
  metadata_acquiring: "acquiring",
  recovering: "recovering",
  downloading: "downloading",
  preparing: "preparing",
  queued: "queued",
  paused: "paused",
  completed: "completed",
  failed: "failed",
  cancelled: "cancelled",
  seeding: "seeding",
};
import {
  formatBytes,
  formatDatetime,
  formatEta,
  formatPercent,
  formatRelativeTime,
  formatSpeed,
  friendlyErrorMessage,
} from "../lib/format";
import {
  canCancel,
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
  onRetry: (id: string) => void;
  onSetSchedule: (id: string, ms: number) => Promise<void>;
  onClearSchedule: (id: string) => Promise<void>;
  onError: (message: string) => void;
  onEnsureSchedule: (download: Download) => void;
  onContextMenu?: (x: number, y: number, download: Download) => void;
  speedHistory?: number[];
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
  onRetry,
  onSetSchedule,
  onClearSchedule,
  onError,
  onEnsureSchedule,
  onContextMenu,
  speedHistory,
}: DownloadRowProps) {
  const { progress } = download;
  const pct = progress.percentage_basis_points;
  const isLegacyAria2 = download.engine === "aria2";

  // Trigger lazy schedule fetch for schedulable downloads.
  onEnsureSchedule(download);

  return (
    <article
      className={`download-card${selected ? " download-card--selected" : ""}${checked ? " download-card--checked" : ""}`}
      data-status={download.status}
      aria-selected={selected}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu?.(e.clientX, e.clientY, download);
      }}
      onClick={(event) => {
        if (event.button === 0) onSelect();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onSelect();
        } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
          e.preventDefault();
          const bounds = e.currentTarget.getBoundingClientRect();
          onContextMenu?.(bounds.left + 24, bounds.top + 24, download);
        }
      }}
      tabIndex={0}
      role="button"
      aria-keyshortcuts="Shift+F10"
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
          <span className={`status status-${download.status}`}>
            {STATUS_LABELS[download.status] ?? download.status}
          </span>
          {schedule?.enabled && (
            <span
              className="badge-scheduled"
              title={`Scheduled: ${formatDatetime(schedule.scheduled_at_ms)}`}
            >
              Scheduled
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
              {speedHistory && speedHistory.length >= 3 && (
                <Sparkline data={speedHistory} width={80} height={20} />
              )}
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
          <p className="download-error">{friendlyErrorMessage(download.error.message)}</p>
        )}
        {isLegacyAria2 && (
          <p className="download-error">
            Legacy aria2 job cannot resume because the external engine was removed.
          </p>
        )}

        {!isLegacyAria2 && canSchedule(download.status) && (
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
        {!isLegacyAria2 && canPause(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onPause(download.id)}
            aria-label={`Pause ${download.filename}`}
          >
            Pause
          </button>
        )}
        {!isLegacyAria2 && canResume(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onResume(download.id)}
            aria-label={`Resume ${download.filename}`}
          >
            Resume
          </button>
        )}
        {!isLegacyAria2 && canRetry(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onRetry(download.id)}
            aria-label={`Retry ${download.filename}`}
          >
            Retry
          </button>
        )}
        {isLegacyAria2 && canCancel(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onCancel(download.id)}
            aria-label={`Cancel ${download.filename}`}
          >
            Cancel
          </button>
        )}
      </div>
    </article>
  );
});
