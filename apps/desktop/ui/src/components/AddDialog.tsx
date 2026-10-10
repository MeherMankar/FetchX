import { useRef, useState, type FormEvent } from "react";

import { addDownload, scheduleDownload, setDownloadPriority } from "../api/downloads";
import type { DownloadPriority } from "../api/types";
import { errorMessage, msToDatetimeLocal } from "../lib/format";

interface AddDialogProps {
  initialUrl?: string;
  onAdded: () => void;
  onClose: () => void;
}

export function AddDialog({ initialUrl = "", onAdded, onClose }: AddDialogProps) {
  const [url, setUrl] = useState(initialUrl);
  const [priority, setPriority] = useState<DownloadPriority>("normal");
  const [scheduleEnabled, setScheduleEnabled] = useState(false);
  const [scheduledAt, setScheduledAt] = useState(
    msToDatetimeLocal(Math.ceil(Date.now() / 60000) * 60000 + 3_600_000),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const urlInputRef = useRef<HTMLInputElement>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setError(null);

    // Validate schedule time before sending to backend.
    if (scheduleEnabled) {
      const ms = new Date(scheduledAt).getTime();
      if (!Number.isFinite(ms) || ms <= Date.now()) {
        setError("Please choose a schedule time in the future.");
        return;
      }
    }

    setBusy(true);
    try {
      // addDownload creates a queued item at normal priority.
      // Priority and scheduling are separate commands once the download exists,
      // because the backend models them independently.
      const download = await addDownload({ url });

      // Apply priority if not normal (the backend defaults to normal).
      if (priority !== "normal") {
        await setDownloadPriority(download.id, priority);
      }

      // Apply schedule if requested.
      if (scheduleEnabled) {
        const ms = new Date(scheduledAt).getTime();
        await scheduleDownload(download.id, ms);
      }

      onAdded();
    } catch (cause) {
      setError(errorMessage(cause));
      setBusy(false);
    }
  };

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
    <div className="dialog-backdrop" onClick={handleBackdropClick} aria-modal="true" role="dialog" aria-labelledby="add-dialog-title">
      <div className="dialog">
        <header className="dialog-header">
          <h2 id="add-dialog-title">Add download</h2>
          <button className="icon-button" aria-label="Close dialog" onClick={onClose}>×</button>
        </header>

        <form className="dialog-body" onSubmit={(e) => void submit(e)}>
          {error && (
            <div className="dialog-error" role="alert">{error}</div>
          )}

          <div className="field">
            <label htmlFor="add-url" className="field-label">URL <span className="required" aria-hidden="true">*</span></label>
            <input
              id="add-url"
              ref={urlInputRef}
              type="url"
              inputMode="url"
              className="field-input"
              placeholder="https://example.com/file.zip"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              required
              disabled={busy}
              autoFocus
            />
            <p className="field-hint">HTTP and HTTPS URLs only. Credentials in URLs are not accepted.</p>
          </div>

          <div className="field">
            <label htmlFor="add-priority" className="field-label">Priority</label>
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
              <label htmlFor="add-scheduled-at" className="field-label">Start at</label>
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
              Engine, filename, and destination are determined by the backend.
            </p>
            <div className="dialog-actions">
              <button type="button" className="button button-secondary" onClick={onClose} disabled={busy}>
                Cancel
              </button>
              <button type="submit" className="button button-primary" disabled={busy || !url}>
                {busy ? "Adding…" : "Add download"}
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}
