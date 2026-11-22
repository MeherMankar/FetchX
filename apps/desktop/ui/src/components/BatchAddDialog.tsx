import { useState } from "react";

import { addDownload, extractDownloadLinks } from "../api/downloads";
import { errorMessage } from "../lib/format";

interface BatchAddDialogProps {
  onComplete: () => void;
  onClose: () => void;
  onToast: (message: string, kind: "success" | "error") => void;
}

export function BatchAddDialog({ onComplete, onClose, onToast }: BatchAddDialogProps) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const handleAddAll = async () => {
    setError(null);
    let urls: string[];
    try {
      urls = await extractDownloadLinks(text);
    } catch (cause) {
      setError(errorMessage(cause));
      return;
    }
    if (urls.length === 0) {
      setError("No URLs found. Paste one or more HTTP/HTTPS URLs.");
      return;
    }

    setBusy(true);
    setProgress({ done: 0, total: urls.length });
    let failed = 0;
    for (let i = 0; i < urls.length; i++) {
      try {
        await addDownload({ url: urls[i] });
      } catch {
        failed++;
      }
      setProgress({ done: i + 1, total: urls.length });
    }
    setBusy(false);
    const queued = urls.length - failed;
    onToast(
      failed === 0
        ? `${queued} download${queued === 1 ? "" : "s"} queued.`
        : `${queued} of ${urls.length} downloads queued; ${failed} failed.`,
      failed === 0 ? "success" : "error",
    );
    onComplete();
  };

  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
    <div
      className="dialog-backdrop"
      onClick={handleBackdropClick}
      aria-modal="true"
      role="dialog"
      aria-labelledby="batch-dialog-title"
    >
      <div className="dialog">
        <header className="dialog-header">
          <h2 id="batch-dialog-title">Batch add</h2>
          <button className="icon-button" aria-label="Close dialog" onClick={onClose}>
            ×
          </button>
        </header>

        <div className="dialog-body">
          {error && (
            <div className="dialog-error" role="alert">
              {error}
            </div>
          )}

          <div className="field">
            <label htmlFor="batch-urls" className="field-label">URLs</label>
            <textarea
              id="batch-urls"
              className="field-input"
              rows={10}
              placeholder="Paste URLs here, one per line or space-separated"
              value={text}
              onChange={(e) => setText(e.target.value)}
              disabled={busy}
              // eslint-disable-next-line jsx-a11y/no-autofocus
              autoFocus
            />
            <p className="field-hint">HTTP and HTTPS links only.</p>
          </div>

          {progress && (
            <p className="field-hint" role="status">
              Queuing {progress.done} / {progress.total}…
            </p>
          )}

          <div className="dialog-footer">
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
                type="button"
                className="button button-primary"
                onClick={() => void handleAddAll()}
                disabled={busy || !text.trim()}
              >
                {busy ? `Queuing ${progress?.done ?? 0} / ${progress?.total ?? 0}…` : "Add all"}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
