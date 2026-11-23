import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { getAppInfo } from "./api/app";
import {
  getQueueSnapshot,
  pauseQueueDispatch,
  resumeQueueDispatch,
  stopTorrentSeeding,
  updateDownloadUrl,
  pauseAllDownloads,
  resumeAllDownloads,
  openDownloadFolder,
} from "./api/downloads";
import type { Download, DownloadPriority, QueueSnapshot, UpdateCheckResult } from "./api/types";
import { AddDialog } from "./components/AddDialog";
import { BatchAddDialog } from "./components/BatchAddDialog";
import { ContextMenu } from "./components/ContextMenu";
import { BandwidthLimitDialog } from "./components/BandwidthLimitDialog";
import { BulkActionBar } from "./components/BulkActionBar";
import { BrowserExtensionSetupModal } from "./components/BrowserExtensionSetupModal";
import { OnboardingModal } from "./components/OnboardingModal";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { DetailPanel } from "./components/DetailPanel";
import { DownloadRow } from "./components/DownloadRow";
import { EmptyState } from "./components/EmptyState";
import { Sidebar } from "./components/Sidebar";
import { SettingsPage } from "./components/SettingsPage";
import { HistoryScreen } from "./components/HistoryScreen";
import { SpeedChart } from "./components/SpeedChart";
import { ConversionDialog } from "./components/ConversionDialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { NotificationEvent } from "./api/types";
import { ToastStack, useToasts } from "./components/Toast";
import { Toolbar } from "./components/Toolbar";
import { useDownloads } from "./hooks/useDownloads";
import { errorMessage } from "./lib/format";
import { canCancel, canPause, countStatuses } from "./lib/status";

// ---------------------------------------------------------------------------
// Public types used by sub-components
// ---------------------------------------------------------------------------

export type NavSection =
  | "all"
  | "active"
  | "queued"
  | "paused"
  | "scheduled"
  | "completed"
  | "failed"
  | "cancelled"
  | "settings"
  | "history";

export type SortKey =
  | "newest"
  | "oldest"
  | "filename"
  | "progress"
  | "size"
  | "speed"
  | "priority"
  | "queue";

// ---------------------------------------------------------------------------
// Filtering and sorting (pure functions, client-side only)
// ---------------------------------------------------------------------------

const PRIORITY_ORDER: Record<string, number> = { high: 0, normal: 1, low: 2 };

function filterDownloads(
  downloads: Download[],
  section: NavSection,
  search: string,
  scheduledIds: Set<string>,
): Download[] {
  const q = search.trim().toLowerCase();
  return downloads.filter((d) => {
    switch (section) {
      case "all": break;
      case "active":
        if (
          d.status !== "downloading"
          && d.status !== "preparing"
          && d.status !== "metadata_acquiring"
          && d.status !== "recovering"
          && d.status !== "seeding"
        ) return false;
        break;
      case "queued":
        if (d.status !== "queued") return false;
        break;
      case "paused":
        if (d.status !== "paused") return false;
        break;
      case "scheduled":
        if (!scheduledIds.has(d.id)) return false;
        break;
      case "completed":
        if (d.status !== "completed") return false;
        break;
      case "failed":
        if (d.status !== "failed") return false;
        break;
      case "cancelled":
        if (d.status !== "cancelled") return false;
        break;
    }
    if (q) {
      if (!`${d.filename} ${d.url}`.toLowerCase().includes(q)) return false;
    }
    return true;
  });
}

function sortDownloads(downloads: Download[], sort: SortKey): Download[] {
  const sorted = [...downloads];
  switch (sort) {
    case "newest":  sorted.sort((a, b) => b.created_at - a.created_at); break;
    case "oldest":  sorted.sort((a, b) => a.created_at - b.created_at); break;
    case "filename": sorted.sort((a, b) => a.filename.localeCompare(b.filename)); break;
    case "progress":
      sorted.sort((a, b) => (b.progress.percentage_basis_points ?? 0) - (a.progress.percentage_basis_points ?? 0));
      break;
    case "size":
      sorted.sort((a, b) => (b.progress.total_bytes ?? 0) - (a.progress.total_bytes ?? 0));
      break;
    case "speed":
      sorted.sort((a, b) => (b.progress.speed_bytes_per_second ?? 0) - (a.progress.speed_bytes_per_second ?? 0));
      break;
    case "priority":
      sorted.sort((a, b) => (PRIORITY_ORDER[a.priority] ?? 1) - (PRIORITY_ORDER[b.priority] ?? 1));
      break;
    case "queue":
      sorted.sort(
        (a, b) =>
          (a.queue_position ?? Number.MAX_SAFE_INTEGER)
          - (b.queue_position ?? Number.MAX_SAFE_INTEGER),
      );
      break;
  }
  return sorted;
}

