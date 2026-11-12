import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  getBrowserExtensionStatus,
  openBrowserExtensionInstallPage,
} from "../api/browserExtensions";
import {
  cancelHelperInstall,
  getHelperStatus,
  getEngineConfig,
  getEngineStatus,
  installHelper,
  resetEngineConfig,
  saveEngineConfig,
  testYtDlpConfig,
} from "../api/engine";
import type {
  BrowserExtensionStatus,
  DiagnosticResult,
  EngineConfigDto,
  EngineInstanceStatus,
  EngineStatus,
  HelperInstallProgress,
  HelperStatus,
  ManagedHelperId,
  ResourcePolicyDto,
  ResourceProfile,
  SaveYtDlpConfigRequest,
  WeeklyQueueScheduleDto,
} from "../api/types";
import { errorMessage } from "../lib/format";

// ---------------------------------------------------------------------------
// Status indicator
// ---------------------------------------------------------------------------

function BrowserExtensionSection() {
  const [status, setStatus] = useState<BrowserExtensionStatus | null>(null);
  const [busyBrowser, setBusyBrowser] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await getBrowserExtensionStatus());
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const openInstallPage = async (browser: string) => {
    setBusyBrowser(browser);
    setMessage(null);
    try {
      await openBrowserExtensionInstallPage(browser);
      setMessage(`Opened ${browser}. Follow the local-install steps below to add FetchX.`);
      await refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusyBrowser(null);
    }
  };

  return (
    <section className="settings-section" aria-labelledby="browser-extension-heading">
      <div className="settings-section-header">
        <h3 id="browser-extension-heading" className="settings-section-title">
          Browser extensions
        </h3>
      </div>

      <p className="settings-hint">
        FetchX is not yet published in the browser stores, so browsers cannot
        automatically install the extension. Open a browser below and load the
        local extension once.
      </p>

      {error && <div className="error-banner" role="alert">{error}</div>}
      {message && <div className="settings-notice" role="status">{message}</div>}

      {status && (
        <>
          <div className="settings-row">
            <span className="settings-label">Native host</span>
            <div className="settings-control">
              <span>
                {status.native_host_binary_path
                  ? `Available: ${status.native_host_binary_path}`
                  : "Not found. Browser downloads will not reach FetchX until the native host is installed."}
              </span>
            </div>
          </div>

          <div className="settings-row">
            <span className="settings-label">Chromium extension</span>
            <div className="settings-control">
              <code className="settings-path">{status.chromium_extension_path}</code>
              <span className="settings-hint">
                In Chrome, Edge, or another Chromium browser, enable Developer mode
                and choose “Load unpacked”, then select this folder.
              </span>
            </div>
          </div>

          <div className="settings-row">
            <span className="settings-label">Firefox extension</span>
            <div className="settings-control">
              <code className="settings-path">{status.firefox_extension_path}</code>
              <span className="settings-hint">
                In Firefox, choose “Load Temporary Add-on” and select this folder’s
                manifest.json. Temporary add-ons are removed when Firefox exits;
                permanent installation requires a signed add-on.
              </span>
            </div>
          </div>

          <div className="browser-extension-list">
            {status.browsers.filter((browser) => browser.detected).map((browser) => (
              <div className="browser-extension-item" key={browser.browser}>
                <span>{browser.browser}</span>
                <span className="settings-hint">
                  {browser.native_host_registered ? "Native host ready" : "Native host not registered"}
                </span>
                <button
                  className="button button-secondary"
                  onClick={() => void openInstallPage(browser.browser)}
                  disabled={busyBrowser !== null}
                >
                  {busyBrowser === browser.browser ? "Opening…" : "Open install page"}
                </button>
              </div>
            ))}
            {status.browsers.every((browser) => !browser.detected) && (
              <p className="settings-hint">
                No supported browsers were detected in standard Windows locations.
              </p>
            )}
          </div>
        </>
      )}
    </section>
  );
}

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
// yt-dlp section
// ---------------------------------------------------------------------------

interface YtDlpSectionProps {
  cfg: EngineConfigDto;
  runtimeStatus: EngineInstanceStatus | null;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (msg: string) => void;
}

