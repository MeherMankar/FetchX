import { memo, useEffect, useState } from "react";

import type { Download, DownloadPriority, DownloadSchedule } from "../api/types";
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
  onQueueMove: (id: string, direction: "up" | "down") => void;
  onSetSchedule: (id: string, ms: number) => Promise<void>;
  onClearSchedule: (id: string) => Promise<void>;
  onError: (message: string) => void;
  onEnsureSchedule: (download: Download) => void;
  onUpdateUrl: (id: string, newUrl: string) => Promise<void>;
  onSetBandwidthLimit: (id: string) => void;
  onContextMenu?: (e: React.MouseEvent, download: Download) => void;
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
  onDelete,
  onRetry,
  onPriorityChange,
  onQueueMove,
  onSetSchedule,
  onClearSchedule,
  onError,
  onEnsureSchedule,
  onUpdateUrl,
  onSetBandwidthLimit,
  onContextMenu,
  speedHistory,
}: DownloadRowProps) {
  const { progress } = download;
  const pct = progress.percentage_basis_points;
  const isLegacyAria2 = download.engine === "aria2";

  const [urlEditing, setUrlEditing] = useState(false);
  const [urlDraft, setUrlDraft] = useState('');
  const [urlSaving, setUrlSaving] = useState(false);

  useEffect(() => {
    setUrlEditing(false);
  }, [download.id]);

  const canUpdateUrl = !isLegacyAria2 && (
    download.status === "failed" ||
    download.status === "cancelled" ||
    download.status === "paused" ||
    download.status === "queued"
  );
  const urlRetryable = download.status === "failed" || download.status === "cancelled";
  const submitUrl = async () => {
    const newUrl = urlDraft.trim();
    if (!newUrl) {
      onError("Paste a fresh download URL.");
      return;
    }

    setUrlSaving(true);
    try {
      await onUpdateUrl(download.id, newUrl);
      setUrlEditing(false);
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setUrlSaving(false);
    }
  };

  // Trigger lazy schedule fetch for schedulable downloads.
  onEnsureSchedule(download);

  return (
    <article
      className={`download-card${selected ? " download-card--selected" : ""}${checked ? " download-card--checked" : ""}`}
      data-status={download.status}
      aria-selected={selected}
      onContextMenu={(e) => onContextMenu?.(e, download)}
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

        {canUpdateUrl && urlEditing && (
          // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
          <div
            className="url-refresh-row"
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
          >
            <input
              className="url-refresh-input"
              type="text"
              value={urlDraft}
              placeholder="Paste refreshed download URL"
              onChange={(e) => setUrlDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  void submitUrl();
                } else if (e.key === "Escape") {
                  setUrlEditing(false);
                }
              }}
              disabled={urlSaving}
              // eslint-disable-next-line jsx-a11y/no-autofocus
              autoFocus
            />
            <p className="url-refresh-help">
              FetchX resumes saved bytes only when the server confirms the same file; otherwise it safely restarts.
            </p>
            <button
              className="button button-secondary"
              disabled={urlSaving}
              onClick={() => void submitUrl()}
            >
              {urlSaving ? "Updating..." : urlRetryable ? "Replace & retry" : "Update URL"}
            </button>
            <button
              className="button button-secondary"
              disabled={urlSaving}
              onClick={() => setUrlEditing(false)}
            >
              Cancel
            </button>
          </div>
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
        {!isLegacyAria2 && download.status === "queued" && (
          <>
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onQueueMove(download.id, "up")}
              aria-label={`Move ${download.filename} earlier in queue`}
              title="Move earlier in queue"
            >
              Earlier
            </button>
            <button
              className="button button-secondary"
              disabled={busy}
              onClick={() => onQueueMove(download.id, "down")}
              aria-label={`Move ${download.filename} later in queue`}
              title="Move later in queue"
            >
              Later
            </button>
          </>
        )}
        {!isLegacyAria2 && canChangePriority(download.status) && (
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
        {canUpdateUrl && !urlEditing && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => { setUrlDraft(download.url); setUrlEditing(true); }}
            aria-label={`Update URL for ${download.filename}`}
          >
            {urlRetryable ? "Refresh link" : "Update URL"}
          </button>
        )}
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
        {!isLegacyAria2 && canCancel(download.status) && (
          <button
            className="button button-secondary"
            disabled={busy}
            onClick={() => onCancel(download.id)}
            aria-label={`Cancel ${download.filename}`}
          >
            Cancel
          </button>
        )}
        {(download.status === 'downloading' || download.status === 'queued' || download.status === 'paused') && (
          <button
            className="button button-icon"
            title="Set speed limit"
            onClick={(e) => { e.stopPropagation(); onSetBandwidthLimit(download.id); }}
            aria-label="Set speed limit"
          >
            {download.bandwidth_limit ? formatSpeed(download.bandwidth_limit) : '∞'}
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
