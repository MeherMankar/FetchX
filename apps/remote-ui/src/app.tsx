import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";

import {
  RemoteClient,
  RemoteError,
  protocolVersion,
  type DownloadPriority,
  type DownloadStatus,
  type EngineRuntimeStatusEntry,
  type HealthInfo,
  type ProtocolInfo,
  type RemoteDownload,
  type RemoteEvent,
} from "./protocol";
import { TorrentSelectionPanel } from "./TorrentSelectionPanel";

type Section =
  | "dashboard"
  | "downloads"
  | "queue"
  | "engines"
  | "scheduler"
  | "settings"
  | "devices"
  | "plugins"
  | "system";

type TorrentEngineChoice = "automatic" | "aria2" | "native_torrent";

const navigation: Array<{ id: Section; label: string; icon: string }> = [
  { id: "dashboard", label: "Dashboard", icon: "⌂" },
  { id: "downloads", label: "Downloads", icon: "↓" },
  { id: "queue", label: "Queue", icon: "≡" },
  { id: "engines", label: "Engines", icon: "◈" },
  { id: "scheduler", label: "Scheduler", icon: "◷" },
  { id: "settings", label: "Settings", icon: "⚙" },
  { id: "devices", label: "Devices", icon: "⌘" },
  { id: "plugins", label: "Plugins", icon: "▦" },
  { id: "system", label: "System / Health", icon: "◇" },
];

const unavailable: Record<Exclude<Section, "dashboard" | "downloads" | "queue" | "system">, {
  title: string;
  detail: string;
}> = {
  engines: {
    title: "Engine runtime status is not available in the current protocol.",
    detail: "This server does not advertise engine.runtimeStatus.",
  },
  scheduler: {
    title: "Scheduler controls are not available in the current protocol.",
    detail: "No remote scheduler listing or scheduling RPC is advertised by this server.",
  },
  settings: {
    title: "Remote settings are not available in the current protocol.",
    detail: "The current server does not advertise settings read or write operations.",
  },
  devices: {
    title: "Device management is not available in the current protocol.",
    detail: "Phase 21C provides a primary bearer token; device pairing and session management are not exposed.",
  },
  plugins: {
    title: "Remote plugin management is not available in the current protocol.",
    detail: "No remote plugin listing or management operation is advertised by this server.",
  },
};

interface DownloadListResult {
  downloads: RemoteDownload[];
}

interface EngineRuntimeStatusResponse {
  engines: EngineRuntimeStatusEntry[];
}

interface CreateResult {
  id: string;
}