function YtDlpSection({ cfg, runtimeStatus, onSaved, onError }: YtDlpSectionProps) {
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
        yt_dlp: req,
        resource_policy: cfg.resource_policy,
        weekly_queue_schedule: cfg.weekly_queue_schedule,
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
      <div className="settings-section-header">
        <h3 id="ytdlp-heading" className="settings-section-title">yt-dlp</h3>
        <RuntimeStatusBadge status={runtimeStatus} />
      </div>

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

function DirectHttpSection({ runtimeStatus }: { runtimeStatus: EngineInstanceStatus | null }) {
  return (
    <section className="settings-section" aria-labelledby="directhttp-heading">
      <div className="settings-section-header">
        <h3 id="directhttp-heading" className="settings-section-title">Direct HTTP</h3>
        <RuntimeStatusBadge status={runtimeStatus ?? { enabled: true, available: true, version: null, error: null }} />
      </div>
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
// Runtime status badge
// ---------------------------------------------------------------------------

function RuntimeStatusBadge({ status }: { status: EngineInstanceStatus | null }) {
  if (!status) return null;

  if (!status.enabled) {
    return <span className="runtime-badge runtime-badge-disabled">Disabled</span>;
  }
  if (status.available) {
    return (
      <span className="runtime-badge runtime-badge-ok">
        ● Running{status.version ? ` · ${status.version}` : ""}
      </span>
    );
  }
  return (
    <span
      className="runtime-badge runtime-badge-err"
      title={status.error ?? undefined}
    >
      ● Unavailable
    </span>
  );
}

// ---------------------------------------------------------------------------
// Root SettingsPage
// ---------------------------------------------------------------------------

function isResourceProfile(value: string): value is ResourceProfile {
  return ["unlimited", "normal", "limited", "night", "custom"].includes(value);
}

function HelperInstallPanel({
  onConfigChanged,
}: {
  onConfigChanged: (config: EngineConfigDto) => void;
}) {
  const [helpers, setHelpers] = useState<HelperStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [progress, setProgress] = useState<Partial<Record<ManagedHelperId, HelperInstallProgress>>>({});
  const [errors, setErrors] = useState<Partial<Record<ManagedHelperId, string>>>({});

  const refresh = useCallback(async () => {
    try {
      setHelpers(await getHelperStatus());
    } catch (cause) {
      setErrors((current) => ({ ...current, yt_dlp: errorMessage(cause) }));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    let unlisten: (() => void) | undefined;
    void listen<HelperInstallProgress>("helper-install-progress", (event) => {
      const update = event.payload;
      setProgress((current) => ({ ...current, [update.helper]: update }));
      if (update.phase === "completed" || update.phase === "failed" || update.phase === "cancelled") {
        if (update.message) {
          setErrors((current) => ({ ...current, [update.helper]: update.message ?? undefined }));
        }
        if (update.phase === "completed") {
          setErrors((current) => ({ ...current, [update.helper]: undefined }));
          void refresh();
        }
      }
    }).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
  }, [refresh]);

  const install = async (helper: ManagedHelperId) => {
    setErrors((current) => ({ ...current, [helper]: undefined }));
    try {
      await installHelper(helper);
      onConfigChanged(await getEngineConfig());
      await refresh();
    } catch (cause) {
      setErrors((current) => ({ ...current, [helper]: errorMessage(cause) }));
      await refresh();
    }
  };

  const cancel = async (helper: ManagedHelperId) => {
    try {
      await cancelHelperInstall(helper);
    } catch (cause) {
      setErrors((current) => ({ ...current, [helper]: errorMessage(cause) }));
    }
  };

  const items: Array<{ id: ManagedHelperId; name: string }> = [
    { id: "yt_dlp", name: "yt-dlp" },
    { id: "ffmpeg", name: "FFmpeg" },
    { id: "ffprobe", name: "FFprobe" },
  ];

  return (
    <section className="settings-section" aria-labelledby="helper-manager-heading">
      <div className="settings-section-header">
        <h3 id="helper-manager-heading" className="settings-section-title">Optional helper manager</h3>
      </div>
      <p className="settings-hint">
        Optional tools are installed only when you choose Install. FetchX downloads from fixed HTTPS sources,
        verifies the published SHA-256 checksum, and stores executables in its per-user application data folder.
        Engine changes take effect after restarting FetchX.
      </p>
      {loading ? (
        <div className="settings-loading" role="status">Checking installed helpers…</div>
      ) : (
        <div className="helper-manager-list">
          {items.map(({ id, name }) => {
            const status = helpers.find((helper) => helper.helper === id);
            const current = progress[id];
            const busy = current !== undefined
              && ["downloading", "verifying", "installing"].includes(current.phase);
            const packageBusy = id === "ffmpeg" || id === "ffprobe"
              ? progress.ffmpeg !== undefined && ["downloading", "verifying", "installing"].includes(progress.ffmpeg.phase)
                || progress.ffprobe !== undefined && ["downloading", "verifying", "installing"].includes(progress.ffprobe.phase)
              : busy;
            return (
              <div className="helper-manager-item" key={id}>
                <div className="helper-manager-details">
                  <strong>{name}</strong>
                  <span className={status?.available ? "helper-manager-available" : "helper-manager-missing"}>
                    {status?.available ? `Installed · ${status.version}` : status?.installed ? "Installed · validation failed" : "Not installed"}
                  </span>
                  {status?.executable_path && (
                    <span className="helper-manager-path" title={status.executable_path}>
                      {status.executable_path}
                    </span>
                  )}
                  {status?.architecture && <span className="settings-hint">Architecture: {status.architecture}</span>}
                  {!status?.available && status?.validation && (
                    <span className="settings-hint">{status.validation}</span>
                  )}
                  {current && busy && (
                    <span className="settings-hint" role="status" aria-live="polite">
                      {current.phase === "downloading" && current.total_bytes
                        ? `Downloading… ${Math.round(current.received_bytes * 100 / current.total_bytes)}%`
                        : `${current.phase[0].toUpperCase()}${current.phase.slice(1)}…`}
                    </span>
                  )}
                  {errors[id] && <span className="helper-manager-error" role="alert">{errors[id]}</span>}
                </div>
                <div className="helper-manager-actions">
                  <button
                    className="button button-secondary"
                    disabled={!status?.install_supported || packageBusy}
                    onClick={() => void install(id)}
                  >
                    {packageBusy ? "Working…" : status?.installed ? "Reinstall / Update" : "Install"}
                  </button>
                  {packageBusy && current?.phase !== "installing" && (
                    <button
                      className="button button-secondary"
                      onClick={() => void cancel(id)}
                    >
                      Cancel
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
      <p className="settings-hint">
        yt-dlp uses its upstream release artifacts.
        FFmpeg and FFprobe use Gyan.dev’s Windows essentials build and published checksum. Windows x64 is supported;
        other architectures are rejected.
      </p>
    </section>
  );
}

function policyForProfile(
  profile: ResourceProfile,
  current: ResourcePolicyDto,
): ResourcePolicyDto {
  const engineLimits = {
    direct_http_bytes_per_second: null,
    hls_bytes_per_second: null,
    yt_dlp_bytes_per_second: null,
  };
  const profiles: Partial<Record<ResourceProfile, ResourcePolicyDto>> = {
    unlimited: {
      profile,
      max_active_downloads: 8,
      global_connection_limit: 64,
      per_download_connection_limit: 32,
      global_bandwidth_bytes_per_second: null,
      per_download_bandwidth_bytes_per_second: null,
      engine_bandwidth_limits: engineLimits,
    },
    normal: {
      profile,
      max_active_downloads: 3,
      global_connection_limit: 16,
      per_download_connection_limit: 8,
      global_bandwidth_bytes_per_second: null,
      per_download_bandwidth_bytes_per_second: null,
      engine_bandwidth_limits: engineLimits,
    },
    limited: {
      profile,
      max_active_downloads: 2,
      global_connection_limit: 8,
      per_download_connection_limit: 4,
      global_bandwidth_bytes_per_second: 2 * 1024 * 1024,
      per_download_bandwidth_bytes_per_second: 1024 * 1024,
      engine_bandwidth_limits: engineLimits,
    },
    night: {
      profile,
      max_active_downloads: 2,
      global_connection_limit: 4,
      per_download_connection_limit: 2,
      global_bandwidth_bytes_per_second: 512 * 1024,
      per_download_bandwidth_bytes_per_second: 256 * 1024,
      engine_bandwidth_limits: engineLimits,
    },
  };
  return profiles[profile] ?? { ...current, profile };
}

function ResourcePolicySection({
  cfg,
  onSaved,
  onError,
}: {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (message: string) => void;
}) {
  const [policy, setPolicy] = useState<ResourcePolicyDto>(cfg.resource_policy);
  const [busy, setBusy] = useState(false);

  useEffect(() => setPolicy(cfg.resource_policy), [cfg.resource_policy]);

  const save = async () => {
    setBusy(true);
    try {
      const updated = await saveEngineConfig({
        yt_dlp: {
          enabled: cfg.yt_dlp.enabled,
          executable_path: cfg.yt_dlp.executable_path,
        },
        resource_policy: policy,
        weekly_queue_schedule: cfg.weekly_queue_schedule,
      });
      onSaved(updated);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const updateNumber = (
    key: "max_active_downloads" | "global_connection_limit" |
      "per_download_connection_limit",
    value: string,
  ) => {
    const parsed = Number.parseInt(value, 10);
    setPolicy((current) => ({
      ...current,
      profile: "custom",
      [key]: Number.isFinite(parsed) ? parsed : 0,
    }));
  };

  const updateBandwidth = (
    key: "global_bandwidth_bytes_per_second" |
      "per_download_bandwidth_bytes_per_second",
    value: string,
  ) => {
    const parsed = value === "" ? null : Number.parseInt(value, 10);
    setPolicy((current) => ({
      ...current,
      profile: "custom",
      [key]: parsed !== null && Number.isFinite(parsed) ? parsed : null,
    }));
  };

  return (
    <section className="settings-section" aria-labelledby="resources-heading">
      <div className="settings-section-header">
        <h3 id="resources-heading" className="settings-section-title">Resource policy</h3>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-profile">Profile</label>
        <div className="settings-control">
          <select
            id="resource-profile"
            className="settings-input"
            value={policy.profile}
            disabled={busy}
            onChange={(event) => {
              const profile = event.target.value;
              if (isResourceProfile(profile)) {
                setPolicy((current) => policyForProfile(profile, current));
              }
            }}
          >
            <option value="unlimited">Unlimited (bounded safety ceilings)</option>
            <option value="normal">Normal</option>
            <option value="limited">Limited</option>
            <option value="night">Night</option>
            <option value="custom">Custom</option>
          </select>
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-active">Maximum active downloads</label>
        <div className="settings-control">
          <input id="resource-active" type="number" min={1} max={32}
            className="settings-input settings-input-sm"
            value={policy.max_active_downloads} disabled={busy}
            onChange={(event) => updateNumber("max_active_downloads", event.target.value)} />
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-global-connections">Global connections</label>
        <div className="settings-control">
          <input id="resource-global-connections" type="number" min={1} max={64}
            className="settings-input settings-input-sm"
            value={policy.global_connection_limit} disabled={busy}
            onChange={(event) => updateNumber("global_connection_limit", event.target.value)} />
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-download-connections">Connections per download</label>
        <div className="settings-control">
          <input id="resource-download-connections" type="number" min={1} max={32}
            className="settings-input settings-input-sm"
            value={policy.per_download_connection_limit} disabled={busy}
            onChange={(event) => updateNumber("per_download_connection_limit", event.target.value)} />
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-global-bandwidth">Global bandwidth (bytes/s)</label>
        <div className="settings-control">
          <input id="resource-global-bandwidth" type="number" min={1}
            className="settings-input settings-input-sm"
            value={policy.global_bandwidth_bytes_per_second ?? ""} disabled={busy}
            onChange={(event) => updateBandwidth("global_bandwidth_bytes_per_second", event.target.value)} />
          <p className="settings-hint">Leave empty for unlimited.</p>
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-download-bandwidth">Bandwidth per download (bytes/s)</label>
        <div className="settings-control">
          <input id="resource-download-bandwidth" type="number" min={1}
            className="settings-input settings-input-sm"
            value={policy.per_download_bandwidth_bytes_per_second ?? ""} disabled={busy}
            onChange={(event) => updateBandwidth("per_download_bandwidth_bytes_per_second", event.target.value)} />
        </div>
      </div>
      <p className="settings-hint">
        HTTP and HLS use shared Rust bandwidth limits. yt-dlp uses native per-download limits where supported.
        FFmpeg processing has its own local process ceiling and is not network bandwidth limited.
        The queue accepts explicit pressure signals: elevated pressure reduces new allocations, high pressure limits new starts, and critical pressure blocks new work without stopping active downloads. Automatic OS-pressure detection is not wired.
      </p>
      <div className="settings-actions">
        <button className="button button-primary" onClick={() => void save()} disabled={busy}>
          {busy ? "Saving…" : "Save resource policy"}
        </button>
      </div>
    </section>
  );
}

function formatScheduleTime(minute: number): string {
  const hours = Math.floor(minute / 60).toString().padStart(2, "0");
  const minutes = (minute % 60).toString().padStart(2, "0");
  return `${hours}:${minutes}`;
}

function parseScheduleTime(value: string): number | null {
  const match = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(value);
  return match ? Number(match[1]) * 60 + Number(match[2]) : null;
}

const WEEKDAY_LABELS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function WeeklyQueueScheduleSection({
  cfg,
  onSaved,
  onError,
}: {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (message: string) => void;
}) {
  const [schedule, setSchedule] = useState<WeeklyQueueScheduleDto>(cfg.weekly_queue_schedule);
  const [busy, setBusy] = useState(false);

  useEffect(() => setSchedule(cfg.weekly_queue_schedule), [cfg.weekly_queue_schedule]);

  const save = async () => {
    if (schedule.enabled && !schedule.weekdays.some(Boolean)) {
      onError("Choose at least one day for the download window.");
      return;
    }
    if (schedule.start_minute === schedule.end_minute) {
      onError("The download window start and end times must be different.");
      return;
    }

    setBusy(true);
    try {
      const updated = await saveEngineConfig({
        yt_dlp: {
          enabled: cfg.yt_dlp.enabled,
          executable_path: cfg.yt_dlp.executable_path,
        },
        resource_policy: cfg.resource_policy,
        weekly_queue_schedule: schedule,
      });
      onSaved(updated);
      onError("");
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const updateTime = (key: "start_minute" | "end_minute", value: string) => {
    const minute = parseScheduleTime(value);
    if (minute !== null) {
      setSchedule((current) => ({ ...current, [key]: minute }));
    }
  };

  return (
    <section className="settings-section" aria-labelledby="queue-schedule-heading">
      <div className="settings-section-header">
        <h3 id="queue-schedule-heading" className="settings-section-title">
          Download schedule
        </h3>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="weekly-schedule-enabled">
          Weekly download window
        </label>
        <div className="settings-control">
          <input
            id="weekly-schedule-enabled"
            type="checkbox"
            checked={schedule.enabled}
            disabled={busy}
            onChange={(event) =>
              setSchedule((current) => ({ ...current, enabled: event.target.checked }))
            }
          />
          <p className="settings-hint">
            Outside this window, new queued downloads wait. Active downloads continue.
            Times use this computer’s local time.
          </p>
        </div>
      </div>
      <div className="settings-row">
        <span className="settings-label">Days</span>
        <div className="settings-control weekday-picker" role="group" aria-label="Download days">
          {WEEKDAY_LABELS.map((label, index) => (
            <button
              key={label}
              type="button"
              className={`weekday-button${schedule.weekdays[index] ? " weekday-button-active" : ""}`}
              aria-pressed={schedule.weekdays[index]}
              disabled={busy || !schedule.enabled}
              onClick={() =>
                setSchedule((current) => ({
                  ...current,
                  weekdays: current.weekdays.map((selected, day) =>
                    day === index ? !selected : selected,
                  ),
                }))
              }
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="weekly-schedule-start">Start</label>
        <div className="settings-control">
          <input
            id="weekly-schedule-start"
            type="time"
            className="settings-input settings-input-sm"
            value={formatScheduleTime(schedule.start_minute)}
            disabled={busy || !schedule.enabled}
            onChange={(event) => updateTime("start_minute", event.target.value)}
          />
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="weekly-schedule-end">End</label>
        <div className="settings-control">
          <input
            id="weekly-schedule-end"
            type="time"
            className="settings-input settings-input-sm"
            value={formatScheduleTime(schedule.end_minute)}
            disabled={busy || !schedule.enabled}
            onChange={(event) => updateTime("end_minute", event.target.value)}
          />
          <p className="settings-hint">Overnight windows, such as 22:00–06:00, are supported.</p>
        </div>
      </div>
      <div className="settings-actions">
        <button className="button button-primary" onClick={() => void save()} disabled={busy}>
          {busy ? "Saving…" : "Save download schedule"}
        </button>
      </div>
    </section>
  );
}

export function SettingsPage() {
  const [cfg, setCfg] = useState<EngineConfigDto | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<EngineStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [resetBusy, setResetBusy] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [loaded, status] = await Promise.all([
        getEngineConfig(),
        getEngineStatus(),
      ]);
      setCfg(loaded);
      setRuntimeStatus(status);
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
        <strong>Note:</strong> Engine and resource-policy changes take effect after
        restarting FetchX. The weekly queue schedule applies immediately. Active
        downloads keep their existing runtime settings.
      </div>

      <BrowserExtensionSection />
      <HelperInstallPanel onConfigChanged={setCfg} />

      {cfg && (
        <>
          <WeeklyQueueScheduleSection cfg={cfg} onSaved={setCfg} onError={setError} />
          <ResourcePolicySection cfg={cfg} onSaved={setCfg} onError={setError} />
          <DirectHttpSection runtimeStatus={runtimeStatus?.direct_http ?? null} />
          <YtDlpSection
            cfg={cfg}
            runtimeStatus={runtimeStatus?.yt_dlp ?? null}
            onSaved={setCfg}
            onError={setError}
          />
        </>
      )}
    </div>
  );
}