const SECTION_TITLES: Record<NavSection, string> = {
  all: "All Downloads", active: "Active", queued: "Queued", paused: "Paused",
  scheduled: "Scheduled", completed: "Completed", failed: "Failed", cancelled: "Cancelled",
  settings: "Settings", history: "History",
};

const EMPTY_MESSAGES: Record<NavSection, { title: string; subtitle: string }> = {
  all:       { title: "No downloads yet",          subtitle: "Add a download or drop a URL here" },
  active:    { title: "Nothing downloading",        subtitle: "Start a queued download to see it here." },
  queued:    { title: "Queue is empty",             subtitle: "Add a download to fill the queue." },
  paused:    { title: "Nothing paused",             subtitle: "Pause an active download to see it here." },
  scheduled: { title: "No scheduled downloads",    subtitle: "Use the schedule button on a queued download." },
  completed: { title: "No completed downloads",    subtitle: "Finished downloads will appear here." },
  failed:    { title: "No failed downloads",       subtitle: "Downloads that encounter errors will appear here." },
  cancelled: { title: "No cancelled downloads",    subtitle: "" },
  history:   { title: "No history",        subtitle: "Completed downloads appear here." },
  settings:  { title: "",                          subtitle: "" },
};

// ---------------------------------------------------------------------------
// Confirmation dialog state
// ---------------------------------------------------------------------------

type ConfirmState =
  | { kind: "cancel-one";  ids: [string]; filename: string }
  | { kind: "delete-one";  ids: [string]; filename: string }
  | { kind: "cancel-bulk"; ids: string[] }
  | { kind: "delete-bulk"; ids: string[] }
  | null;

function confirmTitle(state: NonNullable<ConfirmState>): string {
  switch (state.kind) {
    case "cancel-one":  return "Cancel download?";
    case "delete-one":  return "Delete download?";
    case "cancel-bulk": return `Cancel ${state.ids.length} downloads?`;
    case "delete-bulk": return `Delete ${state.ids.length} downloads?`;
  }
}