export function App() {
  const [serverUrl, setServerUrl] = useState("http://127.0.0.1:39800");
  const [tokenInput, setTokenInput] = useState("");
  const [client, setClient] = useState<RemoteClient | null>(null);
  const [health, setHealth] = useState<HealthInfo | null>(null);
  const [protocol, setProtocol] = useState<ProtocolInfo | null>(null);
  const [downloads, setDownloads] = useState<RemoteDownload[]>([]);
  const [engineStatuses, setEngineStatuses] = useState<EngineRuntimeStatusEntry[] | null>(null);
  const [section, setSection] = useState<Section>("dashboard");
  const [search, setSearch] = useState("");
  const [newUrl, setNewUrl] = useState("");
  const [priority, setPriority] = useState<DownloadPriority>("normal");
  const [torrentEngine, setTorrentEngine] = useState<TorrentEngineChoice>("automatic");
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [eventState, setEventState] = useState("Disconnected");
  const [lastEvent, setLastEvent] = useState("");
  const [connecting, setConnecting] = useState(false);
  const clientRef = useRef<RemoteClient | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const lastRefreshAt = useRef(0);

  const disconnect = useCallback((clearMessage = false) => {
    socketRef.current?.close();
    socketRef.current = null;
    clientRef.current?.clear();
    clientRef.current = null;
    setClient(null);
    setHealth(null);
    setProtocol(null);
    setDownloads([]);
    setEngineStatuses(null);
    setEventState("Disconnected");
    setTokenInput("");
    if (clearMessage) setError("");
  }, []);

  const refreshDownloads = useCallback(async (activeClient: RemoteClient) => {
    const result = await activeClient.call<DownloadListResult>("download.list");
    if (clientRef.current === activeClient && Array.isArray(result.downloads)) {
      setDownloads(result.downloads);
    }
  }, []);

  const refreshEngineStatuses = useCallback(async (
    activeClient: RemoteClient,
    currentCapabilities: string[],
  ) => {
    if (!currentCapabilities.includes("engine.runtimeStatus")) {
      setEngineStatuses(null);
      return;
    }
    const result = await activeClient.call<EngineRuntimeStatusResponse>("engine.runtimeStatus");
    if (clientRef.current === activeClient && Array.isArray(result.engines)) {
      setEngineStatuses(result.engines);
    }
  }, []);

  const handleError = useCallback((cause: unknown) => {
    setError(cause instanceof Error ? cause.message : "The request failed.");
  }, []);

  const scheduleRefresh = useCallback((activeClient: RemoteClient, event: RemoteEvent) => {
    setLastEvent(event.method);
    const now = Date.now();
    if (now - lastRefreshAt.current < 500) return;
    lastRefreshAt.current = now;
    void refreshDownloads(activeClient).catch(handleError);
  }, [handleError, refreshDownloads]);

  const connect = useCallback(async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (connecting) return;
    setConnecting(true);
    setError("");
    setNotice("");

    let candidate: RemoteClient | null = null;
    try {
      const activeClient = new RemoteClient(serverUrl, tokenInput, () => {
        setError("Authentication expired or was rejected. Enter the current token to reconnect.");
        disconnect();
      });
      candidate = activeClient;
      const currentHealth = await activeClient.health();
      if (currentHealth.status !== "ok" || currentHealth.protocol_version !== protocolVersion) {
        throw new RemoteError("The server health response is incompatible with FetchX protocol v1.", "protocol");
      }
      const currentProtocol = await activeClient.call<ProtocolInfo>("system.discover");
      if (currentProtocol.protocol_version !== protocolVersion) {
        throw new RemoteError(
          `Unsupported protocol version: ${currentProtocol.protocol_version || "unknown"}.`,
          "protocol",
        );
      }
      const canList = currentProtocol.capabilities.includes("download.list");
      const rows = canList
        ? await activeClient.call<DownloadListResult>("download.list")
        : { downloads: [] };
      const currentEngineStatuses = currentProtocol.capabilities.includes("engine.runtimeStatus")
        ? await activeClient.call<EngineRuntimeStatusResponse>("engine.runtimeStatus")
        : null;
      setHealth(currentHealth);
      setProtocol(currentProtocol);
      setDownloads(rows.downloads);
      setEngineStatuses(currentEngineStatuses?.engines ?? null);
      clientRef.current = activeClient;
      setClient(activeClient);
      setTokenInput("");
      setSection("dashboard");
      setNotice("");

      if (currentProtocol.capabilities.includes("system.websocketTicket")) {
        setEventState("Connecting");
        void activeClient.openEvents(
          (remoteEvent) => scheduleRefresh(activeClient, remoteEvent),
          () => {
            if (clientRef.current !== activeClient) return;
            setEventState("Disconnected");
            void activeClient.call<ProtocolInfo>("system.discover").catch((cause: unknown) => {
              if (cause instanceof RemoteError && cause.kind === "authentication") {
                setError(cause.message);
              }
            });
          },
        ).then((socket) => {
          if (clientRef.current !== activeClient) {
            socket.close();
            return;
          }
          socketRef.current = socket;
          setEventState("Live");
        }).catch((cause: unknown) => {
          if (clientRef.current === activeClient) {
            setEventState("Unavailable");
            if (cause instanceof RemoteError && cause.kind === "forbidden") {
              setNotice("Connected, but this account is not permitted to read event notifications.");
            } else {
              setNotice("Connected. Live events are unavailable; downloads will refresh periodically.");
            }
          }
        });
      } else {
        setEventState("Unavailable");
      }
    } catch (cause) {
      if (cause instanceof RemoteError && cause.kind === "authentication") {
        setTokenInput("");
      }
      handleError(cause);
      candidate?.clear();
    } finally {
      setConnecting(false);
    }
  }, [connecting, disconnect, scheduleRefresh, serverUrl, tokenInput]);

  useEffect(() => {
    if (!client) return;
    const timer = window.setInterval(() => {
      void Promise.all([
        refreshDownloads(client),
        refreshEngineStatuses(client, protocol?.capabilities ?? []),
      ]).catch((cause: unknown) => {
        if (cause instanceof RemoteError && cause.kind === "authentication") return;
        handleError(cause);
      });
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [client, handleError, protocol?.capabilities, refreshDownloads, refreshEngineStatuses]);

  useEffect(() => () => {
    socketRef.current?.close();
    clientRef.current?.clear();
  }, []);

  const capabilities = useMemo(() => new Set(protocol?.capabilities ?? []), [protocol]);
  const visibleDownloads = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return downloads.filter((download) => {
      if (section === "queue") {
        if (!["queued", "preparing", "downloading", "paused", "metadata_acquiring", "recovering", "seeding"].includes(download.status)) return false;
      }
      return !query || `${download.filename} ${download.id} ${download.status}`
        .toLocaleLowerCase()
        .includes(query);
    });
  }, [downloads, search, section]);

  const operate = useCallback(async (
    download: RemoteDownload,
    action: "pause" | "resume" | "cancel" | "remove",
  ) => {
    if (!client || busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (!capabilities.has(`download.${action}`)) {
        throw new RemoteError(`download.${action} is not available in the current protocol.`, "protocol");
      }
      await client.call(`download.${action}`, { id: download.id });
      await Promise.all([
        refreshDownloads(client),
        refreshEngineStatuses(client, protocol?.capabilities ?? []),
      ]);
      setNotice(action === "remove" ? "Download record removed. Files on disk are preserved." : `Download ${action} request completed.`);
    } catch (cause) {
      handleError(cause);
    } finally {
      setBusy(false);
    }
  }, [busy, capabilities, client, handleError, refreshDownloads]);

  const changePriority = useCallback(async (download: RemoteDownload, value: DownloadPriority) => {
    if (!client || busy) return;
    setBusy(true);
    setError("");
    try {
      await client.call("download.setPriority", { id: download.id, priority: value });
      await refreshDownloads(client);
    } catch (cause) {
      handleError(cause);
    } finally {
      setBusy(false);
    }
  }, [busy, client, handleError, refreshDownloads]);

  const moveInQueue = useCallback(async (
    download: RemoteDownload,
    direction: "up" | "down",
  ) => {
    if (!client || busy) return;
    setBusy(true);
    setError("");
    try {
      await client.call(direction === "up" ? "queue.moveUp" : "queue.moveDown", {
        id: download.id,
      });
      await refreshDownloads(client);
    } catch (cause) {
      handleError(cause);
    } finally {
      setBusy(false);
    }
  }, [busy, client, handleError, refreshDownloads]);

  const setQueueDispatchPaused = useCallback(async (paused: boolean) => {
    if (!client || busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await client.call(paused ? "queue.pauseDispatch" : "queue.resumeDispatch");
      setNotice(paused
        ? "Queue dispatch held. Active downloads will continue."
        : "Queue dispatch resumed.");
    } catch (cause) {
      handleError(cause);
    } finally {
      setBusy(false);
    }
  }, [busy, client, handleError]);

  const addDownload = useCallback(async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!client || busy || !newUrl.trim()) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (!capabilities.has("download.create")) {
        throw new RemoteError("download.create is not available in the current protocol.", "protocol");
      }
      const created = await client.call<CreateResult>("download.create", {
        url: newUrl.trim(),
        priority,
        ...(newUrl.trim().toLowerCase().startsWith("magnet:")
          && torrentEngine !== "automatic" ? { engine: torrentEngine } : {}),
      });
      setNewUrl("");
      await refreshDownloads(client);
      setNotice(`Download request accepted (${created.id}).`);
    } catch (cause) {
      handleError(cause);
    } finally {
      setBusy(false);
    }
  }, [busy, capabilities, client, handleError, newUrl, priority, refreshDownloads, torrentEngine]);

  const refreshNow = useCallback(async () => {
    if (!client || refreshing) return;
    setRefreshing(true);
    setError("");
    try {
      await refreshDownloads(client);
      const currentHealth = await client.health();
      setHealth(currentHealth);
    } catch (cause) {
      handleError(cause);
    } finally {
      setRefreshing(false);
    }
  }, [client, handleError, protocol?.capabilities, refreshDownloads, refreshEngineStatuses, refreshing]);

  if (!client) {
    return (
      <main className="connect-screen">
        <section className="connect-card">
          <div className="brand-lockup">
            <span className="brand-glyph">F</span>
            <div>
              <p className="eyebrow">FETCHX REMOTE</p>
              <h1>Connect to your server</h1>
            </div>
          </div>
          <p className="muted connect-intro">
            The web client uses the FetchX JSON-RPC protocol. Credentials stay in memory
            for this page session and are never saved in browser storage.
          </p>
          {error && <div className="alert alert-error" role="alert">{error}</div>}
          <form className="connect-form" onSubmit={connect}>
            <label>
              Server URL
              <input
                autoComplete="url"
                required
                value={serverUrl}
                onChange={(change) => setServerUrl(change.target.value)}
                placeholder="https://fetchx.example.com"
              />
            </label>
            <label>
              Authentication token
              <input
                autoComplete="off"
                required
                type="password"
                value={tokenInput}
                onChange={(change) => setTokenInput(change.target.value)}
                placeholder="Paste the token generated by fetchx server init-auth"
              />
            </label>
            <button className="primary-button full-width" type="submit" disabled={connecting}>
              {connecting ? "Connecting…" : "Connect securely"}
            </button>
          </form>
          <div className="security-note">
            <span aria-hidden="true">⌑</span>
            <p>Non-loopback servers must use HTTPS. Configure the exact web UI origin in the server allow-list.</p>
          </div>
          <p className="setup-help">
            Local development: start the server with{" "}
            <code>--allowed-origin http://127.0.0.1:5173</code>.
          </p>
        </section>
      </main>
    );
  }

  const title = navigation.find((item) => item.id === section)?.label ?? "Dashboard";
  const onlineCount = downloads.filter((item) =>
    ["preparing", "downloading", "metadata_acquiring", "recovering", "seeding"].includes(item.status)
  ).length;
  const queuedCount = downloads.filter((item) => item.status === "queued").length;
  const completedCount = downloads.filter((item) => item.status === "completed").length;
  const totalDownloaded = downloads.reduce((sum, item) => sum + item.downloaded_bytes, 0);

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-glyph brand-glyph-small">F</span>
          <div>
            <strong>FetchX</strong>
            <span>REMOTE CLIENT</span>
          </div>
        </div>
        <div className="server-badge">
          <span className="status-dot online" />
          <span className="server-badge-text">{client.serverOrigin}</span>
          <button className="icon-button disconnect-button" onClick={() => disconnect()} aria-label="Disconnect" title="Disconnect">↗</button>
        </div>
        <nav className="navigation" aria-label="Remote server sections">
          {navigation.map((item) => {
            const isAvailable = isSectionAvailable(item.id, capabilities);
            return (
              <button
                className={`nav-link ${section === item.id ? "selected" : ""}`}
                key={item.id}
                onClick={() => setSection(item.id)}
                aria-current={section === item.id ? "page" : undefined}
                title={!isAvailable ? "Not available in current protocol" : undefined}
              >
                <span className="nav-icon" aria-hidden="true">{item.icon}</span>
                <span>{item.label}</span>
                {item.id === "downloads" && <span className="nav-count">{downloads.length}</span>}
                {!isAvailable && item.id !== "system" && <span className="nav-lock" aria-label="Unavailable">—</span>}
              </button>
            );
          })}
        </nav>
        <div className="sidebar-footer">
          <span className={`status-dot ${eventState === "Live" ? "online" : "idle"}`} />
          <span>Events {eventState.toLowerCase()}</span>
          <span className="protocol-tag">{protocol?.protocol_version}</span>
        </div>
      </aside>

      <main className="main-panel">
        <header className="topbar">
          <div>
            <p className="eyebrow">REMOTE SERVER</p>
            <h1>{title}</h1>
          </div>
          <div className="topbar-actions">
            <span className="server-version">v{protocol?.server_version ?? "—"}</span>
            <button className="secondary-button" onClick={() => void refreshNow()} disabled={refreshing}>
              {refreshing ? "Refreshing…" : "↻ Refresh"}
            </button>
          </div>
        </header>

        <div className="content-scroll">
          {error && <div className="alert alert-error" role="alert"><span>{error}</span><button onClick={() => setError("")}>Dismiss</button></div>}
          {notice && <div className="alert alert-info" role="status"><span>{notice}</span><button onClick={() => setNotice("")}>Dismiss</button></div>}

          {section === "dashboard" && (
            <Dashboard
              health={health}
              protocol={protocol}
              downloads={downloads}
              onlineCount={onlineCount}
              queuedCount={queuedCount}
              completedCount={completedCount}
              totalDownloaded={totalDownloaded}
              eventState={eventState}
              lastEvent={lastEvent}
              onOpenDownloads={() => setSection("downloads")}
              onOpenSystem={() => setSection("system")}
            />
          )}
          {section === "downloads" && (
            <DownloadsView
              downloads={visibleDownloads}
              allDownloads={downloads}
              search={search}
              onSearch={setSearch}
              newUrl={newUrl}
              onNewUrl={setNewUrl}
              priority={priority}
              onPriority={setPriority}
              torrentEngine={torrentEngine}
              onTorrentEngine={setTorrentEngine}
              onAdd={(event) => void addDownload(event)}
              onAction={(download, action) => void operate(download, action)}
              onPriorityChange={(download, value) => void changePriority(download, value)}
              onQueueMove={(download, direction) => void moveInQueue(download, direction)}
              client={client}
              onRefreshDownloads={() => void refreshDownloads(client).catch(handleError)}
              busy={busy}
              capabilities={capabilities}
            />
          )}
          {section === "queue" && (
            <QueueView
              downloads={visibleDownloads}
              allCount={downloads.length}
              onAction={(download, action) => void operate(download, action)}
              onQueueMove={(download, direction) => void moveInQueue(download, direction)}
              onQueueDispatchChange={(paused) => void setQueueDispatchPaused(paused)}
              busy={busy}
              capabilities={capabilities}
            />
          )}
          {section === "system" && (
            <SystemView health={health} protocol={protocol} eventState={eventState} />
          )}
          {section === "engines" && (
            <EngineRuntimeView engines={engineStatuses} />
          )}
          {section !== "dashboard" && section !== "downloads" && section !== "queue" && section !== "engines" && section !== "system" && (
            <UnavailableView section={section} />
          )}
        </div>
      </main>
    </div>
  );
}

