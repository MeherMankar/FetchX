import { useState } from "react";
import { setDownloadBandwidthLimit } from "../api/history";
import { errorMessage } from "../lib/format";

interface Props {
  downloadId: string;
  filename: string;
  currentLimit: number | null;
  onSaved: () => void;
  onClose: () => void;
}

type Unit = "kbps" | "mbps";

function toBytes(value: number, unit: Unit): number {
  return unit === "mbps" ? Math.round(value * 1024 * 1024) : Math.round(value * 1024);
}

export function BandwidthLimitDialog({ downloadId, filename, currentLimit, onSaved, onClose }: Props) {
  const [value, setValue] = useState(
    currentLimit ? (currentLimit >= 1024 * 1024 ? (currentLimit / (1024 * 1024)).toFixed(1) : (currentLimit / 1024).toFixed(0)) : ""
  );
  const [unit, setUnit] = useState<Unit>(currentLimit && currentLimit >= 1024 * 1024 ? "mbps" : "kbps");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    try {
      const bps = value.trim() === "" ? null : toBytes(parseFloat(value), unit);
      await setDownloadBandwidthLimit(downloadId, bps);
      onSaved();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="bw-title">
      <div className="dialog-panel">
        <h2 id="bw-title" className="dialog-title">Speed limit</h2>
        <p className="dialog-subtitle" title={filename}><strong>{filename}</strong></p>
        {error && <div className="error-banner" role="alert">{error}</div>}
        <div className="settings-row">
          <label className="settings-label" htmlFor="bw-value">Limit</label>
          <div className="settings-control" style={{ display: "flex", gap: "0.5rem" }}>
            <input
              id="bw-value"
              type="number"
              min={1}
              step={1}
              className="settings-input settings-input-sm"
              placeholder="Unlimited"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              disabled={busy}
            />
            <select
              className="settings-input settings-input-sm"
              value={unit}
              onChange={(e) => setUnit(e.target.value as Unit)}
              disabled={busy}
            >
              <option value="kbps">KB/s</option>
              <option value="mbps">MB/s</option>
            </select>
          </div>
        </div>
        <p className="settings-hint">Leave empty to remove the limit.</p>
        <div className="dialog-actions">
          <button className="button button-secondary" onClick={onClose} disabled={busy}>Cancel</button>
          <button className="button button-primary" onClick={() => void save()} disabled={busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
