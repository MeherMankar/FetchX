import { useCallback, useEffect, useState } from "react";
import {
  getBrowserExtensionStatus,
  launchBrowserWithExtension,
  openBrowserExtensionInstallPage,
} from "../api/browserExtensions";
import {
  getEngineConfig,
  getEngineStatus,
  getFolderCategoriesConfig,
  getPostDownloadAction,
  exportSettings,
  importSettings,
  resetEngineConfig,
  saveEngineConfig,
  setFolderCategoriesConfig,
  setPostDownloadAction,
} from "../api/engine";
import type {
  BrowserExtensionStatus,
  CategoryRuleDto,
  DesktopProxyConfigDto,
  EngineConfigDto,
  EngineStatus,
  FolderCategoriesConfigDto,
  NativeBuiltinsConfigDto,
  PostDownloadActionDto,
  PostDownloadActionKind,
  ResourcePolicyDto,
  ResourceProfile,
  WeeklyQueueScheduleDto,
} from "../api/types";
import { errorMessage } from "../lib/format";

// ---------------------------------------------------------------------------
// Status indicator
// ---------------------------------------------------------------------------

function NativeBuiltinsSection({
  cfg,
  onSaved,
  onError,
}: {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (message: string) => void;
}) {
  const [settings, setSettings] = useState<NativeBuiltinsConfigDto>(cfg.native_builtins);
  const [busy, setBusy] = useState(false);

  useEffect(() => setSettings(cfg.native_builtins), [cfg.native_builtins]);

  const save = async () => {
    setBusy(true);
    try {
      const updated = await saveEngineConfig({
        yt_dlp: { enabled: cfg.yt_dlp.enabled },
        resource_policy: cfg.resource_policy,
        weekly_queue_schedule: cfg.weekly_queue_schedule,
        native_builtins: settings,
      });
      onSaved(updated);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const updateEnabled = (
    key: "page_scraper_enabled" | "url_resolver_enabled",
    enabled: boolean,
  ) => setSettings((current) => ({ ...current, [key]: enabled }));

  return (
    <section className="settings-section" aria-labelledby="native-builtins-heading">
      <div className="settings-section-header">
        <h3 id="native-builtins-heading" className="settings-section-title">
          Native built-ins
        </h3>
      </div>
      <div className="settings-row">
        <span className="settings-label">URL resolver</span>
        <div className="settings-control">
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={settings.url_resolver_enabled}
              disabled={busy}
              onChange={(event) => updateEnabled("url_resolver_enabled", event.target.checked)}
            />
            <span className="toggle-track"><span className="toggle-thumb" /></span>
            <span>{settings.url_resolver_enabled ? "Enabled" : "Disabled"}</span>
          </label>
          <p className="settings-hint">Resolve supported media pages, including Wikimedia Commons file pages.</p>
        </div>
      </div>
      <div className="settings-row">
        <span className="settings-label">Page scraper</span>
        <div className="settings-control">
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={settings.page_scraper_enabled}
              disabled={busy}
              onChange={(event) => updateEnabled("page_scraper_enabled", event.target.checked)}
            />
            <span className="toggle-track"><span className="toggle-thumb" /></span>
            <span>{settings.page_scraper_enabled ? "Enabled" : "Disabled"}</span>
          </label>
          <p className="settings-hint">Look for a direct archive or installer link on submitted HTML pages.</p>
        </div>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="filename-template">Filename template</label>
        <div className="settings-control">
          <input
            id="filename-template"
            className="settings-input"
            value={settings.filename_template}
            maxLength={512}
            disabled={busy}
            onChange={(event) => setSettings((current) => ({
              ...current,
              filename_template: event.target.value,
            }))}
          />
          <p className="settings-hint">
            Supports {"{{title}}"}, {"{{filename}}"}, {"{{extension}}"}, {"{{taskId}}"},
            {" {{host}}"}, {"{{domain}}"}, and {"{{path}}"}.
          </p>
        </div>
      </div>
      <div className="settings-actions">
        <button className="button button-primary" onClick={() => void save()} disabled={busy}>
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </section>
  );
}

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
      setMessage("Opened the Firefox Add-ons page for " + browser + ". Click 'Add to Firefox' to install.");
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
              <span className="settings-hint">
                FetchX is available on the Firefox Add-ons store. Click "Install" to open the listing and add it in one click.
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
                <span
                  className={`status-dot ${browser.native_host_registered ? "dot-ok" : "dot-idle"}`}
                  aria-hidden="true"
                />
                <span>{browser.browser}</span>
                <span className="settings-hint">
                  {browser.native_host_registered ? "Native host ready" : "Not registered"}
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
      desktop_proxy: current.desktop_proxy,
    },
    normal: {
      profile,
      max_active_downloads: 3,
      global_connection_limit: 16,
      per_download_connection_limit: 8,
      global_bandwidth_bytes_per_second: null,
      per_download_bandwidth_bytes_per_second: null,
      engine_bandwidth_limits: engineLimits,
      desktop_proxy: current.desktop_proxy,
    },
    limited: {
      profile,
      max_active_downloads: 2,
      global_connection_limit: 8,
      per_download_connection_limit: 4,
      global_bandwidth_bytes_per_second: 2 * 1024 * 1024,
      per_download_bandwidth_bytes_per_second: 1024 * 1024,
      engine_bandwidth_limits: engineLimits,
      desktop_proxy: current.desktop_proxy,
    },
    night: {
      profile,
      max_active_downloads: 2,
      global_connection_limit: 4,
      per_download_connection_limit: 2,
      global_bandwidth_bytes_per_second: 512 * 1024,
      per_download_bandwidth_bytes_per_second: 256 * 1024,
      engine_bandwidth_limits: engineLimits,
      desktop_proxy: current.desktop_proxy,
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
        <span className="settings-label">Weekly download window</span>
        <div className="settings-control">
          <label className="toggle-switch">
            <input
              id="weekly-schedule-enabled"
              type="checkbox"
              checked={schedule.enabled}
              disabled={busy}
              onChange={(event) =>
                setSchedule((current) => ({ ...current, enabled: event.target.checked }))
              }
            />
            <span className="toggle-track">
              <span className="toggle-thumb" />
            </span>
            <span style={{ fontSize: "13px", color: "var(--text-2)" }}>
              {schedule.enabled ? "Enabled" : "Disabled"}
            </span>
          </label>
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

function ProxySection({
  cfg,
  onSaved,
  onError,
}: {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (msg: string) => void;
}) {
  const [kind, setKind] = useState<"none" | DesktopProxyConfigDto["kind"]>(
    cfg.resource_policy.desktop_proxy?.kind ?? "none",
  );
  const [host, setHost] = useState(cfg.resource_policy.desktop_proxy?.host ?? "");
  const [port, setPort] = useState(String(cfg.resource_policy.desktop_proxy?.port ?? 8080));
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    const proxy = cfg.resource_policy.desktop_proxy;
    setKind(proxy?.kind ?? "none");
    setHost(proxy?.host ?? "");
    setPort(String(proxy?.port ?? 8080));
  }, [cfg.resource_policy.desktop_proxy]);

  const save = async () => {
    const parsedPort = Number.parseInt(port, 10);
    if (
      kind !== "none"
      && (!host.trim() || !Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535)
    ) {
      onError("Enter a proxy host and a port from 1 to 65535.");
      return;
    }

    setBusy(true);
    try {
      const desktopProxy: DesktopProxyConfigDto | null = kind === "none"
        ? null
        : {
          kind,
          host: host.trim(),
          port: parsedPort,
          username: cfg.resource_policy.desktop_proxy?.username ?? null,
        };
      const updated = await saveEngineConfig({
        yt_dlp: { enabled: cfg.yt_dlp.enabled },
        resource_policy: {
          ...cfg.resource_policy,
          desktop_proxy: desktopProxy,
        },
        weekly_queue_schedule: cfg.weekly_queue_schedule,
      });
      onSaved(updated);
      setSaved("Proxy settings saved. Restart FetchX to apply.");
    } catch (cause) {
      onError(errorMessage(cause));
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
            <option value="none">No proxy</option>
            <option value="http">HTTP proxy</option>
            <option value="https">HTTPS proxy</option>
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
        </>
      )}
      {saved && <div className="settings-notice" role="status">{saved}</div>}
      <div className="settings-actions">
        <button className="button button-primary" onClick={() => void save()} disabled={busy}>
          {busy ? "Saving..." : "Save proxy settings"}
        </button>
      </div>
      <p className="settings-hint">
        Applies to Direct HTTP and HLS downloads after restart. Proxy authentication
        is not supported yet.
      </p>
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

// ---------------------------------------------------------------------------
// After-download action section
// ---------------------------------------------------------------------------

const ACTION_LABELS: Record<PostDownloadActionKind, string> = {
  nothing: "Do nothing",
  open_file: "Open file",
  open_folder: "Open containing folder",
  run_command: "Run command",
};

function AfterDownloadSection({ onError }: { onError: (msg: string) => void }) {
  const [action, setAction] = useState<PostDownloadActionDto>({ kind: "nothing" });
  const [commandText, setCommandText] = useState("");
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    getPostDownloadAction()
      .then((dto) => {
        setAction(dto);
        if (dto.kind === "run_command") {
          setCommandText(dto.command ?? "");
        }
      })
      .catch((cause) => onError(errorMessage(cause)));
  }, [onError]);

  const save = async () => {
    setBusy(true);
    setSaved(false);
    try {
      const payload: PostDownloadActionDto =
        action.kind === "run_command"
          ? { kind: "run_command", command: commandText }
          : { kind: action.kind };
      await setPostDownloadAction(payload);
      setAction(payload);
      setSaved(true);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="settings-section" aria-labelledby="after-download-heading">
      <div className="settings-section-header">
        <h3 id="after-download-heading" className="settings-section-title">
          After download
        </h3>
      </div>
      <div className="settings-row">
        <label className="settings-label" htmlFor="after-download-action">
          Action
        </label>
        <div className="settings-control">
          <select
            id="after-download-action"
            className="settings-input"
            value={action.kind}
            disabled={busy}
            onChange={(event) => {
              const kind = event.target.value as PostDownloadActionKind;
              setAction({ kind });
              setSaved(false);
            }}
          >
            {(Object.keys(ACTION_LABELS) as PostDownloadActionKind[]).map((k) => (
              <option key={k} value={k}>
                {ACTION_LABELS[k]}
              </option>
            ))}
          </select>
          <p className="settings-hint">
            Runs automatically each time a download completes.
          </p>
        </div>
      </div>
      {action.kind === "run_command" && (
        <div className="settings-row">
          <label className="settings-label" htmlFor="after-download-command">
            Command
          </label>
          <div className="settings-control">
            <input
              id="after-download-command"
              type="text"
              className="settings-input"
              value={commandText}
              disabled={busy}
              placeholder={'e.g. notify-send "Done: %FETCHX_FILE%"'}
              onChange={(event) => {
                setCommandText(event.target.value);
                setSaved(false);
              }}
            />
            <p className="settings-hint">
              The environment variable <code>FETCHX_FILE</code> is set to the
              downloaded file path. On Windows use <code>%FETCHX_FILE%</code>,
              on macOS/Linux use <code>$FETCHX_FILE</code>.
            </p>
          </div>
        </div>
      )}
      {saved && (
        <div className="settings-notice" role="status">
          After-download action saved.
        </div>
      )}
      <div className="settings-actions">
        <button
          className="button button-primary"
          onClick={() => void save()}
          disabled={busy || (action.kind === "run_command" && commandText.trim() === "")}
        >
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Folder categories section
// ---------------------------------------------------------------------------

const CATEGORY_KEYS: Array<keyof Omit<FolderCategoriesConfigDto, "enabled" | "custom">> = [
  "compressed",
  "documents",
  "music",
  "programs",
  "video",
  "images",
];

const CATEGORY_LABELS: Record<string, string> = {
  compressed: "Compressed",
  documents: "Documents",
  music: "Music",
  programs: "Programs",
  video: "Video",
  images: "Images",
};

function FolderCategoriesSection({ onError }: { onError: (msg: string) => void }) {
  const [cfg, setCfg] = useState<FolderCategoriesConfigDto | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    getFolderCategoriesConfig()
      .then(setCfg)
      .catch((cause) => onError(errorMessage(cause)));
  }, [onError]);

  const updateRule = (
    key: keyof Omit<FolderCategoriesConfigDto, "enabled" | "custom">,
    field: keyof CategoryRuleDto,
    value: string,
  ) => {
    setCfg((current) => {
      if (!current) return current;
      return {
        ...current,
        [key]: { ...current[key], [field]: value },
      };
    });
    setSaved(false);
  };

  const save = async () => {
    if (!cfg) return;
    setBusy(true);
    setSaved(false);
    try {
      await setFolderCategoriesConfig(cfg);
      setSaved(true);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  if (!cfg) return null;

  return (
    <section className="settings-section" aria-labelledby="folder-categories-heading">
      <div className="settings-section-header">
        <h3 id="folder-categories-heading" className="settings-section-title">
          Folder categories
        </h3>
      </div>
      <div className="settings-row">
        <span className="settings-label">Auto-categorise downloads</span>
        <div className="settings-control">
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={cfg.enabled}
              disabled={busy}
              onChange={(event) => {
                setCfg((current) => current ? { ...current, enabled: event.target.checked } : current);
                setSaved(false);
              }}
            />
            <span className="toggle-track">
              <span className="toggle-thumb" />
            </span>
            <span style={{ fontSize: "13px", color: "var(--text-2)" }}>
              {cfg.enabled ? "Enabled" : "Disabled"}
            </span>
          </label>
          <p className="settings-hint">
            Automatically move completed downloads into sub-folders by file type.
          </p>
        </div>
      </div>

      {cfg.enabled && (
        <>
          <table className="settings-category-table" style={{ width: "100%", borderCollapse: "collapse", marginBottom: "0.75rem" }}>
            <thead>
              <tr>
                <th style={{ textAlign: "left", padding: "4px 8px", fontWeight: 600, fontSize: "13px" }}>Category</th>
                <th style={{ textAlign: "left", padding: "4px 8px", fontWeight: 600, fontSize: "13px" }}>Folder</th>
                <th style={{ textAlign: "left", padding: "4px 8px", fontWeight: 600, fontSize: "13px" }}>Extensions (comma-separated)</th>
              </tr>
            </thead>
            <tbody>
              {CATEGORY_KEYS.map((key) => (
                <tr key={key}>
                  <td style={{ padding: "4px 8px", fontSize: "13px", whiteSpace: "nowrap" }}>
                    {CATEGORY_LABELS[key]}
                  </td>
                  <td style={{ padding: "4px 8px" }}>
                    <input
                      type="text"
                      className="settings-input"
                      value={cfg[key].folder}
                      disabled={busy}
                      onChange={(event) => updateRule(key, "folder", event.target.value)}
                      aria-label={`${CATEGORY_LABELS[key]} folder`}
                    />
                  </td>
                  <td style={{ padding: "4px 8px" }}>
                    <input
                      type="text"
                      className="settings-input"
                      value={cfg[key].extensions}
                      disabled={busy}
                      onChange={(event) => updateRule(key, "extensions", event.target.value)}
                      aria-label={`${CATEGORY_LABELS[key]} extensions`}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="settings-row">
            <label className="settings-label" htmlFor="folder-categories-custom">
              Custom rules
            </label>
            <div className="settings-control">
              <textarea
                id="folder-categories-custom"
                className="settings-input"
                rows={3}
                value={cfg.custom}
                disabled={busy}
                placeholder="Books=epub,mobi;3D=stl,obj"
                onChange={(event) => {
                  setCfg((current) => current ? { ...current, custom: event.target.value } : current);
                  setSaved(false);
                }}
              />
              <p className="settings-hint">
                Format: <code>Folder=ext,ext;Folder2=ext</code>. Custom rules override the built-in categories above.
              </p>
            </div>
          </div>
        </>
      )}

      {saved && (
        <div className="settings-notice" role="status">
          Folder categories saved.
        </div>
      )}
      <div className="settings-actions">
        <button
          className="button button-primary"
          onClick={() => void save()}
          disabled={busy}
        >
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Auto-retry section (Feature 5)
// ---------------------------------------------------------------------------

function AutoRetrySection({
  cfg,
  onSaved,
  onError,
}: {
  cfg: EngineConfigDto;
  onSaved: (updated: EngineConfigDto) => void;
  onError: (message: string) => void;
}) {
  const [count, setCount] = useState(cfg.auto_retry_count ?? 0);
  const [delay, setDelay] = useState(cfg.auto_retry_delay_seconds ?? 60);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setCount(cfg.auto_retry_count ?? 0);
    setDelay(cfg.auto_retry_delay_seconds ?? 60);
  }, [cfg.auto_retry_count, cfg.auto_retry_delay_seconds]);

  const enabled = count > 0;

  const save = async (nextCount: number, nextDelay: number) => {
    setBusy(true);
    try {
      const updated = await saveEngineConfig({
        yt_dlp: { enabled: cfg.yt_dlp.enabled },
        resource_policy: cfg.resource_policy,
        weekly_queue_schedule: cfg.weekly_queue_schedule,
        native_builtins: cfg.native_builtins,
        auto_retry_count: nextCount,
        auto_retry_delay_seconds: nextDelay,
      });
      onSaved(updated);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const toggleEnabled = (next: boolean) => {
    const nextCount = next ? Math.max(count, 3) : 0;
    setCount(nextCount);
    void save(nextCount, delay);
  };

  const handleSave = () => void save(count, delay);

  return (
    <section className="settings-section" aria-labelledby="auto-retry-heading">
      <div className="settings-section-header">
        <h3 id="auto-retry-heading" className="settings-section-title">Auto-retry</h3>
      </div>
      <div className="settings-row">
        <span className="settings-label">Auto-retry failed downloads</span>
        <div className="settings-control">
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={enabled}
              disabled={busy}
              onChange={(event) => toggleEnabled(event.target.checked)}
            />
            <span className="toggle-track"><span className="toggle-thumb" /></span>
            <span style={{ fontSize: "13px", color: "var(--text-2)" }}>
              {enabled ? "Enabled" : "Disabled"}
            </span>
          </label>
        </div>
      </div>
      {enabled && (
        <>
          <div className="settings-row">
            <label className="settings-label" htmlFor="auto-retry-count">Max retries</label>
            <div className="settings-control">
              <input
                id="auto-retry-count"
                type="number"
                min={1}
                max={10}
                className="settings-input settings-input-sm"
                value={count}
                disabled={busy}
                onChange={(event) => setCount(Number(event.target.value))}
              />
            </div>
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="auto-retry-delay">Retry delay (seconds)</label>
            <div className="settings-control">
              <input
                id="auto-retry-delay"
                type="number"
                min={5}
                max={300}
                className="settings-input settings-input-sm"
                value={delay}
                disabled={busy}
                onChange={(event) => setDelay(Number(event.target.value))}
              />
            </div>
          </div>
          <div className="settings-actions">
            <button className="button button-primary" onClick={handleSave} disabled={busy}>
              {busy ? "Saving…" : "Save"}
            </button>
          </div>
        </>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Export / Import section
// ---------------------------------------------------------------------------

function ExportImportSection({
  onImported,
  onError,
}: {
  onImported: () => void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState<null | "export" | "import">(null);
  const [notice, setNotice] = useState<string | null>(null);

  const handleExport = async () => {
    setBusy("export");
    setNotice(null);
    try {
      await exportSettings();
      setNotice("Settings exported to the chosen file.");
    } catch (cause) {
      const message = errorMessage(cause);
      if (/cancelled/i.test(message)) {
        setNotice(message);
      } else {
        onError(message);
      }
    } finally {
      setBusy(null);
    }
  };

  const handleImport = async () => {
    setBusy("import");
    setNotice(null);
    try {
      await importSettings();
      setNotice("Settings imported. Reloading…");
      onImported();
    } catch (cause) {
      const message = errorMessage(cause);
      if (/cancelled/i.test(message)) {
        setNotice(message);
      } else {
        onError(message);
      }
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="settings-section" aria-labelledby="export-import-heading">
      <div className="settings-section-header">
        <h3 id="export-import-heading" className="settings-section-title">
          Export / Import
        </h3>
      </div>
      <p className="settings-hint">
        Back up your FetchX settings to a JSON file, or restore them from a
        previous export. Importing replaces your current settings and reloads
        this page.
      </p>
      {notice && (
        <div className="settings-notice" role="status">
          {notice}
        </div>
      )}
      <div className="settings-actions">
        <button
          className="button button-secondary"
          onClick={() => void handleExport()}
          disabled={busy !== null}
        >
          {busy === "export" ? "Exporting…" : "Export settings"}
        </button>
        <button
          className="button button-primary"
          onClick={() => void handleImport()}
          disabled={busy !== null}
        >
          {busy === "import" ? "Importing…" : "Import settings"}
        </button>
      </div>
    </section>
  );
}

export function SettingsPage() {  const [cfg, setCfg] = useState<EngineConfigDto | null>(null);
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

      <ExportImportSection onImported={() => void load()} onError={setError} />
      <BrowserExtensionSection />
      {cfg && (
        <>
          <NativeBuiltinsSection cfg={cfg} onSaved={setCfg} onError={setError} />
          <WeeklyQueueScheduleSection cfg={cfg} onSaved={setCfg} onError={setError} />
          <BandwidthScheduleSection cfg={cfg} onSaved={setCfg} onError={setError} />
          <ResourcePolicySection cfg={cfg} onSaved={setCfg} onError={setError} />
          <AutoRetrySection cfg={cfg} onSaved={setCfg} onError={setError} />
          <AfterDownloadSection onError={setError} />
          <FolderCategoriesSection onError={setError} />
          <ProxySection cfg={cfg} onSaved={setCfg} onError={setError} />
        </>
      )}
    </div>
  );
}
