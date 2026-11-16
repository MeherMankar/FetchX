import { useState } from "react";
import { provideDownloadCredentials } from "../api/history";
import { errorMessage } from "../lib/format";

interface Props {
  downloadId: string;
  filename: string;
  onRetried: () => void;
  onClose: () => void;
}

export function CredentialsDialog({ downloadId, filename, onRetried, onClose }: Props) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!username.trim()) { setError("Username is required."); return; }
    setBusy(true);
    try {
      await provideDownloadCredentials(downloadId, username, password);
      onRetried();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="cred-title">
      <div className="dialog-panel">
        <h2 id="cred-title" className="dialog-title">Authentication required</h2>
        <p className="dialog-subtitle">
          <strong title={filename}>{filename}</strong> requires a username and password.
        </p>
        {error && <div className="error-banner" role="alert">{error}</div>}
        <div className="settings-row">
          <label className="settings-label" htmlFor="cred-user">Username</label>
          <input
            id="cred-user"
            type="text"
            className="settings-input"
            autoComplete="username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            disabled={busy}
          />
        </div>
        <div className="settings-row">
          <label className="settings-label" htmlFor="cred-pass">Password</label>
          <input
            id="cred-pass"
            type="password"
            className="settings-input"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            disabled={busy}
            onKeyDown={(e) => { if (e.key === "Enter") void submit(); }}
          />
        </div>
        <div className="dialog-actions">
          <button className="button button-secondary" onClick={onClose} disabled={busy}>Cancel</button>
          <button className="button button-primary" onClick={() => void submit()} disabled={busy}>
            {busy ? "Retrying…" : "Retry with credentials"}
          </button>
        </div>
      </div>
    </div>
  );
}