function isSectionAvailable(section: Section, capabilities: Set<string>): boolean {
  switch (section) {
    case "dashboard":
    case "system":
      return true;
    case "downloads":
    case "queue":
      return capabilities.has("download.list");
    case "engines":
      return capabilities.has("engine.runtimeStatus");
    case "scheduler":
    case "settings":
    case "devices":
    case "plugins":
      return false;
  }
}

function EngineRuntimeView({ engines }: { engines: EngineRuntimeStatusEntry[] | null }) {
  if (!engines) {
    return <UnavailableView section="engines" />;
  }
  return (
    <div className="page-stack">
      <section className="welcome-panel">
        <div>
          <p className="eyebrow">ENGINE RUNTIME</p>
          <h2>Download engines</h2>
          <p className="muted">Live availability from the shared FetchX runtime. This view does not expose executable paths, configuration, or failure details.</p>
        </div>
      </section>
      <section className="panel" aria-label="Engine runtime status">
        <div className="engine-status-list">
          {engines.map((engine) => (
            <div className="engine-status-row" key={engine.engine}>
              <div>
                <strong>{engine.engine.replaceAll("_", " ")}</strong>
                {engine.version && <span>{engine.version}</span>}
              </div>
              <span className={`pill ${engine.state === "available" ? "pill-green" : engine.state === "unavailable" ? "pill-red" : "pill-muted"}`}>
                {engine.state}
              </span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

function Dashboard(props: {
  health: HealthInfo | null;
  protocol: ProtocolInfo | null;
  downloads: RemoteDownload[];
  onlineCount: number;
  queuedCount: number;
  completedCount: number;
  totalDownloaded: number;
  eventState: string;
  lastEvent: string;
  onOpenDownloads: () => void;
  onOpenSystem: () => void;
}) {
  const { health, protocol, downloads, onlineCount, queuedCount, completedCount } = props;
  const recent = [...downloads].sort((a, b) => b.updated_at - a.updated_at).slice(0, 5);
  return (
    <div className="page-stack">
      <section className="welcome-panel">
        <div>
          <p className="eyebrow">OVERVIEW</p>
          <h2>Your remote server is connected.</h2>
          <p className="muted">Live state is provided by the FetchX server. Actions are authorized by the server, not this interface.</p>
        </div>
        <button className="primary-button" onClick={props.onOpenDownloads}>View downloads <span aria-hidden="true">→</span></button>
      </section>

      <section className="metric-grid" aria-label="Download summary">
        <MetricCard label="Active" value={onlineCount} icon="↗" tone="blue" />
        <MetricCard label="Queued" value={queuedCount} icon="≡" tone="violet" />
        <MetricCard label="Completed" value={completedCount} icon="✓" tone="green" />
        <MetricCard label="Tracked" value={downloads.length} icon="↓" tone="amber" />
      </section>

      <div className="dashboard-grid">
        <section className="panel recent-panel">
          <div className="panel-heading">
            <div><p className="eyebrow">ACTIVITY</p><h2>Recent downloads</h2></div>
            <button className="text-button" onClick={props.onOpenDownloads}>See all <span aria-hidden="true">→</span></button>
          </div>
          {recent.length
            ? <DownloadTable downloads={recent} compact />
            : <EmptyState title="Nothing here yet" detail="Remote downloads will appear after you add them." />}
        </section>
        <section className="panel server-panel">
          <div className="panel-heading">
            <div><p className="eyebrow">SERVER</p><h2>Connection health</h2></div>
            <span className={`pill ${health?.status === "ok" ? "pill-green" : "pill-muted"}`}>
              <span className="status-dot online" />{health?.status ?? "Unknown"}
            </span>
          </div>
          <div className="detail-list">
            <Detail label="Protocol" value={protocol?.protocol_version ?? "—"} />
            <Detail label="Server version" value={protocol?.server_version ?? "—"} />
            <Detail label="Event stream" value={props.eventState} />
            <Detail label="Last event" value={props.lastEvent || "No events received"} />
            <Detail label="Bytes downloaded" value={formatBytes(props.totalDownloaded)} />
          </div>
          <button className="text-button system-link" onClick={props.onOpenSystem}>System details <span aria-hidden="true">→</span></button>
        </section>
      </div>
    </div>
  );
}

function DownloadsView(props: {
  client: RemoteClient;
  downloads: RemoteDownload[];
  allDownloads: RemoteDownload[];
  search: string;
  onSearch: (value: string) => void;
  newUrl: string;
  onNewUrl: (value: string) => void;
  priority: DownloadPriority;
  onPriority: (value: DownloadPriority) => void;
  torrentEngine: TorrentEngineChoice;
  onTorrentEngine: (value: TorrentEngineChoice) => void;
  onAdd: (event: FormEvent<HTMLFormElement>) => void;
  onAction: (download: RemoteDownload, action: "pause" | "resume" | "cancel" | "remove") => void;
  onPriorityChange: (download: RemoteDownload, value: DownloadPriority) => void;
  onQueueMove: (download: RemoteDownload, direction: "up" | "down") => void;
  onRefreshDownloads: () => void;
  busy: boolean;
  capabilities: Set<string>;
}) {
  const [detailsId, setDetailsId] = useState<string | null>(null);
  const canCreate = props.capabilities.has("download.create");
  const isMagnet = props.newUrl.trim().toLowerCase().startsWith("magnet:");
  const waitingForSelection = props.downloads.find((download) =>
    download.engine === "native_torrent" && download.status === "metadata_acquiring"
  );
  const detailsDownload = props.downloads.find((download) => download.id === detailsId)
    ?? waitingForSelection;
  return (
    <div className="page-stack">
      <section className="panel add-panel">
        <div className="panel-heading">
          <div><p className="eyebrow">NEW TRANSFER</p><h2>Add a download</h2></div>
          <span className="capability-chip">{props.allDownloads.length} records</span>
        </div>
        <form className="add-form" onSubmit={props.onAdd}>
          <label className="visually-hidden" htmlFor="download-url">Download URL or magnet URI</label>
          <input
            id="download-url"
            type="text"
            value={props.newUrl}
            onChange={(event) => props.onNewUrl(event.target.value)}
            placeholder="https://example.com/file.zip or magnet:?xt=..."
            required
            disabled={!canCreate || props.busy}
          />
          <label className="visually-hidden" htmlFor="download-priority">Priority</label>
          <select
            id="download-priority"
            value={props.priority}
            onChange={(event) => props.onPriority(event.target.value as DownloadPriority)}
            disabled={!canCreate || props.busy}
          >
            <option value="low">Low priority</option>
            <option value="normal">Normal priority</option>
            <option value="high">High priority</option>
          </select>
          {isMagnet && (
            <select
              aria-label="Torrent engine"
              value={props.torrentEngine}
              onChange={(event) => props.onTorrentEngine(event.target.value as TorrentEngineChoice)}
              disabled={!canCreate || props.busy}
            >
              <option value="automatic">Automatic (server default)</option>
              <option value="aria2">aria2</option>
              <option value="native_torrent">Native torrent</option>
            </select>
          )}
          <button className="primary-button" type="submit" disabled={!canCreate || props.busy}>
            {props.busy ? "Sending…" : "Add download"}
          </button>
        </form>
        {!canCreate && <p className="muted small-note">Download creation is not advertised by this server.</p>}
      </section>
      <section className="panel">
        <div className="panel-heading downloads-heading">
          <div><p className="eyebrow">REMOTE QUEUE</p><h2>Downloads</h2></div>
          <label className="search-field">
            <span aria-hidden="true">⌕</span>
            <input value={props.search} onChange={(event) => props.onSearch(event.target.value)} placeholder="Search name or ID" />
          </label>
        </div>
        {props.downloads.length
          ? <DownloadTable
            downloads={props.downloads}
            onAction={props.onAction}
            onPriorityChange={props.onPriorityChange}
            onQueueMove={props.onQueueMove}
            onTorrentDetails={(download) => setDetailsId((id) => id === download.id ? null : download.id)}
            busy={props.busy}
            canControl={props.capabilities.has("download.pause") && props.capabilities.has("download.resume")}
            canCancel={props.capabilities.has("download.cancel")}
            canRemove={props.capabilities.has("download.remove")}
            canSetPriority={props.capabilities.has("download.setPriority")}
            canMoveQueue={props.capabilities.has("queue.moveUp") && props.capabilities.has("queue.moveDown")}
          />
          : <EmptyState title={props.search ? "No matching downloads" : "No downloads yet"} detail={props.search ? "Try a different search." : "Add a URL or magnet URI above to start."} />}
        {detailsDownload && (
          <TorrentSelectionPanel
            client={props.client}
            download={detailsDownload}
            capabilities={props.capabilities}
            busy={props.busy}
            onConfirmed={props.onRefreshDownloads}
          />
        )}
      </section>
    </div>
  );
}

function QueueView(props: {
  downloads: RemoteDownload[];
  allCount: number;
  onAction: (download: RemoteDownload, action: "pause" | "resume" | "cancel" | "remove") => void;
  onQueueMove: (download: RemoteDownload, direction: "up" | "down") => void;
  onQueueDispatchChange: (paused: boolean) => void;
  busy: boolean;
  capabilities: Set<string>;
}) {
  return (
    <div className="page-stack">
      <section className="welcome-panel queue-intro">
        <div>
          <p className="eyebrow">QUEUE SNAPSHOT</p>
          <h2>{props.downloads.length} queued or active records</h2>
          <p className="muted">Reorder queued downloads within their priority group. Active downloads cannot be moved.</p>
          <div className="toolbar-actions">
            {props.capabilities.has("queue.pauseDispatch") && (
              <button
                disabled={props.busy}
                onClick={() => props.onQueueDispatchChange(true)}
              >
                Hold queue
              </button>
            )}
            {props.capabilities.has("queue.resumeDispatch") && (
              <button
                disabled={props.busy}
                onClick={() => props.onQueueDispatchChange(false)}
              >
                Resume queue
              </button>
            )}
          </div>
        </div>
      </section>
      <section className="panel">
        {props.downloads.length
          ? <DownloadTable
            downloads={props.downloads}
            onAction={props.onAction}
            onQueueMove={props.onQueueMove}
            busy={props.busy}
            canControl={props.capabilities.has("download.pause") && props.capabilities.has("download.resume")}
            canCancel={props.capabilities.has("download.cancel")}
            canRemove={false}
            canMoveQueue={props.capabilities.has("queue.moveUp") && props.capabilities.has("queue.moveDown")}
          />
          : <EmptyState title="The queue is clear" detail={`${props.allCount} total download records are not queued or active.`} />}
      </section>
    </div>
  );
}

function SystemView(props: {
  health: HealthInfo | null;
  protocol: ProtocolInfo | null;
  eventState: string;
}) {
  return (
    <div className="page-stack">
      <section className="welcome-panel">
        <div>
          <p className="eyebrow">SYSTEM STATUS</p>
          <h2>FetchX remote service</h2>
          <p className="muted">Health comes from the public health endpoint; capabilities and version come from authenticated system.discover.</p>
        </div>
        <span className="pill pill-green"><span className="status-dot online" />{props.health?.status ?? "Unknown"}</span>
      </section>
      <section className="panel">
        <div className="panel-heading"><div><p className="eyebrow">PROTOCOL</p><h2>Server information</h2></div></div>
        <div className="detail-list system-details">
          <Detail label="Protocol version" value={props.protocol?.protocol_version ?? "—"} />
          <Detail label="Server version" value={props.protocol?.server_version ?? "—"} />
          <Detail label="Health response" value={props.health?.status ?? "—"} />
          <Detail label="Event stream" value={props.eventState} />
        </div>
        <h3 className="subheading">Advertised capabilities</h3>
        <div className="capability-list">
          {props.protocol?.capabilities.map((capability) => (
            <code className="capability-chip" key={capability}>{capability}</code>
          ))}
        </div>
        <p className="muted small-note">Capabilities describe the server protocol. Authorization is enforced by FetchX on every request.</p>
      </section>
    </div>
  );
}

function UnavailableView({ section }: { section: Exclude<Section, "dashboard" | "downloads" | "queue" | "system"> }) {
  const item = unavailable[section];
  return (
    <section className="unavailable-panel">
      <span className="unavailable-icon" aria-hidden="true">◌</span>
      <p className="eyebrow">NOT AVAILABLE IN CURRENT PROTOCOL</p>
      <h2>{item.title}</h2>
      <p className="muted">{item.detail}</p>
      <p className="small-note">This view is intentionally not simulated by the web client.</p>
    </section>
  );
}

function MetricCard({ label, value, icon, tone }: {
  label: string;
  value: number;
  icon: string;
  tone: "blue" | "violet" | "green" | "amber";
}) {
  return (
    <article className="metric-card">
      <span className={`metric-icon ${tone}`} aria-hidden="true">{icon}</span>
      <div><p>{label}</p><strong>{value}</strong></div>
      <span className="metric-accent" />
    </article>
  );
}

function DownloadTable(props: {
  downloads: RemoteDownload[];
  compact?: boolean;
  onAction?: (download: RemoteDownload, action: "pause" | "resume" | "cancel" | "remove") => void;
  onPriorityChange?: (download: RemoteDownload, value: DownloadPriority) => void;
  onQueueMove?: (download: RemoteDownload, direction: "up" | "down") => void;
  onTorrentDetails?: (download: RemoteDownload) => void;
  busy?: boolean;
  canControl?: boolean;
  canCancel?: boolean;
  canRemove?: boolean;
  canSetPriority?: boolean;
  canMoveQueue?: boolean;
}) {
  return (
    <div className="table-scroll">
      <table className="download-table">
        <thead>
          <tr>
            <th>Download</th>
            <th>Status</th>
            <th>Progress</th>
            <th>Speed</th>
            {!props.compact && <th>Engine</th>}
            {!props.compact && <th>Priority</th>}
            {!props.compact && <th>Actions</th>}
          </tr>
        </thead>
        <tbody>
          {props.downloads.map((download) => {
            const percentage = download.progress_basis_points === null
              ? null
              : Math.min(100, Math.max(0, download.progress_basis_points / 100));
            const canPause = ["preparing", "downloading", "metadata_acquiring", "seeding"].includes(download.status);
            const canResume = download.status === "paused";
            return (
              <tr key={download.id}>
                <td className="download-name-cell">
                  <strong title={download.filename}>{download.filename}</strong>
                  <span title={download.id}>{download.id}</span>
                </td>
                <td><StatusPill status={download.status} /></td>
                <td className="progress-cell">
                  <progress
                    className="progress-line"
                    value={percentage ?? 0}
                    max={100}
                    aria-label={`Progress for ${download.filename}`}
                  />
                  <span>{formatBytes(download.downloaded_bytes)}{download.total_bytes === null ? "" : ` / ${formatBytes(download.total_bytes)}`} {percentage === null ? "" : `· ${percentage.toFixed(1)}%`}</span>
                </td>
                <td className="speed-cell">{formatSpeed(download.speed_bytes_per_second)}</td>
                {!props.compact && <td><code className="engine-label">{formatEngine(download.engine)}</code></td>}
                {!props.compact && (
                  <td>
                    {props.onPriorityChange
                      ? <select
                        className="priority-select"
                        aria-label={`Priority for ${download.filename}`}
                        value={download.priority}
                        disabled={!props.canSetPriority || props.busy}
                        onChange={(event) => props.onPriorityChange?.(download, event.target.value as DownloadPriority)}
                      >
                        <option value="low">Low</option>
                        <option value="normal">Normal</option>
                        <option value="high">High</option>
                      </select>
                      : <span className="muted">{download.priority}</span>}
                  </td>
                )}
                {!props.compact && (
                  <td>
                    <div className="row-actions">
                      {props.onAction && props.canControl && canPause && <button disabled={props.busy} onClick={() => props.onAction?.(download, "pause")}>Pause</button>}
                      {props.onAction && props.canControl && canResume && <button disabled={props.busy} onClick={() => props.onAction?.(download, "resume")}>Resume</button>}
                      {props.onTorrentDetails && download.engine === "native_torrent" && (
                        <button disabled={props.busy} onClick={() => props.onTorrentDetails?.(download)}>Files</button>
                      )}
                      {props.onQueueMove && props.canMoveQueue && download.status === "queued" && <>
                        <button disabled={props.busy} aria-label={`Move ${download.filename} earlier in queue`} onClick={() => props.onQueueMove?.(download, "up")}>↑</button>
                        <button disabled={props.busy} aria-label={`Move ${download.filename} later in queue`} onClick={() => props.onQueueMove?.(download, "down")}>↓</button>
                      </>}
                      {props.onAction && props.canCancel && !["completed", "cancelled", "failed", "stopped"].includes(download.status) && <button disabled={props.busy} onClick={() => props.onAction?.(download, "cancel")}>Cancel</button>}
                      {props.onAction && props.canRemove && <button className="danger-text" disabled={props.busy} onClick={() => {
                        if (window.confirm(`Remove "${download.filename}" from FetchX? Files on disk are preserved.`)) {
                          props.onAction?.(download, "remove");
                        }
                      }}>Remove</button>}
                      {download.error_code && <span className="error-code" title={`Error code: ${download.error_code}`}>{download.error_code}</span>}
                    </div>
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function StatusPill({ status }: { status: DownloadStatus }) {
  const tone = ["completed", "stopped"].includes(status)
    ? "green"
    : ["failed", "cancelled"].includes(status)
      ? "red"
      : ["downloading", "seeding"].includes(status)
        ? "blue"
        : status === "paused"
          ? "amber"
          : "muted";
  return <span className={`pill pill-${tone}`}>{status.replaceAll("_", " ")}</span>;
}

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return <div className="empty-state"><span aria-hidden="true">↓</span><h3>{title}</h3><p>{detail}</p></div>;
}

function Detail({ label, value }: { label: string; value: string }) {
  return <div className="detail-row"><span>{label}</span><strong>{value}</strong></div>;
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatSpeed(bytesPerSecond: number | null): string {
  return bytesPerSecond === null ? "—" : `${formatBytes(bytesPerSecond)}/s`;
}

function formatEngine(engine: string): string {
  return engine.replaceAll("_", " ");
}
