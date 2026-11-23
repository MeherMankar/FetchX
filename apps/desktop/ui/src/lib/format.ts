/** Human-readable formatting utilities. No side effects, no IPC. */

export function formatBytes(value: number | null): string {
  if (value === null) return "—";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value;
  let unit = -1;
  do {
    size /= 1024;
    unit += 1;
  } while (size >= 1024 && unit < units.length - 1);
  return `${size.toFixed(1)} ${units[unit]}`;
}

export function formatSpeed(bytesPerSecond: number | null): string {
  if (bytesPerSecond === null) return "—";
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatEta(seconds: number | null): string {
  if (seconds === null) return "—";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

export function formatPercent(basisPoints: number | null): string {
  if (basisPoints === null) return "—";
  return `${(basisPoints / 100).toFixed(1)}%`;
}

/** "Oct 10, 2026, 14:30" */
export function formatDatetime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "YYYY-MM-DDTHH:MM" for <input type="datetime-local"> */
export function msToDatetimeLocal(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

export function errorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as Record<string, unknown>).message === "string"
  ) {
    return (error as { message: string }).message;
  }
  if (typeof error === "string") return error;
  return "The request could not be completed.";
}

/** Maps known technical error strings to plain-language equivalents.
 *  Truncates to 120 characters with an ellipsis when needed.
 *  Returns the raw string unchanged when no mapping applies. */
export function friendlyErrorMessage(raw: string): string {
  type Replacement = string | ((m: string) => string);
  const MAPPINGS: Array<[string | RegExp, Replacement]> = [
    [
      "The HTTP request failed.",
      "The connection to the download server failed or was interrupted. Try again.",
    ],
    [
      /The HTTP server returned status 4\d\d/,
      (m: string) => {
        const match = m.match(/(\d{3})/);
        const code = match ? match[1] : "";
        return `The server rejected the request (error ${code}). The URL may be invalid or require login.`;
      },
    ],
    [
      /The HTTP server returned status 5\d\d/,
      (m: string) => {
        const match = m.match(/(\d{3})/);
        const code = match ? match[1] : "";
        return `The server returned an error (${code}). Try again in a few minutes.`;
      },
    ],
    [
      "The built-in media processor failed while merging the video and audio streams.",
      "The video and audio tracks could not be merged. Try retrying — if it keeps failing, the format may not be supported.",
    ],
    [
      "The built-in media processor could not be initialized.",
      "Media processing is unavailable. Restart FetchX and try again.",
    ],
    [
      "yt-dlp left separate output files instead of a merged media file",
      "The download finished but the video and audio could not be combined. Try a different quality or format.",
    ],
    [
      "The SFTP download could not be completed.",
      "Could not connect via SFTP. Check the address, username, and that the server is reachable.",
    ],
    [
      "The downloaded file failed SHA-256 verification and was not published.",
      "The downloaded file appears to be corrupted — the checksum did not match. Try downloading again.",
    ],
  ];

  for (const [pattern, replacement] of MAPPINGS) {
    const matches =
      typeof pattern === "string" ? raw.includes(pattern) : pattern.test(raw);
    if (matches) {
      const result = typeof replacement === "function" ? replacement(raw) : replacement;
      return result.length > 120 ? result.slice(0, 117) + "…" : result;
    }
  }

  return raw.length > 120 ? raw.slice(0, 117) + "…" : raw;
}

/**
 * Returns a relative time string (e.g. "Just now", "3 minutes ago", "2 hours ago").
 * Falls back to an absolute datetime string for events older than 24 hours.
 */
export function formatRelativeTime(ms: number): string {
  const diffMs = Date.now() - ms;
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 10) return "Just now";
  if (diffSec < 60) return `${diffSec} seconds ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return diffMin === 1 ? "1 minute ago" : `${diffMin} minutes ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return diffHr === 1 ? "1 hour ago" : `${diffHr} hours ago`;
  const diffDay = Math.floor(diffHr / 24);
  if (diffDay === 1) return "Yesterday";
  // Older than 2 days — show absolute date.
  return formatDatetime(ms);
}