function confirmMessage(state: NonNullable<ConfirmState>): string {
  switch (state.kind) {
    case "cancel-one":
      return `"${state.filename}" will be cancelled. You can retry it later.`;
    case "delete-one":
      return `"${state.filename}" will be removed. The download entry will be deleted, but downloaded and partial files will be preserved on disk.`;
    case "cancel-bulk":
      return `${state.ids.length} downloads will be cancelled. You can retry them later.`;
    case "delete-bulk":
      return `${state.ids.length} downloads will be removed. Download entries will be deleted, but files on disk will be preserved.`;
  }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export function App() {
  const [appVersion, setAppVersion] = useState<string | undefined>(undefined);
  const [navSection, setNavSection] = useState<NavSection>("all");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<SortKey>("newest");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [checkedIds, setCheckedIds] = useState<Set<string>>(new Set());
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [addDialogUrl, setAddDialogUrl] = useState("");
  const [globalError, setGlobalError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmState>(null);
  const [snapshot, setSnapshot] = useState<QueueSnapshot | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [conversionDownload, setConversionDownload] = useState<import("./api/types").Download | null>(null);
  const [showBatchDialog, setShowBatchDialog] = useState(false);
  const [bandwidthLimitTarget, setBandwidthLimitTarget] = useState<{ id: string; filename: string; currentLimit: number | null } | null>(null);
  const [clipboardBanners, setClipboardBanners] = useState<Array<{ id: string; url: string; filename: string; timeoutId: ReturnType<typeof setTimeout> }>>([]);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; download: Download } | null>(null);
  const [theme, setTheme] = useState(() => localStorage.getItem("fetchx_theme") ?? "dark");
  const searchRef = useRef<HTMLInputElement>(null);
  const prevCompletedRef = useRef<Set<string>>(new Set());
  const lastNotificationRef = useRef('');
  const { toasts, addToast, dismiss: dismissToast } = useToasts();

  // Speed history for per-download sparklines
  const speedHistoryRef = useRef<Map<string, number[]>>(new Map());
  const [speedHistorySnapshot, setSpeedHistorySnapshot] = useState<Map<string, number[]>>(new Map());

  // Update banner state for in-app update notifications
  const [updateBanner, setUpdateBanner] = useState<UpdateCheckResult | null>(null);

  const {
    downloads, schedules, loading, error, busyId, load,
    ensureScheduleFetched, pause, resume, cancel, remove, retry,
    bulkOperation, setPriority, moveUpInQueue, moveDownInQueue, setSchedule, clearSchedule, dismissError,
  } = useDownloads();

  // -------------------------------------------------------------------------
  // App info
  // -------------------------------------------------------------------------

  useEffect(() => {
    void getAppInfo()
      .then((info) => setAppVersion(info.version))
      .catch(() => { /* cosmetic */ });
  }, []);

  // Apply and persist the theme (dark / light).
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("fetchx_theme", theme);
  }, [theme]);

  // Listen for the update-available event emitted by the background checker
  useEffect(() => {
    let off: (() => void) | undefined;
    void listen<UpdateCheckResult>("fetchx-update-available", (e) => {
      setUpdateBanner(e.payload);
    }).then((fn) => { off = fn; });
    return () => off?.();
  }, []);

  // Listen for backend notifications
  useEffect(() => {
    let off: (() => void)|undefined;
    void listen<NotificationEvent>("fetchx-notification", (e) => {
      const text = e.payload.title + (e.payload.body ? ": " + e.payload.body : "");
      if (text !== lastNotificationRef.current) {
        lastNotificationRef.current = text;
        addToast(text, "success");
      }
    }).then((fn) => { off = fn; });
    return () => off?.();
  }, [addToast]);

  // -------------------------------------------------------------------------
  // Queue snapshot — refresh on every download event + on a slow background
  // interval to catch connection allocation changes not triggered by events.
  // The interval is 10 s which is acceptable since this is supplementary info.
  // -------------------------------------------------------------------------

  const refreshSnapshot = useCallback(() => {
    void getQueueSnapshot()
      .then(setSnapshot)
      .catch(() => { /* non-critical */ });
  }, []);

  const toggleQueueDispatch = useCallback(async () => {
    try {
      const next = snapshot?.manual_dispatch_paused
        ? await resumeQueueDispatch()
        : await pauseQueueDispatch();
      setSnapshot(next);
      setGlobalError(null);
    } catch (cause) {
      setGlobalError(errorMessage(cause));
    }
  }, [snapshot?.manual_dispatch_paused]);

  useEffect(() => {
    refreshSnapshot();
    const id = setInterval(refreshSnapshot, 10_000);
    return () => clearInterval(id);
  }, [refreshSnapshot]);

  // Refresh snapshot whenever download list changes (events arrive).
  useEffect(() => {
    if (!loading) refreshSnapshot();
  }, [downloads, loading, refreshSnapshot]);

  // -------------------------------------------------------------------------
  // Completion toast
  // -------------------------------------------------------------------------

  useEffect(() => {
    const prevIds = prevCompletedRef.current;
    const newCompleted = downloads.filter(
      (d) => d.status === "completed" && !prevIds.has(d.id),
    );
    for (const d of newCompleted) {
      addToast(`"${d.filename}" completed.`, "success");
      prevIds.add(d.id);
    }
    // Clean up removed downloads from the tracking set.
    const currentIds = new Set(downloads.map((d) => d.id));
    for (const id of prevIds) {
      if (!currentIds.has(id)) prevIds.delete(id);
    }
  }, [downloads, addToast]);

  // -------------------------------------------------------------------------
  // Per-download speed history for sparklines
  // -------------------------------------------------------------------------

  useEffect(() => {
    const map = speedHistoryRef.current;
    const currentIds = new Set(downloads.map((d) => d.id));

    // Remove entries for downloads that no longer exist
    for (const id of map.keys()) {
      if (!currentIds.has(id)) map.delete(id);
    }

    // Append new speed samples
    for (const download of downloads) {
      const speed = download.progress.speed_bytes_per_second;
      if (speed !== null && speed !== undefined) {
        const existing = map.get(download.id) ?? [];
        const updated = [...existing, speed].slice(-30);
        map.set(download.id, updated);
      }
    }

    // Update snapshot so components can read it
    setSpeedHistorySnapshot(() => {
      const next = new Map<string, number[]>();
      for (const [id, arr] of map) {
        next.set(id, arr);
      }
      return next;
    });
  }, [downloads]);

  // -------------------------------------------------------------------------
  // Clipboard URL banners
  // -------------------------------------------------------------------------

  useEffect(() => {
    let off: (() => void) | undefined;
    void listen<{ urls: string[] }>('clipboard_urls_detected', (event) => {
      const currentUrls = new Set(downloads.map((d) => d.url));
      setClipboardBanners((prev) => {
        const existing = new Set(prev.map((b) => b.url));
        const newBanners: typeof prev = [];
        for (const url of event.payload.urls) {
          if (currentUrls.has(url) || existing.has(url)) continue;
          const filename = (() => {
            try { return new URL(url).pathname.split('/').pop() || url; } catch { return url; }
          })();
          const id = crypto.randomUUID();
          const timeoutId = setTimeout(() => {
            setClipboardBanners((b) => b.filter((x) => x.id !== id));
          }, 15_000);
          newBanners.push({ id, url, filename, timeoutId });
        }
        return newBanners.length > 0 ? [...prev, ...newBanners] : prev;
      });
    }).then((fn) => { off = fn; });
    return () => off?.();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [downloads]);

  const dismissBanner = useCallback((id: string) => {
    setClipboardBanners((prev) => {
      const banner = prev.find((b) => b.id === id);
      if (banner) clearTimeout(banner.timeoutId);
      return prev.filter((b) => b.id !== id);
    });
  }, []);

  // -------------------------------------------------------------------------
  // Derived state
  // -------------------------------------------------------------------------

  const scheduledIds = useMemo(() => {
    const ids = new Set<string>();
    for (const [id, sched] of Object.entries(schedules)) {
      if (sched?.enabled) ids.add(id);
    }
    return ids;
  }, [schedules]);

  const baseCounts = useMemo(() => countStatuses(downloads), [downloads]);
  const counts = useMemo(() => ({ ...baseCounts, scheduled: scheduledIds.size }), [baseCounts, scheduledIds]);

  const activeCount = baseCounts.active;
  const pausedCount = baseCounts.paused;

  const handlePauseAll = useCallback(() => {
    void pauseAllDownloads()
      .then(() => { void load(); refreshSnapshot(); })
      .catch((cause: unknown) => setGlobalError(errorMessage(cause)));
  }, [load, refreshSnapshot]);

  const handleResumeAll = useCallback(() => {
    void resumeAllDownloads()
      .then(() => { void load(); refreshSnapshot(); })
      .catch((cause: unknown) => setGlobalError(errorMessage(cause)));
  }, [load, refreshSnapshot]);

  const filtered = useMemo(
    () => filterDownloads(downloads, navSection, search, scheduledIds),
    [downloads, navSection, search, scheduledIds],
  );

  const sorted = useMemo(() => sortDownloads(filtered, sort), [filtered, sort]);

  const selectedDownload = useMemo(
    () => (selectedId ? downloads.find((d) => d.id === selectedId) ?? null : null),
    [downloads, selectedId],
  );

  // Keep selection consistent when downloads disappear.
  useEffect(() => {
    if (selectedId && !downloads.find((d) => d.id === selectedId)) setSelectedId(null);
    setCheckedIds((prev) => {
      const next = new Set<string>();
      for (const id of prev) {
        if (downloads.find((d) => d.id === id)) next.add(id);
      }
      return next.size === prev.size ? prev : next;
    });
  }, [downloads, selectedId]);

  // Bulk-bar derived state.
  const checkedDownloads = useMemo(
    () => downloads.filter((d) => checkedIds.has(d.id)),
    [downloads, checkedIds],
  );
  const canPauseAny = checkedDownloads.some((d) => canPause(d.status));
  const canCancelAny = checkedDownloads.some((d) => canCancel(d.status));

  // -------------------------------------------------------------------------
  // Single-download actions (with confirmation for destructive ones)
  // -------------------------------------------------------------------------

  const handleCancel = useCallback((id: string) => {
    const d = downloads.find((dl) => dl.id === id);
    setConfirm({ kind: "cancel-one", ids: [id], filename: d?.filename ?? id });
  }, [downloads]);

  const handleDelete = useCallback((id: string) => {
    const d = downloads.find((dl) => dl.id === id);
    setConfirm({ kind: "delete-one", ids: [id], filename: d?.filename ?? id });
  }, [downloads]);

  const handleRetry = useCallback((id: string) => void retry(id), [retry]);

  const handlePriorityChange = useCallback(
    (id: string, priority: DownloadPriority) => void setPriority(id, priority),
    [setPriority],
  );
  const handleQueueMove = useCallback((id: string, direction: "up" | "down") => {
    setSort("queue");
    if (direction === "up") void moveUpInQueue(id);
    else void moveDownInQueue(id);
  }, [moveDownInQueue, moveUpInQueue]);
  const handleSetSchedule = useCallback((id: string, ms: number) => setSchedule(id, ms), [setSchedule]);
  const handleClearSchedule = useCallback((id: string) => clearSchedule(id), [clearSchedule]);
  const handleError = useCallback((message: string) => setGlobalError(message), []);
  const handleStopSeeding = useCallback(async (id: string) => {
    try {
      await stopTorrentSeeding(id);
      await load();
      refreshSnapshot();
    } catch (cause) {
      setGlobalError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [load, refreshSnapshot]);

  const handleUpdateUrl = useCallback((id: string, newUrl: string) => {
    updateDownloadUrl(id, newUrl).catch((cause: unknown) => {
      setGlobalError(cause instanceof Error ? cause.message : String(cause));
    });
  }, []);

  const handleContextMenu = useCallback((e: React.MouseEvent, download: Download) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY, download });
  }, []);

  const handleContextMenuCopyUrl = useCallback((download: Download) => {
    void navigator.clipboard.writeText(download.url).catch(() => { /* non-critical */ });
  }, []);

  const handleContextMenuOpenFolder = useCallback((id: string) => {
    void openDownloadFolder(id).catch((cause: unknown) => {
      setGlobalError(cause instanceof Error ? cause.message : String(cause));
    });
  }, []);

  // -------------------------------------------------------------------------
  // Bulk actions
  // -------------------------------------------------------------------------

  const handleBulkPause = useCallback(() => {
    const ids = checkedDownloads.filter((d) => canPause(d.status)).map((d) => d.id);
    if (!ids.length) return;
    void (async () => {
      const failures = await bulkOperation(ids, (id) => pause(id) as Promise<void>);
      setCheckedIds(new Set());
      if (failures.length) setGlobalError(`${failures.length} pause(s) failed.`);
    })();
  }, [checkedDownloads, bulkOperation, pause]);

  const handleBulkCancel = useCallback(() => {
    const ids = checkedDownloads.filter((d) => canCancel(d.status)).map((d) => d.id);
    if (!ids.length) return;
    setConfirm({ kind: "cancel-bulk", ids });
  }, [checkedDownloads]);

  const handleBulkDelete = useCallback(() => {
    const ids = [...checkedIds];
    if (!ids.length) return;
    setConfirm({ kind: "delete-bulk", ids });
  }, [checkedIds]);

  // -------------------------------------------------------------------------
  // Confirmation execution
  // -------------------------------------------------------------------------

  const executeConfirm = useCallback(async () => {
    if (!confirm) return;
    const { ids } = confirm;
    setConfirm(null);

    if (confirm.kind === "cancel-one" || confirm.kind === "cancel-bulk") {
      const failures = await bulkOperation(ids, (id) => cancel(id) as Promise<void>);
      if (confirm.kind === "cancel-bulk") {
        setCheckedIds(new Set());
        if (failures.length) setGlobalError(`${failures.length} cancel(s) failed.`);
      } else if (failures.length) {
        setGlobalError(failures[0]);
      }
    } else {
      const failures = await bulkOperation(ids, (id) => remove(id) as Promise<void>);
      if (confirm.kind === "delete-bulk") {
        setCheckedIds(new Set());
        if (failures.length) setGlobalError(`${failures.length} delete(s) failed.`);
      } else {
        if (selectedId === ids[0]) setSelectedId(null);
        if (failures.length) setGlobalError(failures[0]);
      }
    }
  }, [confirm, bulkOperation, cancel, remove, selectedId]);

  // -------------------------------------------------------------------------
  // Add dialog
  // -------------------------------------------------------------------------

  const openAddDialog = useCallback((url = "") => {
    setAddDialogUrl(url);
    setShowAddDialog(true);
  }, []);

  const handleAdded = useCallback(() => {
    setShowAddDialog(false);
    setAddDialogUrl("");
    void load();
    refreshSnapshot();
  }, [load, refreshSnapshot]);

  // -------------------------------------------------------------------------
  // Drag-and-drop URL adding
  // -------------------------------------------------------------------------

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (e.dataTransfer.types.includes("text/uri-list") || e.dataTransfer.types.includes("text/plain")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    // Only clear if leaving the pane itself (not a child).
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
      setIsDragging(false);
    }
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);

    // Dropped files: open the add dialog for .metalink / .meta4 manifests.
    // OS file drops are delivered via Tauri's onDragDropEvent below (the webview
    // does not populate dataTransfer.files for them); this covers any
    // webview-provided file drops.
    const droppedFiles = e.dataTransfer.files;
    if (droppedFiles && droppedFiles.length > 0) {
      for (let i = 0; i < droppedFiles.length; i += 1) {
        const file = droppedFiles.item(i);
        if (file && /\.(metalink|meta4)$/i.test(file.name)) {
          const path = (file as File & { path?: string }).path ?? file.name;
          openAddDialog(path);
          return;
        }
      }
    }

    // Extract the first valid HTTP/HTTPS URL from the drop data.
    const raw =
      e.dataTransfer.getData("text/uri-list") ||
      e.dataTransfer.getData("text/plain");

    if (!raw) return;

    // text/uri-list may contain multiple lines; pick the first non-comment.
    const firstUrl = raw
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#"))
      .find((line) => line.startsWith("http://") || line.startsWith("https://"));

    if (firstUrl) {
      openAddDialog(firstUrl);
    }
  }, [openAddDialog]);

  // -------------------------------------------------------------------------
  // OS file drag-and-drop (Tauri)
  // -------------------------------------------------------------------------
  // Tauri v2 intercepts OS file drops instead of forwarding them to the
  // webview's HTML5 drag events, so dataTransfer.files is empty for them. We
  // listen to Tauri's native drag-drop event, which provides absolute paths.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((dragEvent) => {
        const payload = dragEvent.payload;
        if (payload.type === "enter") {
          if (payload.paths.some((path) => /\.(metalink|meta4)$/i.test(path))) {
            setIsDragging(true);
          }
        } else if (payload.type === "drop") {
          setIsDragging(false);
          const metalink = payload.paths.find((path) =>
            /\.(metalink|meta4)$/i.test(path),
          );
          if (metalink) {
            openAddDialog(metalink);
          }
        } else if (payload.type === "leave") {
          setIsDragging(false);
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      unlisten?.();
    };
  }, [openAddDialog]);

  // -------------------------------------------------------------------------
  // Keyboard shortcuts
  // -------------------------------------------------------------------------

  useEffect(() => {
    const isEditable = (el: EventTarget | null) => {
      if (!(el instanceof HTMLElement)) return false;
      const tag = el.tagName;
      return (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        el.isContentEditable
      );
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      // Don't fire shortcuts while the user is typing.
      if (isEditable(e.target)) return;

      switch (e.key) {
        case "n":
        case "N":
        case "/":
          e.preventDefault();
          searchRef.current?.focus();
          break;

        case "a":
        case "A":
          if (!showAddDialog && !confirm) {
            e.preventDefault();
            openAddDialog();
          }
          break;

        case "Escape":
          if (confirm) {
            setConfirm(null);
          } else if (showAddDialog) {
            setShowAddDialog(false);
          } else if (selectedId) {
            setSelectedId(null);
          } else if (checkedIds.size > 0) {
            setCheckedIds(new Set());
          }
          break;

        case "ArrowDown":
          if (sorted.length > 0) {
            e.preventDefault();
            const idx = selectedId ? sorted.findIndex((d) => d.id === selectedId) : -1;
            const next = sorted[Math.min(idx + 1, sorted.length - 1)];
            if (next) setSelectedId(next.id);
          }
          break;

        case "ArrowUp":
          if (sorted.length > 0) {
            e.preventDefault();
            const idx = selectedId ? sorted.findIndex((d) => d.id === selectedId) : sorted.length;
            const prev = sorted[Math.max(idx - 1, 0)];
            if (prev) setSelectedId(prev.id);
          }
          break;

        case "Enter":
          if (selectedId && !showAddDialog && !confirm) {
            // Detail panel is already open when selectedId is set; toggle it.
            e.preventDefault();
          }
          break;

        case "Delete":
        case "Backspace":
          if (selectedId && !showAddDialog && !confirm) {
            e.preventDefault();
            handleDelete(selectedId);
          }
          break;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    showAddDialog, confirm, selectedId, checkedIds, sorted,
    openAddDialog, handleDelete,
  ]);

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------

  const activeError = globalError ?? error;

  return (
    <div className="app-layout">
      <OnboardingModal openAddDialog={openAddDialog} />
      <BrowserExtensionSetupModal />
      <Sidebar
        active={navSection}
        counts={counts}
        snapshot={snapshot}
        onSelect={(section) => {
          setNavSection(section);
          setSelectedId(null);
          setCheckedIds(new Set());
        }}
      />

      <div className="app-main">
        <Toolbar
          title={SECTION_TITLES[navSection]}
          count={sorted.length}
          search={search}
          sort={sort}
          searchRef={searchRef}
          onSearchChange={setSearch}
          onSortChange={setSort}
          onRefresh={() => { void load(); refreshSnapshot(); }}
          dispatchManuallyPaused={snapshot?.manual_dispatch_paused ?? false}
          dispatchSchedulePaused={snapshot?.schedule_dispatch_paused ?? false}
          onToggleQueueDispatch={() => { void toggleQueueDispatch(); }}
          onAddClick={() => openAddDialog()}
          onBatchClick={() => setShowBatchDialog(true)}
          activeCount={activeCount}
          pausedCount={pausedCount}
          onPauseAll={handlePauseAll}
          onResumeAll={handleResumeAll}
          appVersion={appVersion}
          theme={theme}
          onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        />

        {updateBanner && (
          <div className="update-banner" role="alert">
            <span>FetchX {updateBanner.latest_version} is available</span>
            <button
              className="button button-primary"
              onClick={() => { window.open(updateBanner.release_url, '_blank'); }}
            >
              Download
            </button>
            <button
              className="icon-button"
              aria-label="Dismiss update notification"
              onClick={() => setUpdateBanner(null)}
            >
              ×
            </button>
          </div>
        )}

        {activeError && (
          <div className="error-banner" role="alert">
            <span>{activeError}</span>
            <button
              className="icon-button"
              aria-label="Dismiss error"
              onClick={() => { setGlobalError(null); dismissError(); }}
            >
              ×
            </button>
          </div>
        )}

        {checkedIds.size > 0 && navSection !== "settings" && (
          <BulkActionBar
            count={checkedIds.size}
            canPauseAny={canPauseAny}
            canCancelAny={canCancelAny}
            onPauseAll={handleBulkPause}
            onCancelAll={handleBulkCancel}
            onDeleteAll={handleBulkDelete}
            onClearSelection={() => setCheckedIds(new Set())}
          />
        )}

        {navSection === "settings" ? (
          <div className="content-area">
            <div className="download-pane">
              <SettingsPage />
            </div>
          </div>
        ) : navSection === "history" ? (
          <div className="content-area">
            <div className="download-pane">
              <HistoryScreen />
            </div>
          </div>
        ) : (
        <div className={`content-area${selectedDownload ? " content-area--split" : ""}`}>
          <div
            className={`download-pane${isDragging ? " download-pane--drag-over" : ""}`}
            onDragOver={handleDragOver}
            onDragLeave={handleDragLeave}
            onDrop={handleDrop}
          >
            {isDragging && (
              <div className="drag-overlay" aria-hidden="true">
                Drop URL to add download
              </div>
            )}

            {clipboardBanners.map((banner) => (
              <div key={banner.id} className="clipboard-banner" role="alert">
                URL detected in clipboard — {banner.filename}
                <button onClick={() => { openAddDialog(banner.url); dismissBanner(banner.id); }}>Download</button>
                <button onClick={() => dismissBanner(banner.id)}>Dismiss</button>
              </div>
            ))}

            {loading ? (
              <div className="empty-state" role="status">
                <span>Loading downloads…</span>
              </div>
            ) : sorted.length === 0 ? (
              search ? (
                <EmptyState title="No results" subtitle={`Nothing matches "${search}".`} />
              ) : (
                <EmptyState
                  title={EMPTY_MESSAGES[navSection].title}
                  subtitle={EMPTY_MESSAGES[navSection].subtitle}
                />
              )
            ) : (
              <div className="download-list">
                {sorted.map((download) => (
                  <DownloadRow
                    key={download.id}
                    download={download}
                    schedule={schedules[download.id]}
                    busy={busyId === download.id}
                    selected={selectedId === download.id}
                    checked={checkedIds.has(download.id)}
                    onSelect={() =>
                      setSelectedId((prev) => (prev === download.id ? null : download.id))
                    }
                    onCheckToggle={(e) => {
                      e.stopPropagation();
                      setCheckedIds((prev) => {
                        const next = new Set(prev);
                        if (next.has(download.id)) next.delete(download.id);
                        else next.add(download.id);
                        return next;
                      });
                    }}
                    onPause={pause}
                    onResume={resume}
                    onCancel={handleCancel}
                    onDelete={handleDelete}
                    onRetry={handleRetry}
                    onPriorityChange={handlePriorityChange}
                    onQueueMove={handleQueueMove}
                    onSetSchedule={handleSetSchedule}
                    onClearSchedule={handleClearSchedule}
                    onError={handleError}
                    onEnsureSchedule={ensureScheduleFetched}
                    onUpdateUrl={handleUpdateUrl}
                    onSetBandwidthLimit={(id) => {
                      const d = downloads.find((x) => x.id === id);
                      setBandwidthLimitTarget({ id, filename: d?.filename ?? id, currentLimit: d?.bandwidth_limit ?? null });
                    }}
                    onContextMenu={handleContextMenu}
                    speedHistory={speedHistorySnapshot.get(download.id)}
                  />
                ))}
              </div>
            )}
          </div>

          {selectedDownload && (
            <DetailPanel
              download={selectedDownload}
              downloads={downloads}
              schedule={schedules[selectedDownload.id] ?? null}
              busy={busyId === selectedDownload.id}
              onClose={() => setSelectedId(null)}
              onPause={pause}
              onResume={resume}
              onCancel={handleCancel}
              onDelete={handleDelete}
              onRetry={handleRetry}
              onPriorityChange={handlePriorityChange}
              onSetSchedule={handleSetSchedule}
              onClearSchedule={handleClearSchedule}
              onError={handleError}
              onStopSeeding={handleStopSeeding}
              onUpdateUrl={handleUpdateUrl}
            />
          )}
        </div>
        )} {/* end settings ternary */}

        <SpeedChart />

        <footer className="app-footer">
          Downloads are stored in your system Downloads folder. Removing an item preserves its files.
          {checkedIds.size === 0 && (
            <span className="footer-hint">
              · <kbd>A</kbd> to add · <kbd>/</kbd> to search · <kbd>↑</kbd><kbd>↓</kbd> to navigate
            </span>
          )}
        </footer>
      </div>

      {showAddDialog && (
        <AddDialog
          initialUrl={addDialogUrl}
          onAdded={handleAdded}
          onClose={() => { setShowAddDialog(false); setAddDialogUrl(""); }}
        />
      )}

      {showBatchDialog && (
        <BatchAddDialog
          onComplete={() => { setShowBatchDialog(false); void load(); refreshSnapshot(); }}
          onClose={() => setShowBatchDialog(false)}
          onToast={(message, kind) => addToast(message, kind)}
        />
      )}

      {bandwidthLimitTarget && (
        <BandwidthLimitDialog
          downloadId={bandwidthLimitTarget.id}
          filename={bandwidthLimitTarget.filename}
          currentLimit={bandwidthLimitTarget.currentLimit}
          onSaved={() => { setBandwidthLimitTarget(null); void load(); }}
          onClose={() => setBandwidthLimitTarget(null)}
        />
      )}

      {confirm && (
        <ConfirmDialog
          title={confirmTitle(confirm)}
          message={confirmMessage(confirm)}
          confirmLabel={confirm.kind.startsWith("cancel") ? "Cancel download" : "Delete"}
          cancelLabel="Go back"
          danger={true}
          onConfirm={() => void executeConfirm()}
          onCancel={() => setConfirm(null)}
        />
      )}

      {conversionDownload && (
        <ConversionDialog
          download={conversionDownload}
          onStarted={() => { setConversionDownload(null); addToast("Conversion started.", "success"); }}
          onClose={() => setConversionDownload(null)}
        />
      )}

      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          download={contextMenu.download}
          onClose={() => setContextMenu(null)}
          onPause={pause}
          onResume={resume}
          onCancel={handleCancel}
          onDelete={handleDelete}
          onRetry={handleRetry}
          onCopyUrl={handleContextMenuCopyUrl}
          onOpenFolder={handleContextMenuOpenFolder}
        />
      )}

      <ToastStack toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
