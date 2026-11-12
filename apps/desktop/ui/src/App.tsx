import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { getAppInfo } from "./api/app";
import {
  getQueueSnapshot,
  pauseQueueDispatch,
  resumeQueueDispatch,
  stopTorrentSeeding,
} from "./api/downloads";
import type { Download, DownloadPriority, QueueSnapshot } from "./api/types";
import { AddDialog } from "./components/AddDialog";
import { BulkActionBar } from "./components/BulkActionBar";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { DetailPanel } from "./components/DetailPanel";
import { DownloadRow } from "./components/DownloadRow";
import { EmptyState } from "./components/EmptyState";
import { Sidebar } from "./components/Sidebar";
import { SettingsPage } from "./components/SettingsPage";
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
  | "settings";

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
  settings: "Settings",
};

const EMPTY_MESSAGES: Record<NavSection, { title: string; subtitle: string }> = {
  all:       { title: "No downloads yet",          subtitle: "Click + Add or drag a URL here." },
  active:    { title: "Nothing downloading",        subtitle: "Start a queued download to see it here." },
  queued:    { title: "Queue is empty",             subtitle: "Add a download to fill the queue." },
  paused:    { title: "Nothing paused",             subtitle: "Pause an active download to see it here." },
  scheduled: { title: "No scheduled downloads",    subtitle: "Use the schedule button on a queued download." },
  completed: { title: "No completed downloads",    subtitle: "Finished downloads will appear here." },
  failed:    { title: "No failed downloads",       subtitle: "Downloads that encounter errors will appear here." },
  cancelled: { title: "No cancelled downloads",    subtitle: "" },
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
  const searchRef = useRef<HTMLInputElement>(null);
  const prevCompletedRef = useRef<Set<string>>(new Set());
  const { toasts, addToast, dismiss: dismissToast } = useToasts();

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
          appVersion={appVersion}
        />

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
                  />
                ))}
              </div>
            )}
          </div>

          {selectedDownload && (
            <DetailPanel
              download={selectedDownload}
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
            />
          )}
        </div>
        )} {/* end settings ternary */}

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

      <ToastStack toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
