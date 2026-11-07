import { useCallback, useEffect, useRef, useState } from "react";

import {
  cancelDownload,
  deleteDownload,
  getDownloadSchedule,
  listenForDownloadChanges,
  listDownloads,
  moveDownloadDownInQueue,
  moveDownloadUpInQueue,
  pauseDownload,
  removeDownloadSchedule,
  resumeDownload,
  retryDownload,
  scheduleDownload,
  setDownloadPriority,
} from "../api/downloads";
import type { Download, DownloadPriority, DownloadSchedule } from "../api/types";
import { errorMessage } from "../lib/format";
import { canSchedule } from "../lib/status";

/**
 * Central download state hook.  All IPC and event subscription lives here;
 * components stay purely presentational.
 */
export function useDownloads() {
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [schedules, setSchedules] = useState<Record<string, DownloadSchedule | null>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const fetchedSchedules = useRef<Set<string>>(new Set());

  // -------------------------------------------------------------------------
  // Initial load + event subscription
  // -------------------------------------------------------------------------

  const load = useCallback(async () => {
    try {
      const items = await listDownloads();
      setDownloads(items);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const initialize = async () => {
      try {
        const stop = await listenForDownloadChanges(
          (download) => {
            setDownloads((current) => {
              const existingIndex = current.findIndex((item) => item.id === download.id);
              if (existingIndex < 0) return [...current, download];
              return current.map((item, index) =>
                index === existingIndex ? download : item,
              );
            });
          },
          (downloadId) => {
            setDownloads((current) => current.filter((d) => d.id !== downloadId));
            setSchedules((current) => {
              const next = { ...current };
              delete next[downloadId];
              return next;
            });
            fetchedSchedules.current.delete(downloadId);
          },
        );
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      } catch (cause) {
        if (!disposed) setError(errorMessage(cause));
      }
      if (!disposed) await load();
    };

    void initialize();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [load]);

  // -------------------------------------------------------------------------
  // Lazy schedule fetch — called from render, not inside effects
  // -------------------------------------------------------------------------

  const ensureScheduleFetched = useCallback(
    (download: Download) => {
      if (!canSchedule(download.status)) return;
      if (fetchedSchedules.current.has(download.id)) return;
      fetchedSchedules.current.add(download.id);
      void getDownloadSchedule(download.id).then((schedule) => {
        setSchedules((current) => ({ ...current, [download.id]: schedule }));
      });
    },
    [],
  );

  // -------------------------------------------------------------------------
  // Actions
  // -------------------------------------------------------------------------

  const withBusy = useCallback(
    async (id: string, action: () => Promise<unknown>) => {
      setBusyId(id);
      setError(null);
      try {
        await action();
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  const pause = useCallback(
    (id: string) => withBusy(id, () => pauseDownload(id)),
    [withBusy],
  );

  const resume = useCallback(
    (id: string) => withBusy(id, () => resumeDownload(id)),
    [withBusy],
  );

  const cancel = useCallback(
    (id: string) => withBusy(id, () => cancelDownload(id)),
    [withBusy],
  );

  const remove = useCallback(
    (id: string) => withBusy(id, () => deleteDownload(id)),
    [withBusy],
  );

  const retry = useCallback(
    (id: string) => withBusy(id, () => retryDownload(id)),
    [withBusy],
  );

  /**
   * Runs an action on each id sequentially, collecting per-item errors.
   * Never stops early on failure.  Returns list of failure messages.
   */
  const bulkOperation = useCallback(
    async (ids: string[], action: (id: string) => Promise<void>): Promise<string[]> => {
      const failures: string[] = [];
      for (const id of ids) {
        try {
          await action(id);
        } catch (cause) {
          failures.push(errorMessage(cause));
        }
      }
      return failures;
    },
    [],
  );

  const setPriority = useCallback(
    async (id: string, priority: DownloadPriority) => {
      setBusyId(id);
      setError(null);
      try {
        const updated = await setDownloadPriority(id, priority);
        setDownloads((current) =>
          current.map((d) => (d.id === updated.id ? updated : d)),
        );
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  const moveUpInQueue = useCallback(
    async (id: string) => {
      await withBusy(id, async () => {
        await moveDownloadUpInQueue(id);
        await load();
      });
    },
    [load, withBusy],
  );

  const moveDownInQueue = useCallback(
    async (id: string) => {
      await withBusy(id, async () => {
        await moveDownloadDownInQueue(id);
        await load();
      });
    },
    [load, withBusy],
  );

  const setSchedule = useCallback(
    async (id: string, scheduledAtMs: number) => {
      setBusyId(id);
      setError(null);
      try {
        const schedule = await scheduleDownload(id, scheduledAtMs);
        setSchedules((current) => ({ ...current, [id]: schedule }));
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  const clearSchedule = useCallback(
    async (id: string) => {
      setBusyId(id);
      setError(null);
      try {
        await removeDownloadSchedule(id);
        setSchedules((current) => ({ ...current, [id]: null }));
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  const dismissError = useCallback(() => setError(null), []);

  return {
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
    retry,
    bulkOperation,
    setPriority,
    moveUpInQueue,
    moveDownInQueue,
    setSchedule,
    clearSchedule,
    dismissError,
  };
}
