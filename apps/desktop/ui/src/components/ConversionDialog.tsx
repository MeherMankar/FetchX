import { useState } from "react";
import { startMediaRemux } from "../api/downloads";
import type { Download, MediaProcessJob } from "../api/types";
import { errorMessage } from "../lib/format";

type OutputFormat = "mp4" | "mkv" | "webm" | "mp3" | "flac" | "wav" | "vtt";

const FORMAT_LABELS: Record<OutputFormat, string> = {
  mp4: "MP4 (remux)",
  mkv: "Matroska (remux)",
  webm: "WebM (remux)",
  mp3: "MP3 (audio extract)",
  flac: "FLAC (audio extract)",
  wav: "WAV (audio extract)",
  vtt: "WebVTT (subtitle)",
};

interface Props {
  download: Download;
  onStarted: (job: MediaProcessJob) => void;
  onClose: () => void;
}

export function ConversionDialog({ download, onStarted, onClose }: Props) {
  const [format, setFormat] = useState<OutputFormat>("mp4");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const start = async () => {
    setBusy(true);
    try {
      const job = await startMediaRemux(download.id, format as "mp4" | "mkv" | "webm" | "vtt");
      onStarted(job);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="conv-title">
      <div className="dialog-panel">
        <h2 id="conv-title" className="dialog-title">Convert / Remux</h2>
        <p className="dialog-subtitle" title={download.filename}>
          <strong>{download.filename}</strong>
        </p>
        {error && <div className="error-banner" role="alert">{error}</div>}
        <div className="settings-row">
          <label className="settings-label" htmlFor="conv-format">Output format</label>
          <select
            id="conv-format"
            className="settings-input"
            value={format}
            onChange={(e) => setFormat(e.target.value as OutputFormat)}
            disabled={busy}
          >
            {(Object.entries(FORMAT_LABELS) as [OutputFormat, string][]).map(([k, label]) => (
              <option key={k} value={k}>{label}</option>
            ))}
          </select>
        </div>
        <p className="settings-hint">
          Stream-copy (remux) preserves quality and is fast. Audio extract decodes then re-encodes.
        </p>
        <div className="dialog-actions">
          <button className="button button-secondary" onClick={onClose} disabled={busy}>Cancel</button>
          <button className="button button-primary" onClick={() => void start()} disabled={busy}>
            {busy ? "Starting…" : "Convert"}
          </button>
        </div>
      </div>
    </div>
  );
}
