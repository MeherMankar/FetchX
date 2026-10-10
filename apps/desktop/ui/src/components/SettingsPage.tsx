import { useCallback, useEffect, useState } from "react";

import {
  getEngineConfig,
  resetEngineConfig,
  saveEngineConfig,
  testAria2Config,
  testYtDlpConfig,
} from "../api/engine";
import type {
  DiagnosticResult,
  EngineConfigDto,
  SaveAria2ConfigRequest,
  SaveYtDlpConfigRequest,
} from "../api/types";
import { errorMessage } from "../lib/format";

// ---------------------------------------------------------------------------
// Status indicator
// ---------------------------------------------------------------------------

function StatusDot({
  available,
  label,
}: {
  available: boolean | null;
  label: string;
}) {
  const cls =
    available === null
      ? "dot-idle"
      : available
        ? "dot-ok"
        : "dot-err";
  return (
    <span className={`status-dot ${cls}`} aria-label={label} title={label} />
  );
}

// ---------------------------------------------------------------------------
// Diagnostic display
// ---------------------------------------------------------------------------

function DiagnosticDisplay({ result }: { result: DiagnosticResult | null }) {
  if (!result) return null;
  return (
    <div
      className={`diagnostic-box ${result.available ? "diagnostic-ok" : "diagnostic-err"}`}
      role="status"
      aria-live="polite"
    >
      <StatusDot available={result.available} label={result.available ? "Available" : "Unavailable"} />
      <div className="diagnostic-text">
        <span>{result.message}</span>
        {result.version && (
          <span className="diagnostic-version">Version: {result.version}</span>
        )}
        {result.executable && (
          <span className="diagnostic-path" title={result.executable}>
            {result.executable}
          </span>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Aria2 section
// ---------------------------------------------------------------------------

interface Aria2SectionProps {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (msg: string) => void;
}

function Aria2Section({ cfg, onSaved, onError }: Aria2SectionProps) {
  const [enabled, setEnabled] = useState(cfg.aria2.enabled);
  const [execPath, setExecPath] = useState(cfg.aria2.executable_path ?? "");
  const [port, setPort] = useState(String(cfg.aria2.rpc_port));
  const [secretUpdate, setSecretUpdate] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [diagnostic, setDiagnostic] = useState<DiagnosticResult | null>(null);
  const [testBusy, setTestBusy] = useState(false);

  // Sync if parent config changes (e.g. after reset).
  useEffect(() => {
    setEnabled(cfg.aria2.enabled);
    setExecPath(cfg.aria2.executable_path ?? "");
    setPort(String(cfg.aria2.rpc_port));
    setSecretUpdate(null);
    setDiagnostic(null);
  }, [cfg]);

  const save = async () => {
    const portNum = parseInt(port, 10);
    if (!Number.isInteger(portNum) || portNum < 1 || portNum > 65535) {
      onError("RPC port must be a number between 1 and 65535.");
      return;
    }
    setBusy(true);
    try {
      const req: SaveAria2ConfigRequest = {
        enabled,
        executable_path: execPath.trim() || null,
        rpc_port: portNum,
        rpc_secret_update: secretUpdate,
      };
      const updated = await saveEngineConfig({
        aria2: req,
        yt_dlp: {
          enabled: cfg.yt_dlp.enabled,
          executable_path: cfg.yt_dlp.executable_path,
        },
      });
      onSaved(updated);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setTestBusy(true);
    setDiagnostic(null);
    try {
      const result = await testAria2Config();
      setDiagnostic(result);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setTestBusy(false);
    }
  };

  return (
    <section className="settings-section" aria-labelledby="aria2-heading">
      <h3 id="aria2-heading" className="settings-section-title">aria2</h3>

      <div className="settings-row">
        <label className="settings-label" htmlFor="aria2-enabled">Enabled</label>
        <div className="settings-control">
          <input
            id="aria2-enabled"
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            disabled={busy}
          />
        </div>
      </div>

      <div className="settings-row">
        <label className="settings-label" htmlFor="aria2-exec">Executable path</label>
        <div className="settings-control">
          <input
            id="aria2-exec"
            type="text"
            className="settings-input"
            placeholder="aria2c (discovered from PATH)"
            value={execPath}
            onChange={(e) => setExecPath(e.target.value)}
            disabled={busy}
          />
          <p className="settings-hint">
            Leave blank to discover from PATH. Must be an absolute path to aria2c.
          </p>
        </div>
      </div>

      <div className="settings-row">
        <label className="settings-label" htmlFor="aria2-port">RPC port</label>
        <div className="settings-control">
          <input
            id="aria2-port"
            type="number"
            className="settings-input settings-input-sm"
            min={1}
            max={65535}
            value={port}
            onChange={(e) => setPort(e.target.value)}
            disabled={busy}
          />
          <p className="settings-hint">
            RPC listens on loopback only. Default: 6800.
          </p>
        </div>
      </div>

      <div className="settings-row">
        <label className="settings-label" htmlFor="aria2-secret">RPC secret</label>
        <div className="settings-control">
          {secretUpdate === null ? (
            <div className="secret-display">
              <span className="secret-badge">
                {cfg.aria2.has_secret ? "Configured" : "Not set"}
              </span>
              <button
                className="button button-secondary button-xs"
                onClick={() => setSecretUpdate("")}
                disabled={busy}
              >
                {cfg.aria2.has_secret ? "Change" : "Set"}
              </button>
              {cfg.aria2.has_secret && (
                <button
                  className="button button-secondary button-xs"
                  onClick={() => setSecretUpdate("")}
                  disabled={busy}
                >
                  Clear
                </button>
              )}
            </div>
          ) : (
            <div className="secret-edit">
              <input
                id="aria2-secret"
                type="password"
                className="settings-input"
                placeholder="Enter new secret (no spaces)"
                value={secretUpdate}
                onChange={(e) => setSecretUpdate(e.target.value)}
                disabled={busy}
                autoComplete="new-password"
              />
              <button
                className="button button-secondary button-xs"
                onClick={() => setSecretUpdate(null)}
                disabled={busy}
              >
                Cancel
              </button>
            </div>
          )}
          <p className="settings-hint">
            Secret is stored in the application database. It is never shown
            in the UI after saving.
          </p>
        </div>
      </div>

      <DiagnosticDisplay result={diagnostic} />

      <div className="settings-actions">
        <button
          className="button button-secondary"
          onClick={() => void test()}
          disabled={busy || testBusy}
        >
          {testBusy ? "Testing…" : "Test configuration"}
        </button>
        <button
          className="button button-primary"
          onClick={() => void save()}
          disabled={busy || testBusy}
        >
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// yt-dlp section
// ---------------------------------------------------------------------------

interface YtDlpSectionProps {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (msg: string) => void;
}

function YtDlpSection({ cfg, onSaved, onError }: YtDlpSectionProps) {
  const [enabled, setEnabled] = useState(cfg.yt_dlp.enabled);
  const [execPath, setExecPath] = useState(cfg.yt_dlp.executable_path ?? "");
  const [busy, setBusy] = useState(false);
  const [diagnostic, setDiagnostic] = useState<DiagnosticResult | null>(null);
  const [testBusy, setTestBusy] = useState(false);

  useEffect(() => {
    setEnabled(cfg.yt_dlp.enabled);
    setExecPath(cfg.yt_dlp.executable_path ?? "");
    setDiagnostic(null);
  }, [cfg]);

  const save = async () => {
    setBusy(true);
    try {
      const req: SaveYtDlpConfigRequest = {
        enabled,
        executable_path: execPath.trim() || null,
      };
      const updated = await saveEngineConfig({
        aria2: {
          enabled: cfg.aria2.enabled,
          executable_path: cfg.aria2.executable_path,
          rpc_port: cfg.aria2.rpc_port,
          rpc_secret_update: null,
        },
        yt_dlp: req,
      });
      onSaved(updated);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setTestBusy(true);
    setDiagnostic(null);
    try {
      const result = await testYtDlpConfig();
      setDiagnostic(result);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setTestBusy(false);
    }
  };

  return (
    <section className="settings-section" aria-labelledby="ytdlp-heading">
      <h3 id="ytdlp-heading" className="settings-section-title">yt-dlp</h3>

      <div className="settings-row">
        <label className="settings-label" htmlFor="ytdlp-enabled">Enabled</label>
        <div className="settings-control">
          <input
            id="ytdlp-enabled"
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            disabled={busy}
          />
        </div>
      </div>

      <div className="settings-row">
        <label className="settings-label" htmlFor="ytdlp-exec">Executable path</label>
        <div className="settings-control">
          <input
            id="ytdlp-exec"
            type="text"
            className="settings-input"
            placeholder="yt-dlp (discovered from PATH)"
            value={execPath}
            onChange={(e) => setExecPath(e.target.value)}
            disabled={busy}
          />
          <p className="settings-hint">
            Leave blank to discover from PATH. Must point to the yt-dlp executable.
          </p>
        </div>
      </div>

      <DiagnosticDisplay result={diagnostic} />

      <div className="settings-actions">
        <button
          className="button button-secondary"
          onClick={() => void test()}
          disabled={busy || testBusy}
        >
          {testBusy ? "Testing…" : "Test configuration"}
        </button>
        <button
          className="button button-primary"
          onClick={() => void save()}
          disabled={busy || testBusy}
        >
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Direct HTTP section
// ---------------------------------------------------------------------------

function DirectHttpSection() {
  return (
    <section className="settings-section" aria-labelledby="directhttp-heading">
      <h3 id="directhttp-heading" className="settings-section-title">Direct HTTP</h3>
      <div className="settings-row">
        <span className="settings-label">Status</span>
        <div className="settings-control">
          <span className="settings-builtin-badge">
            <StatusDot available={true} label="Built-in" />
            Built-in — always available
          </span>
          <p className="settings-hint">
            Direct HTTP/HTTPS downloads are handled by the built-in engine and
            cannot be disabled.
          </p>
        </div>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Root SettingsPage
// ---------------------------------------------------------------------------

export function SettingsPage() {
  const [cfg, setCfg] = useState<EngineConfigDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [resetBusy, setResetBusy] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const loaded = await getEngineConfig();
      setCfg(loaded);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleReset = async () => {
    setResetBusy(true);
    try {
      const defaults = await resetEngineConfig();
      setCfg(defaults);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setResetBusy(false);
    }
  };

  if (loading) {
    return (
      <div className="settings-loading" role="status">
        Loading settings…
      </div>
    );
  }

  return (
    <div className="settings-page">
      <div className="settings-header">
        <h2 className="settings-title">Download Engines</h2>
        <button
          className="button button-secondary"
          onClick={() => void handleReset()}
          disabled={resetBusy}
        >
          {resetBusy ? "Resetting…" : "Reset to defaults"}
        </button>
      </div>

      {error && (
        <div className="error-banner" role="alert">
          <span>{error}</span>
          <button
            className="icon-button"
            aria-label="Dismiss error"
            onClick={() => setError(null)}
          >
            ×
          </button>
        </div>
      )}

      <div className="settings-notice" role="note">
        <strong>Note:</strong> Changing engine settings takes effect for new downloads.
        Existing active downloads continue using their assigned engine. Disabling an
        engine does not cancel downloads already in progress.
      </div>

      {cfg && (
        <>
          <DirectHttpSection />
          <Aria2Section
            cfg={cfg}
            onSaved={setCfg}
            onError={setError}
          />
          <YtDlpSection
            cfg={cfg}
            onSaved={setCfg}
            onError={setError}
          />
        </>
      )}
    </div>
  );
}
