import { useCallback, useEffect, useMemo, useState } from "react";

import { getAppInfo } from "./api/app";
import type { Download, DownloadPriority } from "./api/types";
import { AddDialog } from "./components/AddDialog";
import { DetailPanel } from "./components/DetailPanel";
import { DownloadRow } from "./components/DownloadRow";
import { EmptyState } from "./components/EmptyState";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { useDownloads } from "./hooks/useDownloads";
import { countStatuses } from "./lib/status";

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
  | "cancelled";

export type SortKey =
  | "newest"
  | "oldest"
  | "filename"
  | "progress"
  | "size"
  | "speed"
  | "priority";

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
    // Section filter
    switch (section) {
      case "all":
        break;
      case "active":
        if (d.status !== "downloading" && d.status !== "preparing") return false;
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

    // Search filter — match filename or URL, case-insensitive
    if (q) {
      const haystack = `${d.filename} ${d.url}`.toLowerCase();
      if (!haystack.includes(q)) return false;
    }

    return true;
  });
}

function sortDownloads(downloads: Download[], sort: SortKey): Download[] {
  const sorted = [...downloads];
  switch (sort) {
    case "newest":
      sorted.sort((a, b) => b.created_at - a.created_at);
      break;
    case "oldest":
      sorted.sort((a, b) => a.created_at - b.created_at);
      break;
    case "filename":
      sorted.sort((a, b) => a.filename.localeCompare(b.filename));
      break;
    case "progress":
      sorted.sort(
        (a, b) =>
          (b.progress.percentage_basis_points ?? 0) -
          (a.progress.percentage_basis_points ?? 0),
      );
      break;
    case "size":
      sorted.sort(
        (a, b) => (b.progress.total_bytes ?? 0) - (a.progress.total_bytes ?? 0),
      );
      break;
    case "speed":
      sorted.sort(
        (a, b) =>
          (b.progress.speed_bytes_per_second ?? 0) -
          (a.progress.speed_bytes_per_second ?? 0),
      );
      break;
    case "priority":
      sorted.sort(
        (a, b) =>
          (PRIORITY_ORDER[a.priority] ?? 1) - (PRIORITY_ORDER[b.priority] ?? 1),
      );
      break;
  }
  return sorted;
}

const SECTION_TITLES: Record<NavSection, string> = {
  all: "All Downloads",
  active: "Active",
  queued: "Queued",
  paused: "Paused",
  scheduled: "Scheduled",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
};

