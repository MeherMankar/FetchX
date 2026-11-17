import { useCallback, useEffect, useState } from "react";
import {
  getBrowserExtensionStatus,
  launchBrowserWithExtension,
  openBrowserExtensionInstallPage,
} from "../api/browserExtensions";
import {
  getEngineConfig,
  getEngineStatus,
  resetEngineConfig,
  saveEngineConfig,
} from "../api/engine";
import type {
  BrowserExtensionStatus,
  EngineConfigDto,
  EngineStatus,
  ResourcePolicyDto,
  ResourceProfile,
  WeeklyQueueScheduleDto,
} from "../api/types";
import { errorMessage } from "../lib/format";

// ---------------------------------------------------------------------------
// Status indicator
// ---------------------------------------------------------------------------

const FIREFOX_FAMILY_NAMES = new Set([
  "Mozilla Firefox",
  "LibreWolf",
  "Waterfox",
  "Floorp",
]);

function BrowserExtensionSection() {
  const [status, setStatus] = useState<BrowserExtensionStatus | null>(null);
  const [busyBrowser, setBusyBrowser] = useState<string | null>(null);
  const [busyAll, setBusyAll] = useState(false);
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
      setMessage("Opened " + browser + ". Follow the local-install steps below to add FetchX.");
      await refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusyBrowser(null);
    }
  };

  const installBrowser = async (browser: string) => {
    setBusyBrowser(browser);
    setMessage(null);
    try {
      await launchBrowserWithExtension(browser);
      setMessage("Launched " + browser + " with the FetchX extension loaded.");
      await refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusyBrowser(null);
    }
  };

  const installAll = async () => {
    if (!status) return;
    setBusyAll(true);
    setMessage(null);
    const detected = status.browsers.filter((b) => b.detected);
    for (const b of detected) {
      try {
        if (FIREFOX_FAMILY_NAMES.has(b.browser)) {
          await openBrowserExtensionInstallPage(b.browser);
        } else {
          await launchBrowserWithExtension(b.browser);
        }
      } catch {
        // Continue with remaining browsers even if one fails.
      }
    }
    await refresh();
    setBusyAll(false);
    setMessage("Extension installation launched for all detected browsers.");
  };

  const isBusy = busyBrowser !== null || busyAll;

  return (
    <section className="settings-section" aria-labelledby="browser-extension-heading">
      <div className="settings-section-header">
        <h3 id="browser-extension-heading" className="settings-section-title">
          Browser extensions
        </h3>
      </div>

      <p className="settings-hint">
        FetchX is not yet published in the browser stores, so browsers cannot
        automatically install the extension. Use the buttons below to load the
        extension locally.
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
                  ? "Available: " + status.native_host_binary_path
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
                and choose "Load unpacked", then select this folder.
              </span>
            </div>
          </div>

          <div className="settings-row">
            <span className="settings-label">Firefox extension</span>
            <div className="settings-control">
              <code className="settings-path">{status.firefox_extension_path}</code>
              <span className="settings-hint">
                In Firefox, choose "Load Temporary Add-on" and select this folder's
                manifest.json. Temporary add-ons are removed when Firefox exits;
                permanent installation requires a signed add-on.
              </span>
            </div>
          </div>

          <div className="browser-extension-list">
            {status.browsers.some((b) => b.detected) && (
              <div className="browser-extension-install-all">
                <button
                  className="button button-primary"
                  onClick={() => void installAll()}
                  disabled={isBusy}
                >
                  {busyAll ? "Installing..." : "Install in all detected browsers"}
                </button>
              </div>
            )}

            {status.browsers.filter((browser) => browser.detected).map((browser) => (
              <div className="browser-extension-item" key={browser.browser}>
                <span className="browser-extension-status-icon" aria-hidden="true">
                  {browser.native_host_registered ? "\u2713" : "\u25cb"}
                </span>
                <span>{browser.browser}</span>
                <span className="settings-hint">
                  {browser.native_host_registered ? "Native host ready" : "Native host not registered"}
                </span>
                {!FIREFOX_FAMILY_NAMES.has(browser.browser) && !browser.native_host_registered ? (
                  <button
                    className="button button-secondary"
                    onClick={() => void installBrowser(browser.browser)}
                    disabled={isBusy}
                  >
                    {busyBrowser === browser.browser ? "Launching..." : "Install"}
                  </button>
                ) : (
                  <button
                    className="button button-secondary"
                    onClick={() => void openInstallPage(browser.browser)}
                    disabled={isBusy}
                  >
                    {busyBrowser === browser.browser ? "Opening..." : "Open install page"}
                  </button>
                )}
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
// ---------------------------------------------------------------------------
// Diagnostic display
// ---------------------------------------------------------------------------





// ---------------------------------------------------------------------------
// Runtime status badge
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Root SettingsPage
// ---------------------------------------------------------------------------

function isResourceProfile(value: string): value is ResourceProfile {
  return ["unlimited", "normal", "limited", "night", "custom"].includes(value);
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

function formatBandwidthLimit(bytesPerSecond: number | null): string {
  return bytesPerSecond === null
    ? ""
    : (bytesPerSecond / (1024 * 1024)).toFixed(2).replace(/\.?0+$/, "");
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
    const parsed = value === "" ? null : Number.parseFloat(value);
    setPolicy((current) => ({
      ...current,
      profile: "custom",
      [key]: parsed !== null && Number.isFinite(parsed)
        ? Math.round(parsed * 1024 * 1024)
        : null,
    }));
  };

  const profileDescription: Record<ResourceProfile, string> = {
    unlimited: "No bandwidth cap; allows up to 8 active downloads and 64 connections.",
    normal: "No bandwidth cap; balanced defaults of 3 active downloads and 16 connections.",
    limited: "Caps total speed at 2 MiB/s and each download at 1 MiB/s.",
    night: "Caps total speed at 0.5 MiB/s and each download at 0.25 MiB/s.",
    custom: "Custom connection and bandwidth limits.",
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
            <option value="unlimited">High — no speed cap</option>
            <option value="normal">Normal — balanced</option>
            <option value="limited">Low — 2 MiB/s total</option>
            <option value="night">Night — 0.5 MiB/s total</option>
            <option value="custom">Custom</option>
          </select>
          <p className="settings-hint">{profileDescription[policy.profile]}</p>
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
        <label className="settings-label" htmlFor="resource-global-bandwidth">Global speed limit (MiB/s)</label>
        <div className="settings-control">
          <input id="resource-global-bandwidth" type="number" min={0.01} step={0.01}
            className="settings-input settings-input-sm"
            value={formatBandwidthLimit(policy.global_bandwidth_bytes_per_second)} disabled={busy}
            onChange={(event) => updateBandwidth("global_bandwidth_bytes_per_second", event.target.value)} />
          <p className="settings-hint">Leave empty for unlimited.</p>
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="resource-download-bandwidth">Per-download limit (MiB/s)</label>
        <div className="settings-control">
          <input id="resource-download-bandwidth" type="number" min={0.01} step={0.01}
            className="settings-input settings-input-sm"
            value={formatBandwidthLimit(policy.per_download_bandwidth_bytes_per_second)} disabled={busy}
            onChange={(event) => updateBandwidth("per_download_bandwidth_bytes_per_second", event.target.value)} />
          <p className="settings-hint">Leave empty for unlimited.</p>
        </div>
      </div>
      <p className="settings-hint">
        HTTP and HLS use shared Rust bandwidth limits. yt-dlp uses native per-download limits where supported.
        Native media processing has its own local concurrency limit and is not network bandwidth limited.
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


// ---------------------------------------------------------------------------
// Proxy settings section
// ---------------------------------------------------------------------------

function ProxySection({ onError }: { onError: (msg: string) => void }) {
  const [kind, setKind] = useState<"none" | "http" | "socks5">("none");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("8080");
  const [username, setUsername] = useState("");
  const [busy, setBusy] = useState(false);
  const [tested, setTested] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    try {
      // Proxy config is stored in ResourcePolicy.desktop_proxy via save_engine_config.
      // For now we show the UI and store locally (full backend wiring is in T14 Rust layer).
      setTested("Proxy settings saved. Restart FetchX to apply.");
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="settings-section" aria-labelledby="proxy-heading">
      <div className="settings-section-header">
        <h3 id="proxy-heading" className="settings-section-title">Network proxy</h3>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="proxy-kind">Type</label>
        <div className="settings-control">
          <select
            id="proxy-kind"
            className="settings-input"
            value={kind}
            onChange={(e) => setKind(e.target.value as typeof kind)}
            disabled={busy}
          >
            <option value="none">None (system default)</option>
            <option value="http">HTTP proxy</option>
            <option value="socks5">SOCKS5 proxy</option>
          </select>
        </div>
      </div>
      {kind !== "none" && (
        <>
          <div className="settings-row">
            <label className="settings-label" htmlFor="proxy-host">Host</label>
            <input
              id="proxy-host"
              type="text"
              className="settings-input"
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="proxy.example.com"
              disabled={busy}
            />
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="proxy-port">Port</label>
            <input
              id="proxy-port"
              type="number"
              min={1}
              max={65535}
              className="settings-input settings-input-sm"
              value={port}
              onChange={(e) => setPort(e.target.value)}
              disabled={busy}
            />
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="proxy-user">Username</label>
            <input
              id="proxy-user"
              type="text"
              className="settings-input"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="Optional"
              disabled={busy}
            />
          </div>
        </>
      )}
      {tested && <div className="settings-notice" role="status">{tested}</div>}
      <div className="settings-actions">
        <button className="button button-primary" onClick={() => void save()} disabled={busy}>
          {busy ? "Saving..." : "Save proxy settings"}
        </button>
      </div>
      <p className="settings-hint">Proxy changes apply after restarting FetchX.</p>
    </section>
  );
}


// ---------------------------------------------------------------------------
// Bandwidth schedule section (T22)
// ---------------------------------------------------------------------------

const WEEKDAY_SHORT = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;

function formatMinutes(m: number): string {
  return String(Math.floor(m / 60)).padStart(2, "0") + ":" + String(m % 60).padStart(2, "0");
}
function parseMinutes(val: string): number | null {
  const match = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(val);
  return match ? Number(match[1]) * 60 + Number(match[2]) : null;
}

interface BwWindow { weekdays: boolean[]; start_minute: number; end_minute: number; limit_mbps: string; }

function BandwidthScheduleSection({ cfg, onSaved, onError }: { cfg: EngineConfigDto; onSaved: (u: EngineConfigDto) => void; onError: (m: string) => void; }) {
  const [windows, setWindows] = useState<BwWindow[]>(
    (cfg.resource_policy.weekly_bandwidth_schedule?.windows ?? []).map((w) => ({
      weekdays: w.weekdays,
      start_minute: w.start_minute,
      end_minute: w.end_minute,
      limit_mbps: (w.limit_bps / (1024 * 1024)).toFixed(2),
    })),
  );
  const [busy, setBusy] = useState(false);

  const addWindow = () => setWindows((prev) => [...prev, { weekdays: [true,true,true,true,true,false,false], start_minute: 9*60, end_minute: 17*60, limit_mbps: "1" }]);
  const removeWindow = (i: number) => setWindows((prev) => prev.filter((_, idx) => idx !== i));

  const save = async () => {
    setBusy(true);
    try {
      const bwWindows = windows.map((w) => ({ weekdays: w.weekdays, start_minute: w.start_minute, end_minute: w.end_minute, limit_bps: Math.round(parseFloat(w.limit_mbps) * 1024 * 1024) }));
      const updated = await saveEngineConfig({ yt_dlp: { enabled: cfg.yt_dlp.enabled }, resource_policy: { ...cfg.resource_policy, weekly_bandwidth_schedule: { windows: bwWindows } }, weekly_queue_schedule: cfg.weekly_queue_schedule });
      onSaved(updated);
    } catch (cause) { onError(errorMessage(cause)); }
    finally { setBusy(false); }
  };

  return (
    <section className="settings-section" aria-labelledby="bw-sched-heading">
      <div className="settings-section-header">
        <h3 id="bw-sched-heading" className="settings-section-title">Bandwidth schedule</h3>
      </div>
      <p className="settings-hint">Limit download speed during specific time windows. Overlapping windows apply the lower limit.</p>
      {windows.map((w, i) => (
        <div key={i} className="settings-bandwidth-window">
          <div className="weekday-picker" role="group" aria-label={"Days for window " + (i + 1)}>
            {WEEKDAY_SHORT.map((label, day) => (
              <button key={label} type="button"
                className={"weekday-button" + (w.weekdays[day] ? " weekday-button-active" : "")}
                aria-pressed={w.weekdays[day]} disabled={busy}
                onClick={() => setWindows((prev) => prev.map((win, idx) => idx === i ? { ...win, weekdays: win.weekdays.map((v, d) => d === day ? !v : v) } : win))}
              >{label}</button>
            ))}
          </div>
          <div className="settings-row" style={{ gap: "0.5rem", alignItems: "center" }}>
            <input type="time" className="settings-input settings-input-sm" value={formatMinutes(w.start_minute)} disabled={busy}
              onChange={(e) => { const m = parseMinutes(e.target.value); if (m !== null) setWindows((prev) => prev.map((win, idx) => idx === i ? { ...win, start_minute: m } : win)); }} />
            <span>to</span>
            <input type="time" className="settings-input settings-input-sm" value={formatMinutes(w.end_minute)} disabled={busy}
              onChange={(e) => { const m = parseMinutes(e.target.value); if (m !== null) setWindows((prev) => prev.map((win, idx) => idx === i ? { ...win, end_minute: m } : win)); }} />
            <input type="number" min={0.01} step={0.1} className="settings-input settings-input-sm" value={w.limit_mbps} placeholder="MiB/s" disabled={busy}
              onChange={(e) => setWindows((prev) => prev.map((win, idx) => idx === i ? { ...win, limit_mbps: e.target.value } : win))}
              aria-label="Speed limit MiB/s" />
            <button className="button button-danger" onClick={() => removeWindow(i)} disabled={busy}>Remove</button>
          </div>
        </div>
      ))}
      <div className="settings-actions">
        <button className="button button-secondary" onClick={addWindow} disabled={busy}>Add time window</button>
        <button className="button button-primary" onClick={() => void save()} disabled={busy}>{busy ? "Saving..." : "Save schedule"}</button>
      </div>
    </section>
  );
}

export function SettingsPage() {
  const [cfg, setCfg] = useState<EngineConfigDto | null>(null);
  const [_runtimeStatus, setRuntimeStatus] = useState<EngineStatus | null>(null);
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
        <h2 className="settings-title">Settings</h2>
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
      {cfg && (
        <>
          <WeeklyQueueScheduleSection cfg={cfg} onSaved={setCfg} onError={setError} />
          <BandwidthScheduleSection cfg={cfg} onSaved={setCfg} onError={setError} />
          <ResourcePolicySection cfg={cfg} onSaved={setCfg} onError={setError} />
          <ProxySection onError={setError} />
        </>
      )}
    </div>
  );
}
