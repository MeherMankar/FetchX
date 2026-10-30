export type DownloadSubmission =
  | { kind: "accepted"; downloadId: string }
  | { kind: "rejected"; message: string }
  | { kind: "unknown"; message: string };

export type HandoffResult =
  | { kind: "not_paused" }
  | { kind: "accepted"; downloadId: string; browserDownloadCancelled: boolean }
  | { kind: "rejected"; message: string; browserDownloadResumed: boolean }
  | { kind: "unknown"; message: string };

export interface DownloadHandoffPort {
  pause(downloadId: number): Promise<void>;
  resume(downloadId: number): Promise<void>;
  cancel(downloadId: number): Promise<void>;
  submit(): Promise<DownloadSubmission>;
}

export interface DownloadEventClaimStore {
  load(): Promise<unknown>;
  save(downloadIds: number[]): Promise<void>;
}

export async function claimDownloadEvent(
  downloadId: number,
  claimingDownloadIds: Set<number>,
  store: DownloadEventClaimStore,
  maxClaimedDownloadIds: number,
): Promise<boolean> {
  if (claimingDownloadIds.has(downloadId)) return false;
  claimingDownloadIds.add(downloadId);
  try {
    const stored = await store.load();
    const claimedIds = Array.isArray(stored)
      ? stored.filter((id): id is number => typeof id === "number" && Number.isSafeInteger(id))
      : [];
    if (claimedIds.includes(downloadId)) return false;
    claimedIds.push(downloadId);
    await store.save(claimedIds.slice(-maxClaimedDownloadIds));
    return true;
  } finally {
    claimingDownloadIds.delete(downloadId);
  }
}

export async function handoffBrowserDownload(
  browserDownloadId: number,
  port: DownloadHandoffPort,
): Promise<HandoffResult> {
  try {
    await port.pause(browserDownloadId);
  } catch {
    return { kind: "not_paused" };
  }

  let submission: DownloadSubmission;
  try {
    submission = await port.submit();
  } catch {
    submission = {
      kind: "unknown",
      message: "FetchX may have accepted the download, but confirmation was not received.",
    };
  }

  if (submission.kind === "accepted") {
    try {
      await port.cancel(browserDownloadId);
      return {
        kind: "accepted",
        downloadId: submission.downloadId,
        browserDownloadCancelled: true,
      };
    } catch {
      return {
        kind: "accepted",
        downloadId: submission.downloadId,
        browserDownloadCancelled: false,
      };
    }
  }

  if (submission.kind === "unknown") {
    return submission;
  }

  try {
    await port.resume(browserDownloadId);
    return {
      kind: "rejected",
      message: submission.message,
      browserDownloadResumed: true,
    };
  } catch {
    return {
      kind: "rejected",
      message: submission.message,
      browserDownloadResumed: false,
    };
  }
}

export function filenameFromBrowserPath(path: string | undefined): string | undefined {
  const filename = path?.split(/[\\/]/).pop()?.trim();
  return filename && filename !== "." && filename !== ".."
    ? filename.slice(0, 255)
    : undefined;
}

export function isBlockedDomain(hostname: string, blockedDomains: string[]): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  return blockedDomains.some((entry) => {
    const domain = entry.trim().toLowerCase().replace(/^\.+|\.+$/g, "");
    return domain !== "" && (host === domain || host.endsWith(`.${domain}`));
  });
}