const EMPTY_MESSAGES: Record<NavSection, { title: string; subtitle: string }> = {
  all: { title: "No downloads yet", subtitle: "Click + Add to get started." },
  active: { title: "Nothing downloading", subtitle: "Start a queued download to see it here." },
  queued: { title: "Queue is empty", subtitle: "Add a download to fill the queue." },
  paused: { title: "Nothing paused", subtitle: "Pause an active download to see it here." },
  scheduled: { title: "No scheduled downloads", subtitle: "Use the schedule button on a queued download." },
  completed: { title: "No completed downloads", subtitle: "Finished downloads will appear here." },
  failed: { title: "No failed downloads", subtitle: "Downloads that encounter errors will appear here." },
  cancelled: { title: "No cancelled downloads", subtitle: "" },
};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export function App() {
  const [appVersion, setAppVersion] = useState<string | undefined>(undefined);
  const [navSection, setNavSection] = useState<NavSection>("all");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<SortKey>("newest");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [globalError, setGlobalError] = useState<string | null>(null);

  const {
    downloads,
    schedules,
    loading,
    error,
    busyId,
    load,
    ensureScheduleFetched,
    pause,
    resume,
    cancel,
    remove,
    setPriority,
    setSchedule,
    clearSchedule,
    dismissError,
  } = useDownloads();

  // Load app version once.
  useEffect(() => {
    void getAppInfo()
      .then((info) => setAppVersion(info.version))
      .catch(() => {
        /* version is cosmetic; silently ignore */
      });
  }, []);

  // -------------------------------------------------------------------------
  // Derived state
  // -------------------------------------------------------------------------

  // Set of download IDs that have an active (enabled) schedule.
  const scheduledIds = useMemo(() => {
    const ids = new Set<string>();
    for (const [id, sched] of Object.entries(schedules)) {
      if (sched?.enabled) ids.add(id);
    }
    return ids;
  }, [schedules]);

  const baseCounts = useMemo(() => countStatuses(downloads), [downloads]);
  const counts = useMemo(
    () => ({ ...baseCounts, scheduled: scheduledIds.size }),
    [baseCounts, scheduledIds],
  );

  const filtered = useMemo(
    () => filterDownloads(downloads, navSection, search, scheduledIds),
    [downloads, navSection, search, scheduledIds],
  );

  const sorted = useMemo(() => sortDownloads(filtered, sort), [filtered, sort]);

  const selectedDownload = useMemo(
    () => (selectedId ? downloads.find((d) => d.id === selectedId) ?? null : null),
    [downloads, selectedId],
  );

  // If the selected download is removed, close the detail panel.
  useEffect(() => {
    if (selectedId && !downloads.find((d) => d.id === selectedId)) {
      setSelectedId(null);
    }
  }, [downloads, selectedId]);

  // -------------------------------------------------------------------------
  // Action wrappers that also close detail panel if item is deleted
  // -------------------------------------------------------------------------

  const handleDelete = useCallback(
    async (id: string) => {
      await remove(id);
      if (selectedId === id) setSelectedId(null);
    },
    [remove, selectedId],
  );

  const handlePriorityChange = useCallback(
    (id: string, priority: DownloadPriority) => {
      void setPriority(id, priority);
    },
    [setPriority],
  );

  const handleSetSchedule = useCallback(
    (id: string, ms: number) => setSchedule(id, ms),
    [setSchedule],
  );

  const handleClearSchedule = useCallback(
    (id: string) => clearSchedule(id),
    [clearSchedule],
  );

  const handleError = useCallback(
    (message: string) => setGlobalError(message),
    [],
  );

  // -------------------------------------------------------------------------
  // Add dialog
  // -------------------------------------------------------------------------

  const handleAdded = useCallback(() => {
    setShowAddDialog(false);
    void load();
  }, [load]);

  // -------------------------------------------------------------------------
  // Render helpers
  // -------------------------------------------------------------------------

  const activeError = globalError ?? error;

  return (
    <div className="app-layout">
      <Sidebar
        active={navSection}
        counts={counts}
        onSelect={(section) => {
          setNavSection(section);
          setSelectedId(null);
        }}
      />

      <div className="app-main">
        <Toolbar
          title={SECTION_TITLES[navSection]}
          count={sorted.length}
          search={search}
          sort={sort}
          onSearchChange={setSearch}
          onSortChange={setSort}
          onRefresh={() => void load()}
          onAddClick={() => setShowAddDialog(true)}
          appVersion={appVersion}
        />

        {activeError && (
          <div className="error-banner" role="alert">
            <span>{activeError}</span>
            <button
              className="icon-button"
              aria-label="Dismiss error"
              onClick={() => {
                setGlobalError(null);
                dismissError();
              }}
            >
              ×
            </button>
          </div>
        )}

        <div className={`content-area${selectedDownload ? " content-area--split" : ""}`}>
          <div className="download-pane">
            {loading ? (
              <div className="empty-state" role="status">
                <span>Loading downloads…</span>
              </div>
            ) : sorted.length === 0 ? (
              search ? (
                <EmptyState icon="🔍" title="No results" subtitle={`Nothing matches "${search}".`} />
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
                    onSelect={() =>
                      setSelectedId((prev) => (prev === download.id ? null : download.id))
                    }
                    onPause={pause}
                    onResume={resume}
                    onCancel={cancel}
                    onDelete={(id) => void handleDelete(id)}
                    onPriorityChange={handlePriorityChange}
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
              onCancel={cancel}
              onDelete={(id) => void handleDelete(id)}
              onPriorityChange={handlePriorityChange}
              onSetSchedule={handleSetSchedule}
              onClearSchedule={handleClearSchedule}
              onError={handleError}
            />
          )}
        </div>

        <footer className="app-footer">
          Downloads are stored in your system Downloads folder. Removing an item preserves its files.
        </footer>
      </div>

      {showAddDialog && (
        <AddDialog
          onAdded={handleAdded}
          onClose={() => setShowAddDialog(false)}
        />
      )}
    </div>
  );
}
